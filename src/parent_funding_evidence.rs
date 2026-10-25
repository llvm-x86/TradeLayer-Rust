//! Stage 8D5: match claimed external prevouts against **raw funding TX bytes**.
//!
//! Stage 8D4 checks only a claimant-chosen table plus its claimant-chosen
//! SHA256d fingerprint. This module additionally derives the actual txid and
//! output value/script from one complete raw funding transaction per distinct
//! external funding txid. It rejects a snapshot whose rows are inconsistent
//! with that supplied transaction's bytes.
//!
//! This is still NOT a UTXO membership proof. Any party can invent raw
//! funding transactions, claimed inclusion, heights and prior chainstate.
//! In particular this does not authenticate that an output was created by
//! the active Bitcoin chain, remains unspent, has a mature coinbase origin,
//! or is spendable under its script. There is no Script execution, signature
//! check, UTXO mutation or TradeLayer economic authorization.

use crate::parallel_txindex::ParallelIndexConfig;
use crate::parent_utxo_snapshot::{
    inspect_parent_candidate_snapshot, CandidateParentSnapshot, ParentSnapshotError,
    UnverifiedParentCoverage,
};
use crate::raw_block_ingest::{UnverifiedRawBlockIndex, UnverifiedRawBlockView};
use crate::read_only::{
    inspect_transaction, select_unverified_transaction_output, ScanError, MAX_RAW_TRANSACTION,
};
use crate::wave_precheck::{
    inspect_dependency_waves_with_probe, UnverifiedProbeVerdict, UnverifiedWaveProbeReport,
    WaveProbeError,
};
use std::collections::{BTreeMap, BTreeSet};

