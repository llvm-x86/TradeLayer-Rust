//! Stage 25M: bounded Bitcoin BIP141 witness-commitment *diagnostic*.
//!
//! Checks complete caller-supplied, ordered transaction bytes against one
//! locally checked Bitcoin SHA256d header. This is NOT full block validation:
//! header anchor, active chain, UTXO state, script execution, BIP143/ECDSA,
//! weight, consensus activation and channel-state authority remain UNVERIFIED.
//!
//! Bitcoin's txid Merkle root excludes witness bytes. BIP141 commits separately
//! to the wtxid tree, with coinbase leaf zero and the 32-byte coinbase witness
//! reserved value. A perfectly matching synthetic block may contain fake
//! signatures and never authorizes a TradeLayer ledger or clearing operation.

use crate::bitcoin_inclusion::CheckedHeaderSegment;
use crate::chain::Tip;
use crate::channel_witness_audit::Cursor;
use crate::hash256::double_sha256;
use crate::network_profile::TradeLayerMainnet;
use crate::read_only::{inspect_transaction, ScanError};
use std::collections::BTreeSet;

pub const MAX_UNVERIFIED_BLOCK_TRANSACTIONS: usize = 256;
pub const MAX_UNVERIFIED_BLOCK_BYTES: usize = 4_000_000;
const COMMITMENT_PREFIX: [u8; 6] = [0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessBlockError {
    UnsupportedNetwork,
    InvalidBlockHeight,
    InvalidTransactionCount,
    ExcessiveBlockBytes,
    InvalidSelectedInput,
    SelectedInputHasNoWitness,
    RawTransaction(ScanError),
    MalformedCoinbase,
    UnexpectedCoinbase,
    MissingCoinbaseCommitment,
    InvalidCoinbaseReservedValue,
    DuplicateTransactionId,
    MutatedMerkleTree,
    HeaderMerkleRootMismatch,
    WitnessCommitmentMismatch,
    ActiveChainAndChannelAuthorizationUnavailable,
}

impl From<ScanError> for WitnessBlockError {
    fn from(value: ScanError) -> Self {
        Self::RawTransaction(value)
    }
}

/// Opaque observation, scoped to supplied data and a CALLER-CLAIMED anchor.
/// It is deliberately incapable of producing a trusted channel spend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedBitcoinWitnessBlock {
    claimed_anchor: Tip,
    claimed_block: Tip,
    transaction_count: usize,
    selected_txid_raw: [u8; 32],
    selected_wtxid_raw: [u8; 32],
    txid_merkle_root_raw: [u8; 32],
    witness_merkle_root_raw: [u8; 32],
    committed_witness_hash_raw: [u8; 32],
}

impl UnverifiedBitcoinWitnessBlock {
    pub const fn claimed_anchor(&self) -> Tip {
        self.claimed_anchor
    }
    pub const fn claimed_block(&self) -> Tip {
        self.claimed_block
    }
    pub const fn transaction_count(&self) -> usize {
        self.transaction_count
    }
    pub const fn selected_txid_raw(&self) -> [u8; 32] {
        self.selected_txid_raw
    }
    pub const fn selected_wtxid_raw(&self) -> [u8; 32] {
        self.selected_wtxid_raw
    }
    pub const fn txid_merkle_root_raw(&self) -> [u8; 32] {
        self.txid_merkle_root_raw
    }
    pub const fn witness_merkle_root_raw(&self) -> [u8; 32] {
        self.witness_merkle_root_raw
    }
    pub const fn committed_witness_hash_raw(&self) -> [u8; 32] {
        self.committed_witness_hash_raw
    }
    /// Even valid SHA256d commitments are not proof of Bitcoin consensus,
    /// witness/script validity, UTXO existence or channel-state consent.
    pub const fn require_authenticated_channel_spend(&self) -> Result<(), WitnessBlockError> {
        Err(WitnessBlockError::ActiveChainAndChannelAuthorizationUnavailable)
    }
}

