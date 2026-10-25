//! Stage 8D4: strict, read-only parent UTXO *candidate* snapshot coverage.
//!
//! This module checks internal consistency of a caller-provided UTXO table,
//! its claimed parent Bitcoin block and the exact external prevouts referenced
//! by a locally preindexed raw block. It has NO trusted UTXO-set root: Bitcoin
//! Core's chainstate is not authenticated by a caller's SHA256d fingerprint.
//! The height, parent hash, coinbase flags, scripts and values are all claims.
//!
//! An external prevout must appear exactly once. Same-block parent outputs
//! are intentionally *deferred*: they are not in the parent snapshot and
//! cannot be treated as created or spendable until an independently verified
//! predecessor transaction has been applied. No Script, signature, fee,
//! locktime, PoW or economic transition is performed.
//!
//! A wave probe can only run after the complete candidate-snapshot preflight.
//! Neither successful preflight nor a successful probe authorizes any
//! Bitcoin/TradeLayer consensus or state transition.

use crate::hash256::double_sha256;
use crate::parallel_txindex::ParallelIndexConfig;
use crate::raw_block_ingest::{
    DependencyAuditError, UnverifiedRawBlockIndex, UnverifiedRawBlockView,
};
use crate::read_only::{
    inspect_transaction, prepare_transaction_prefix, source_prevouts_from_layout, ScanError,
};
use crate::wave_precheck::{
    inspect_dependency_waves_with_probe, UnverifiedProbeVerdict, UnverifiedWaveProbeReport,
    WaveProbeError,
};
use std::collections::{BTreeMap, BTreeSet};