pub const RAW_FUNDING_EVIDENCE_STATUS: &str = "raw_funding_prevout_bytes_consistent_unverified";
pub const MAX_RAW_FUNDING_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundingEvidenceTrustError {
    FundingChainInclusionUnknown,
    CurrentUtxoStateUnauthenticated,
    ScriptAndSignaturesNotVerified,
    TradeLayerAndBitcoinStateApplyUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FundingEvidenceError {
    Snapshot(ParentSnapshotError),
    EvidenceTooLarge,
    EmptyFundingTransaction { index: usize },
    FundingTransactionMalformed { index: usize, source: ScanError },
    DuplicateFundingTransaction { index: usize },
    UnsortedFundingTransactions { index: usize },
    UnexpectedFundingTransaction { index: usize },
    MissingFundingTransaction { txid_raw: [u8; 32] },
    FundingOutputOutOfRange { index: usize, vout: u32 },
    ClaimedAmountDiffers { index: usize, vout: u32 },
    ClaimedScriptDiffers { index: usize, vout: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedFundingEvidence {
    pub parent_coverage: UnverifiedParentCoverage,
    funding_transactions_parsed: usize,
    exact_prevouts_matched: usize,
    raw_evidence_bytes: usize,
}

impl UnverifiedFundingEvidence {
    pub fn status(&self) -> &'static str {
        RAW_FUNDING_EVIDENCE_STATUS
    }
    pub fn funding_transactions_parsed(&self) -> usize {
        self.funding_transactions_parsed
    }
    pub fn exact_prevouts_matched(&self) -> usize {
        self.exact_prevouts_matched
    }
    pub fn raw_evidence_bytes(&self) -> usize {
        self.raw_evidence_bytes
    }
    pub const fn require_funding_chain_inclusion(&self) -> Result<(), FundingEvidenceTrustError> {
        Err(FundingEvidenceTrustError::FundingChainInclusionUnknown)
    }
    pub const fn require_authenticated_utxo_set(&self) -> Result<(), FundingEvidenceTrustError> {
        Err(FundingEvidenceTrustError::CurrentUtxoStateUnauthenticated)
    }
    pub const fn require_valid_scripts(&self) -> Result<(), FundingEvidenceTrustError> {
        Err(FundingEvidenceTrustError::ScriptAndSignaturesNotVerified)
    }
    pub const fn require_economic_state_apply(&self) -> Result<(), FundingEvidenceTrustError> {
        Err(FundingEvidenceTrustError::TradeLayerAndBitcoinStateApplyUnavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FundingWaveError {
    Funding(FundingEvidenceError),
    Wave(WaveProbeError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedFundingWaveReport {
    pub funding: UnverifiedFundingEvidence,
    pub probe: UnverifiedWaveProbeReport,
}

/// Compare a complete, strictly ordered witness set of funding transaction
/// bytes against the exact external prevout rows required by this block.
/// The same raw funding TX can provide several vouts in the candidate set,
/// but the raw TX itself is supplied once. No RPC or trusted chainstate.
pub fn inspect_raw_funding_evidence(
    view: &UnverifiedRawBlockView<'_>,
    preindex: &UnverifiedRawBlockIndex,
    snapshot: &CandidateParentSnapshot,
    raw_funding_transactions: &[Vec<u8>],
) -> Result<UnverifiedFundingEvidence, FundingEvidenceError> {
    // Reject all Stage8D4 errors, including mutated SegWit preindex, before
    // consuming any external raw funding witness.
    let parent_coverage = inspect_parent_candidate_snapshot(view, preindex, snapshot)
        .map_err(FundingEvidenceError::Snapshot)?;

    let mut expected = BTreeMap::<[u8; 32], Vec<&_>>::new();
    for entry in &snapshot.entries {
        expected
            .entry(entry.outpoint.txid_raw)
            .or_default()
            .push(entry);
    }

    if raw_funding_transactions.len() > crate::parent_utxo_snapshot::MAX_CANDIDATE_SNAPSHOT_ENTRIES
    {
        return Err(FundingEvidenceError::EvidenceTooLarge);
    }
    let mut encountered = BTreeSet::new();
    let mut previous = None;
    let mut total_bytes = 0usize;
    let mut matched = 0usize;

    for (index, raw) in raw_funding_transactions.iter().enumerate() {
        if raw.is_empty() {
            return Err(FundingEvidenceError::EmptyFundingTransaction { index });
        }
        total_bytes = total_bytes
            .checked_add(raw.len())
            .filter(|value| *value <= MAX_RAW_FUNDING_EVIDENCE_BYTES)
            .ok_or(FundingEvidenceError::EvidenceTooLarge)?;
        if raw.len() > MAX_RAW_TRANSACTION {
            return Err(FundingEvidenceError::EvidenceTooLarge);
        }

        let observation = inspect_transaction(raw).map_err(|source| {
            FundingEvidenceError::FundingTransactionMalformed { index, source }
        })?;
        let txid = observation.txid;
        if let Some(prev) = previous {
            if txid == prev {
                return Err(FundingEvidenceError::DuplicateFundingTransaction { index });
            }
            if txid < prev {
                return Err(FundingEvidenceError::UnsortedFundingTransactions { index });
            }
        }
        let Some(rows) = expected.get(&txid) else {
            return Err(FundingEvidenceError::UnexpectedFundingTransaction { index });
        };
        for entry in rows {
            let vout = entry.outpoint.vout;
            let Some((amount, script)) =
                select_unverified_transaction_output(raw, vout).map_err(|source| {
                    FundingEvidenceError::FundingTransactionMalformed { index, source }
                })?
            else {
                return Err(FundingEvidenceError::FundingOutputOutOfRange { index, vout });
            };
            if amount != entry.amount_sat {
                return Err(FundingEvidenceError::ClaimedAmountDiffers { index, vout });
            }
            if script != entry.script_pubkey.as_slice() {
                return Err(FundingEvidenceError::ClaimedScriptDiffers { index, vout });
            }
            matched += 1;
        }
        encountered.insert(txid);
        previous = Some(txid);
    }
    for txid in expected.keys() {
        if !encountered.contains(txid) {
            return Err(FundingEvidenceError::MissingFundingTransaction { txid_raw: *txid });
        }
    }
    debug_assert_eq!(matched, parent_coverage.external_inputs_matched());
    Ok(UnverifiedFundingEvidence {
        parent_coverage,
        funding_transactions_parsed: raw_funding_transactions.len(),
        exact_prevouts_matched: matched,
        raw_evidence_bytes: total_bytes,
    })
}

/// All raw funding evidence and parent-table checks happen before any wave
/// callback is invoked. A rejected/missing funding source cannot run a
/// partial dependency-wave precheck, let alone mutate protocol state.
pub fn inspect_waves_with_raw_funding_evidence<F>(
    view: &UnverifiedRawBlockView<'_>,
    preindex: &UnverifiedRawBlockIndex,
    config: ParallelIndexConfig,
    snapshot: &CandidateParentSnapshot,
    raw_funding_transactions: &[Vec<u8>],
    probe: F,
) -> Result<UnverifiedFundingWaveReport, FundingWaveError>
where
    F: Fn(usize, &[u8]) -> UnverifiedProbeVerdict + Sync,
{
    let funding = inspect_raw_funding_evidence(view, preindex, snapshot, raw_funding_transactions)
        .map_err(FundingWaveError::Funding)?;
    let probe = inspect_dependency_waves_with_probe(view, preindex, config, probe)
        .map_err(FundingWaveError::Wave)?;
    Ok(UnverifiedFundingWaveReport { funding, probe })
}