/// Bitcoin Merkle combining with exact odd-last-leaf padding. Two *real*
/// sibling hashes may not be identical: this is the mutated-tree ambiguity
/// distinguished from a virtual odd-width duplicate.
fn merkle_root(mut level: Vec<[u8; 32]>) -> Result<[u8; 32], WitnessBlockError> {
    if level.is_empty() {
        return Err(WitnessBlockError::InvalidTransactionCount);
    }
    while level.len() > 1 {
        let count = level.len();
        let mut next = Vec::with_capacity(count.div_ceil(2));
        for index in (0..count).step_by(2) {
            let left = level[index];
            let right = if index + 1 < count {
                let right = level[index + 1];
                if left == right {
                    return Err(WitnessBlockError::MutatedMerkleTree);
                }
                right
            } else {
                left
            };
            let mut pair = [0u8; 64];
            pair[..32].copy_from_slice(&left);
            pair[32..].copy_from_slice(&right);
            next.push(double_sha256(&pair));
        }
        level = next;
    }
    Ok(level[0])
}

/// Inspect precisely one coinbase input, its witness reserved value and
/// BIP141 highest-index output commitment. Trailing scriptPubKey bytes are
/// allowed by BIP141; only the leading 38 bytes define the commitment.
fn coinbase_witness_commitment(raw: &[u8]) -> Result<([u8; 32], [u8; 32]), WitnessBlockError> {
    if raw.get(4) != Some(&0) || raw.get(5) != Some(&1) {
        return Err(WitnessBlockError::InvalidCoinbaseReservedValue);
    }
    let mut cursor = Cursor::new(raw);
    cursor.take(6)?; // version and canonical marker/flag (also checked by inspect_transaction)
    if cursor.count()? != 1 {
        return Err(WitnessBlockError::MalformedCoinbase);
    }
    let outpoint = cursor.take(36)?;
    if outpoint[..32] != [0u8; 32] || outpoint[32..] != [0xffu8; 4] {
        return Err(WitnessBlockError::MalformedCoinbase);
    }
    let script_sig = cursor.data()?;
    if !(2..=100).contains(&script_sig.len()) {
        return Err(WitnessBlockError::MalformedCoinbase);
    }
    cursor.take(4)?; // nSequence
    let outputs = cursor.count()?;
    let mut last_commitment = None;
    for _ in 0..outputs {
        cursor.take(8)?; // satoshis
        let script_pubkey = cursor.data()?;
        if script_pubkey.len() >= 38 && script_pubkey[..6] == COMMITMENT_PREFIX {
            let commitment: [u8; 32] = script_pubkey[6..38]
                .try_into()
                .expect("checked commitment length");
            // Highest matching output index has BIP141 precedence.
            last_commitment = Some(commitment);
        }
    }
    if cursor.compact()? != 1 {
        return Err(WitnessBlockError::InvalidCoinbaseReservedValue);
    }
    let reserved = cursor.data()?;
    if reserved.len() != 32 {
        return Err(WitnessBlockError::InvalidCoinbaseReservedValue);
    }
    cursor.take(4)?; // nLockTime
    if cursor.offset() != raw.len() {
        return Err(ScanError::TrailingBytes.into());
    }
    Ok((
        reserved.try_into().expect("32-byte reserved value"),
        last_commitment.ok_or(WitnessBlockError::MissingCoinbaseCommitment)?,
    ))
}

/// The coinbase prevout is permitted at transaction index zero only.
/// Recheck ALL inputs to reject a fake coinbase hidden at any later index.
fn forbid_nonfirst_coinbase(raw: &[u8], has_witness: bool) -> Result<(), WitnessBlockError> {
    let mut cursor = Cursor::new(raw);
    cursor.take(if has_witness { 6 } else { 4 })?;
    let inputs = cursor.count()?;
    for _ in 0..inputs {
        let prevout = cursor.take(36)?;
        if prevout[..32] == [0u8; 32] && prevout[32..] == [0xffu8; 4] {
            return Err(WitnessBlockError::UnexpectedCoinbase);
        }
        cursor.data()?; // scriptSig
        cursor.take(4)?; // nSequence
    }
    Ok(())
}

