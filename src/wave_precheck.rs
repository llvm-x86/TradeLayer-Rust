//! Stage 8D: read-only dependency-wave precheck **scheduling experiment**.
//!
//! The caller's raw Bitcoin-style block and preindex are NOT authenticated.
//! The dependency DAG derives from caller-supplied transaction bytes. A probe
//! is an arbitrary, *untrusted* read-only observation, NOT Script validation:
//! the closure has no protocol-state capability supplied by this API, but
//! Rust cannot prohibit callers from capturing interior-mutable state.
//!
//! Independent positions in a wave may run concurrently. Every worker joins
//! before a subsequent wave is admitted. Each wave retains the lowest
//! source-index rejection, independent of completion/scheduling order. On
//! rejection/panic, NO success report or later-wave work escapes.
//!
//! This is deliberately separate from the optimized TXID indexing path.
//! No UTXO, Script, block-header, consensus, channel-signature or TradeLayer
//! state transition is authorized by any result of this module.

use crate::parallel_txindex::ParallelIndexConfig;
use crate::raw_block_ingest::{
    DependencyAuditError, UnverifiedRawBlockIndex, UnverifiedRawBlockView,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::thread;

pub const WAVE_OBSERVATION_STATUS: &str = "read_only_dependency_waves_unverified";

/// A probe may report only *lack of local rejection*, not validity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnverifiedProbeVerdict {
    NoLocalRejection,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnverifiedWaveTrustError {
    ParentUtxoSnapshotNotAuthenticated,
    BitcoinScriptsNotVerified,
    CanonicalBitcoinAndTradeLayerApplyUnavailable,
}

/// No partial-success object is issued if any wave's probe rejects or panics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedWaveProbeReport {
    wave_count: usize,
    positions_observed: usize,
    external_prevouts_unverified: usize,
}