pub const CANDIDATE_PARENT_UTXO_STATUS: &str = "parent_utxo_witness_coverage_unverified";
pub const CANDIDATE_SNAPSHOT_DOMAIN: &[u8] = b"TRDL-PARENT-UTXO-WITNESS-v1\x00";
pub const MAX_CANDIDATE_SNAPSHOT_ENTRIES: usize = 100_000;
pub const MAX_CANDIDATE_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SNAPSHOT_SCRIPT_BYTES: usize = 10_000;
pub const BITCOIN_MAX_MONEY_SAT: u64 = 21_000_000 * 100_000_000;
pub const BITCOIN_COINBASE_MATURITY: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CandidateOutpoint {
    /// Raw 32-byte Bitcoin TXID in transaction input (wire) byte order.
    pub txid_raw: [u8; 32],
    pub vout: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateParentUtxo {
    pub outpoint: CandidateOutpoint,
    pub amount_sat: u64,
    pub script_pubkey: Vec<u8>,
    /// Claimed height of UTXO creation, not authenticated chain provenance.
    pub created_height: u32,
    /// Claimed coinbase origin. Full Bitcoin consensus must authenticate it.
    pub is_coinbase: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateParentSnapshot {
    /// Claimed previous Bitcoin block ID in raw wire byte order.
    pub parent_block_hash_raw: [u8; 32],
    pub parent_height: u32,
    /// Sorted exact external-prevout witness set. Not a Bitcoin UTXO root.
    pub entries: Vec<CandidateParentUtxo>,
    /// Caller-selected expected digest of this same candidate witness table.
    pub expected_fingerprint: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentSnapshotError {
    ParentBlockMismatch,
    ParentHeightOverflow,
    CandidateSnapshotTooLarge,
    DuplicateCandidateOutpoint { index: usize },
    UnsortedCandidateOutpoints { index: usize },
    OversizedCandidateScript { index: usize },
    InvalidCandidateValue { index: usize },
    FutureCandidateOutput { index: usize },
    CandidateFingerprintMismatch,
    StalePreindex { index: usize },
    MissingRawTransaction { index: usize },
    InvalidRawTransaction { index: usize, source: ScanError },
    Dependencies(DependencyAuditError),
    MissingExternalPrevout { index: usize, input: usize },
    ImmatureClaimedCoinbase { index: usize, input: usize },
    UnusedCandidateOutpoint { index: usize },
    ExternalCountMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateSnapshotTrustError {
    ParentStateRootNotAuthenticated,
    BitcoinScriptsAndFeesNotVerified,
    SameBlockOutputsNotValidated,
    EconomicStateTransitionUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedParentCoverage {
    external_inputs_matched: usize,
    same_block_outputs_deferred: usize,
    candidate_fingerprint: [u8; 32],
}

impl UnverifiedParentCoverage {
    pub fn status(&self) -> &'static str {
        CANDIDATE_PARENT_UTXO_STATUS
    }
    pub fn external_inputs_matched(&self) -> usize {
        self.external_inputs_matched
    }
    pub fn same_block_outputs_deferred(&self) -> usize {
        self.same_block_outputs_deferred
    }
    pub fn candidate_fingerprint(&self) -> [u8; 32] {
        self.candidate_fingerprint
    }
    pub const fn require_authenticated_parent_state(
        &self,
    ) -> Result<(), CandidateSnapshotTrustError> {
        Err(CandidateSnapshotTrustError::ParentStateRootNotAuthenticated)
    }
    pub const fn require_verified_bitcoin_scripts(
        &self,
    ) -> Result<(), CandidateSnapshotTrustError> {
        Err(CandidateSnapshotTrustError::BitcoinScriptsAndFeesNotVerified)
    }
    pub const fn require_same_block_outputs_validated(
        &self,
    ) -> Result<(), CandidateSnapshotTrustError> {
        Err(CandidateSnapshotTrustError::SameBlockOutputsNotValidated)
    }
    pub const fn require_tradelayer_or_bitcoin_apply(
        &self,
    ) -> Result<(), CandidateSnapshotTrustError> {
        Err(CandidateSnapshotTrustError::EconomicStateTransitionUnavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotWaveError {
    Snapshot(ParentSnapshotError),
    Wave(WaveProbeError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedSnapshotWaveReport {
    pub parent_witness: UnverifiedParentCoverage,
    pub probe: UnverifiedWaveProbeReport,
}

fn encode_candidate_snapshot(snapshot: &CandidateParentSnapshot) -> Result<Vec<u8>, ParentSnapshotError> {
    if snapshot.entries.len() > MAX_CANDIDATE_SNAPSHOT_ENTRIES {
        return Err(ParentSnapshotError::CandidateSnapshotTooLarge);
    }
    let mut out = Vec::new();
    out.extend_from_slice(CANDIDATE_SNAPSHOT_DOMAIN);
    out.extend_from_slice(&snapshot.parent_block_hash_raw);
    out.extend_from_slice(&snapshot.parent_height.to_le_bytes());
    out.extend_from_slice(&(snapshot.entries.len() as u32).to_le_bytes());
    let mut previous: Option<CandidateOutpoint> = None;
    for (index, entry) in snapshot.entries.iter().enumerate() {
        if let Some(prev) = previous {
            if prev == entry.outpoint {
                return Err(ParentSnapshotError::DuplicateCandidateOutpoint { index });
            }
            if prev > entry.outpoint {
                return Err(ParentSnapshotError::UnsortedCandidateOutpoints { index });
            }
        }
        if entry.script_pubkey.len() > MAX_SNAPSHOT_SCRIPT_BYTES {
            return Err(ParentSnapshotError::OversizedCandidateScript { index });
        }
        if entry.amount_sat > BITCOIN_MAX_MONEY_SAT {
            return Err(ParentSnapshotError::InvalidCandidateValue { index });
        }
        if entry.created_height > snapshot.parent_height {
            return Err(ParentSnapshotError::FutureCandidateOutput { index });
        }
        let new_len = 32 + 4 + 8 + 4 + 1 + 4 + entry.script_pubkey.len();
        if out.len().checked_add(new_len).is_none_or(|n| n > MAX_CANDIDATE_SNAPSHOT_BYTES) {
            return Err(ParentSnapshotError::CandidateSnapshotTooLarge);
        }
        out.extend_from_slice(&entry.outpoint.txid_raw);
        out.extend_from_slice(&entry.outpoint.vout.to_le_bytes());
        out.extend_from_slice(&entry.amount_sat.to_le_bytes());
        out.extend_from_slice(&entry.created_height.to_le_bytes());
        out.push(u8::from(entry.is_coinbase));
        out.extend_from_slice(&(entry.script_pubkey.len() as u32).to_le_bytes());
        out.extend_from_slice(&entry.script_pubkey);
        previous = Some(entry.outpoint);
    }
    Ok(out)
}

/// SHA256d over a domain-separated, strictly ordered *untrusted* witness table.
/// A claimant can recalculate this digest for a forged table. It is NOT an
/// authenticated UTXO-set commitment or any Bitcoin Core consensus root.
pub fn candidate_snapshot_fingerprint(
    snapshot: &CandidateParentSnapshot,
) -> Result<[u8; 32], ParentSnapshotError> {
    Ok(double_sha256(&encode_candidate_snapshot(snapshot)?))
}

/// Require an exact one-to-one table for every external input to this block.
/// The entire block's raw transaction identity is recomputed independently of
/// the preindex (including SegWit wtxids), so a preindex for a mutated-witness
/// block under the same TXID Merkle root cannot be reused accidentally.
pub fn inspect_parent_candidate_snapshot(
    view: &UnverifiedRawBlockView<'_>,
    preindex: &UnverifiedRawBlockIndex,
    snapshot: &CandidateParentSnapshot,
) -> Result<UnverifiedParentCoverage, ParentSnapshotError> {
    let parent_hash = view
        .source_block_bytes()
        .get(4..36)
        .expect("UnverifiedRawBlockView already checked its 80-byte header");
    if parent_hash != snapshot.parent_block_hash_raw {
        return Err(ParentSnapshotError::ParentBlockMismatch);
    }
    let spend_height = snapshot
        .parent_height
        .checked_add(1)
        .ok_or(ParentSnapshotError::ParentHeightOverflow)?;
    let fingerprint = candidate_snapshot_fingerprint(snapshot)?;
    if fingerprint != snapshot.expected_fingerprint {
        return Err(ParentSnapshotError::CandidateFingerprintMismatch);
    }
    let plan = view
        .plan_dependency_waves(preindex)
        .map_err(ParentSnapshotError::Dependencies)?;
    let rows = preindex.ordered_preindex().ordered();
    let mut produced_txids = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let raw = view.raw_transaction(index)
            .ok_or(ParentSnapshotError::MissingRawTransaction { index })?;
        let observed = inspect_transaction(raw)
            .map_err(|source| ParentSnapshotError::InvalidRawTransaction { index, source })?;
        if row.position != index || observed != row.observation {
            return Err(ParentSnapshotError::StalePreindex { index });
        }
        produced_txids.insert(observed.txid, index);
    }

    let table: BTreeMap<CandidateOutpoint, (usize, &CandidateParentUtxo)> = snapshot
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.outpoint, (index, entry)))
        .collect();
    let mut consumed = BTreeSet::<CandidateOutpoint>::new();
    let mut external = 0usize;
    let mut deferred = 0usize;
    for index in 1..view.transaction_count() {
        let raw = view.raw_transaction(index)
            .ok_or(ParentSnapshotError::MissingRawTransaction { index })?;
        let layout = prepare_transaction_prefix(raw)
            .map_err(|source| ParentSnapshotError::InvalidRawTransaction { index, source })?;
        let inputs = source_prevouts_from_layout(raw, &layout)
            .map_err(|source| ParentSnapshotError::InvalidRawTransaction { index, source })?;
        for (input, prevout) in inputs.iter().enumerate() {
            if produced_txids.contains_key(&prevout.txid_raw) {
                deferred += 1;
                continue;
            }
            let outpoint = CandidateOutpoint {
                txid_raw: prevout.txid_raw,
                vout: prevout.vout,
            };
            let Some((_, entry)) = table.get(&outpoint) else {
                return Err(ParentSnapshotError::MissingExternalPrevout { index, input });
            };
            if entry.is_coinbase
                && spend_height - entry.created_height < BITCOIN_COINBASE_MATURITY
            {
                return Err(ParentSnapshotError::ImmatureClaimedCoinbase { index, input });
            }
            consumed.insert(outpoint);
            external += 1;
        }
    }
    if let Some((index, _)) = snapshot
        .entries
        .iter()
        .enumerate()
        .find(|(_, entry)| !consumed.contains(&entry.outpoint))
    {
        return Err(ParentSnapshotError::UnusedCandidateOutpoint { index });
    }
    if external != plan.unverified_external_prevouts() {
        return Err(ParentSnapshotError::ExternalCountMismatch);
    }
    if deferred != plan.same_block_edges() {
        return Err(ParentSnapshotError::ExternalCountMismatch);
    }
    Ok(UnverifiedParentCoverage {
        external_inputs_matched: external,
        same_block_outputs_deferred: deferred,
        candidate_fingerprint: fingerprint,
    })
}

/// Compose the candidate-snapshot audit with Stage 8D3 wave barriers.
/// On *any* snapshot inconsistency the read-only callback is never called.
/// Completing both stages is still not Bitcoin Script or TradeLayer consensus.
pub fn inspect_waves_with_candidate_snapshot<F>(
    view: &UnverifiedRawBlockView<'_>,
    preindex: &UnverifiedRawBlockIndex,
    config: ParallelIndexConfig,
    snapshot: &CandidateParentSnapshot,
    probe: F,
) -> Result<UnverifiedSnapshotWaveReport, SnapshotWaveError>
where
    F: Fn(usize, &[u8]) -> UnverifiedProbeVerdict + Sync,
{
    let parent_witness = inspect_parent_candidate_snapshot(view, preindex, snapshot)
        .map_err(SnapshotWaveError::Snapshot)?;
    let probe = inspect_dependency_waves_with_probe(view, preindex, config, probe)
        .map_err(SnapshotWaveError::Wave)?;
    Ok(UnverifiedSnapshotWaveReport {
        parent_witness,
        probe,
    })
}