/// Check a COMPLETE caller-supplied transaction list, from coinbase onward,
/// against a selected header within the *same* checked Bitcoin segment.
/// selected_spending_index must refer to an actual SegWit transaction; its
/// txid/wtxid are retained for later untrusted cross-reference to Stage25L.
///
/// Full block consensus and best-chain/UTXO membership are beyond this API.
pub fn audit_bitcoin_bip141_witness_block_unverified(
    network: TradeLayerMainnet,
    segment: &CheckedHeaderSegment,
    claimed_height: u64,
    ordered_raw_transactions: &[&[u8]],
    selected_spending_index: usize,
) -> Result<UnverifiedBitcoinWitnessBlock, WitnessBlockError> {
    if network != TradeLayerMainnet::Bitcoin {
        return Err(WitnessBlockError::UnsupportedNetwork);
    }
    if !(2..=MAX_UNVERIFIED_BLOCK_TRANSACTIONS).contains(&ordered_raw_transactions.len()) {
        return Err(WitnessBlockError::InvalidTransactionCount);
    }
    if selected_spending_index == 0 || selected_spending_index >= ordered_raw_transactions.len() {
        return Err(WitnessBlockError::InvalidSelectedInput);
    }
    let index = claimed_height
        .checked_sub(segment.anchor().height)
        .and_then(|delta| delta.checked_sub(1))
        .and_then(|delta| usize::try_from(delta).ok())
        .ok_or(WitnessBlockError::InvalidBlockHeight)?;
    let header = segment
        .headers()
        .get(index)
        .ok_or(WitnessBlockError::InvalidBlockHeight)?;

    let mut total_bytes = 0usize;
    let mut txids = Vec::with_capacity(ordered_raw_transactions.len());
    let mut wtxids = Vec::with_capacity(ordered_raw_transactions.len());
    let mut seen = BTreeSet::new();
    let mut selected_txid = None;
    let mut selected_wtxid = None;
    for (i, raw) in ordered_raw_transactions.iter().enumerate() {
        total_bytes = total_bytes
            .checked_add(raw.len())
            .ok_or(WitnessBlockError::ExcessiveBlockBytes)?;
        if total_bytes > MAX_UNVERIFIED_BLOCK_BYTES {
            return Err(WitnessBlockError::ExcessiveBlockBytes);
        }
        let inspected = inspect_transaction(raw)?;
        if i != 0 {
            forbid_nonfirst_coinbase(raw, inspected.has_witness)?;
        }
        if !seen.insert(inspected.txid) {
            return Err(WitnessBlockError::DuplicateTransactionId);
        }
        txids.push(inspected.txid);
        wtxids.push(if i == 0 { [0u8; 32] } else { inspected.wtxid });
        if i == selected_spending_index {
            if !inspected.has_witness {
                return Err(WitnessBlockError::SelectedInputHasNoWitness);
            }
            selected_txid = Some(inspected.txid);
            selected_wtxid = Some(inspected.wtxid);
        }
    }

    let (reserved, commitment) = coinbase_witness_commitment(ordered_raw_transactions[0])?;
    let txid_merkle = merkle_root(txids)?;
    if txid_merkle != header.merkle_root_raw() {
        return Err(WitnessBlockError::HeaderMerkleRootMismatch);
    }
    let witness_merkle = merkle_root(wtxids)?;
    let mut preimage = [0u8; 64];
    preimage[..32].copy_from_slice(&witness_merkle);
    preimage[32..].copy_from_slice(&reserved);
    if double_sha256(&preimage) != commitment {
        return Err(WitnessBlockError::WitnessCommitmentMismatch);
    }
    Ok(UnverifiedBitcoinWitnessBlock {
        claimed_anchor: segment.anchor(),
        claimed_block: header.tip(),
        transaction_count: ordered_raw_transactions.len(),
        selected_txid_raw: selected_txid.expect("focus index validated"),
        selected_wtxid_raw: selected_wtxid.expect("focus witness validated"),
        txid_merkle_root_raw: txid_merkle,
        witness_merkle_root_raw: witness_merkle,
        committed_witness_hash_raw: commitment,
    })
}