impl UnverifiedWaveProbeReport {
    pub fn status(&self) -> &'static str {
        WAVE_OBSERVATION_STATUS
    }

    pub fn wave_count(&self) -> usize {
        self.wave_count
    }

    pub fn positions_observed(&self) -> usize {
        self.positions_observed
    }

    pub fn unverified_external_prevouts(&self) -> usize {
        self.external_prevouts_unverified
    }

    pub const fn require_authenticated_parent_utxos(
        &self,
    ) -> Result<(), UnverifiedWaveTrustError> {
        Err(UnverifiedWaveTrustError::ParentUtxoSnapshotNotAuthenticated)
    }

    pub const fn require_valid_bitcoin_scripts(
        &self,
    ) -> Result<(), UnverifiedWaveTrustError> {
        Err(UnverifiedWaveTrustError::BitcoinScriptsNotVerified)
    }

    pub const fn require_consensus_or_economic_apply(
        &self,
    ) -> Result<(), UnverifiedWaveTrustError> {
        Err(UnverifiedWaveTrustError::CanonicalBitcoinAndTradeLayerApplyUnavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaveProbeError {
    InvalidWorkerConfig,
    Dependencies(DependencyAuditError),
    MissingRawTransaction { wave: usize, index: usize },
    ProbeRejected { wave: usize, index: usize },
    ProbePanicked { wave: usize, index: usize },
    WorkerPanicked { wave: usize },
}

#[derive(Clone, Copy)]
enum ProbeFailure {
    Missing,
    Rejected,
    Panicked,
}

fn failure(wave: usize, index: usize, kind: ProbeFailure) -> WaveProbeError {
    match kind {
        ProbeFailure::Missing => WaveProbeError::MissingRawTransaction { wave, index },
        ProbeFailure::Rejected => WaveProbeError::ProbeRejected { wave, index },
        ProbeFailure::Panicked => WaveProbeError::ProbePanicked { wave, index },
    }
}

/// Run an *untrusted* probe over dependency waves derived anew from raw bytes.
///
/// Each layer is independent only with respect to syntactically identified
/// same-block prevout edges. External UTXOs are UNKNOWN. The probe gets an
/// immutable slice of the original block's transaction bytes, plus its
/// original index. No mutable ledger/UTXO handle is passed or constructed.
///
/// To preserve fail-closed ordering, a later dependency wave never runs
/// until all current-wave workers have joined and every position has reported
/// no local rejection. Failures are ordered by (wave, source position).
/// This is NOT globally lowest source index across unexecuted later waves:
/// barrier order takes precedence over source order across different waves.
///
/// One scoped worker team is used per wave, bounded by the config's <=16
/// worker limit. There is no per-transaction shared writable output buffer.
/// Window size remains validated for caller policy but is deliberately not
/// used to respawn threads every 512 independent positions.
/// No performance speedup is asserted by this research harness.
pub fn inspect_dependency_waves_with_probe<F>(
    view: &UnverifiedRawBlockView<'_>,
    preindex: &UnverifiedRawBlockIndex,
    config: ParallelIndexConfig,
    probe: F,
) -> Result<UnverifiedWaveProbeReport, WaveProbeError>
where
    F: Fn(usize, &[u8]) -> UnverifiedProbeVerdict + Sync,
{
    let config = config
        .validate()
        .map_err(|_| WaveProbeError::InvalidWorkerConfig)?;
    // Plan every time from this exact borrowed block and preindex; never
    // allow a caller to provide a forged list of supposedly safe waves.
    let plan = view
        .plan_dependency_waves(preindex)
        .map_err(WaveProbeError::Dependencies)?;
    let mut completed = 0usize;

    for (wave_number, wave) in plan.waves().iter().enumerate() {
        let worker_count = config.workers.min(wave.len());
        if worker_count == 0 {
            continue;
        }

        // Each lane visits its source positions in ascending order. Once
        // that lane encounters a failure, later positions in that lane
        // cannot precede its first failure, so they need not run.
        let lane_probe = |lane: usize| -> Option<(usize, ProbeFailure)> {
            for &index in wave.iter().skip(lane).step_by(worker_count) {
                let Some(raw) = view.raw_transaction(index) else {
                    return Some((index, ProbeFailure::Missing));
                };
                match catch_unwind(AssertUnwindSafe(|| probe(index, raw))) {
                    Ok(UnverifiedProbeVerdict::NoLocalRejection) => {}
                    Ok(UnverifiedProbeVerdict::Reject) => {
                        return Some((index, ProbeFailure::Rejected));
                    }
                    Err(_) => return Some((index, ProbeFailure::Panicked)),
                }
            }
            None
        };

        if worker_count == 1 {
            if let Some((index, kind)) = lane_probe(0) {
                return Err(failure(wave_number, index, kind));
            }
        } else {
            // Scoped workers borrow the immutable block, the wave and the
            // stateless probe. Join EVERY worker, including after failure.
            let result = thread::scope(|scope| {
                let mut handles = Vec::with_capacity(worker_count);
                for lane in 0..worker_count {
                    let job = &lane_probe;
                    handles.push(scope.spawn(move || job(lane)));
                }
                let mut earliest: Option<(usize, ProbeFailure)> = None;
                let mut panicked = false;
                for handle in handles {
                    match handle.join() {
                        Ok(Some((index, kind))) => {
                            if earliest.is_none_or(|(previous, _)| index < previous) {
                                earliest = Some((index, kind));
                            }
                        }
                        Ok(None) => {}
                        Err(_) => panicked = true,
                    }
                }
                (earliest, panicked)
            });
            // An unexpected panic beyond the per-probe unwind fence has
            // unknown side effects; refuse any partial success.
            if result.1 {
                return Err(WaveProbeError::WorkerPanicked { wave: wave_number });
            }
            if let Some((index, kind)) = result.0 {
                return Err(failure(wave_number, index, kind));
            }
        }
        completed += wave.len();
    }

    debug_assert_eq!(completed, view.transaction_count());
    Ok(UnverifiedWaveProbeReport {
        wave_count: plan.waves().len(),
        positions_observed: completed,
        external_prevouts_unverified: plan.unverified_external_prevouts(),
    })
}
