//! Stage8C: one borrowed Bitcoin-style raw block -> ordered parallel TXID
//! and TradeLayer carrier preindex. No wallet/RPC, new crates or unsafe code.
//!
//! A caller-supplied raw block header and its TXID Merkle root are checked
//! against its transaction bytes. This is NOT full Bitcoin consensus: it
//! does not validate header PoW, cumulative chainwork, active-chain status,
//! coinbase subsidy/witness commitment, scripts, UTXOs, locktimes, fees,
//! double-spend validity, activation, TradeLayer state or real signatures.
//!
//! Raw transaction slices borrow directly from the *single* supplied block
//! buffer: no Vec<Vec<u8>> serialized-TX copying or per-TX RPC is required.
//! CPU-heavy TXID/wtxid/carrier preindex runs on bounded workers, and its
//! results are always restored in canonical block order, with no state writes.
//! A future trusted engine must validate dependencies and apply state changes
//! in order, with atomic rollback on reorg and independently verified chain.

use crate::hash256::double_sha256;
use crate::parallel_txindex::{
    inspect_unverified_block_slices_parallel, inspect_unverified_block_slices_serial,
    ParallelIndexConfig, ParallelIndexError, UnverifiedOrderedBlockIndex,
};
use crate::read_only::{compact_size_prefix, transaction_prefix_lengths, ScanError};
use std::ops::Range;

pub const MAX_UNVERIFIED_RAW_BLOCK_BYTES: usize = 4_000_000;
pub const MAX_UNVERIFIED_BLOCK_WEIGHT: u64 = 4_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawBlockIngestError {
    TruncatedBlockHeader,
    OversizedRawBlock,
    MissingCompactSize(ScanError),
    ZeroTransactions,
    TooManyTransactions,
    TransactionBoundary { index: usize, source: ScanError },
    TrailingRawBlockData,
    BlockWeightExceeded,
    RequestedHeaderHashMismatch,
    InvalidTxidMerkleTree,
    HeaderMerkleRootMismatch,
    TransactionPreindex(ParallelIndexError),
    FetchFailed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawBlockTrustError {
    ActiveChainAndFullConsensusUnverified,
    ScriptAndUtxoStateUnverified,
    TradeLayerStateAndSignedRevisionUnverified,
}

/// A raw block on a borrowed byte buffer. Transaction boundaries are
/// canonical CompactSize positions, not independently authenticated chain
/// membership. The expected header hash (if supplied) is only an equality
/// check against claimed bytes and does NOT authenticate Bitcoin PoW.
#[derive(Debug, Clone)]
pub struct UnverifiedRawBlockView<'a> {
    block_bytes: &'a [u8],
    header_hash_raw: [u8; 32],
    header_merkle_root_raw: [u8; 32],
    transaction_ranges: Vec<Range<usize>>,
    stripped_block_bytes: usize,
    block_weight: u64,
}

impl<'a> UnverifiedRawBlockView<'a> {
    pub fn parse(
        raw_block: &'a [u8],
        expected_header_hash_raw: Option<[u8; 32]>,
    ) -> Result<Self, RawBlockIngestError> {
        if raw_block.len() > MAX_UNVERIFIED_RAW_BLOCK_BYTES {
            return Err(RawBlockIngestError::OversizedRawBlock);
        }
        if raw_block.len() < 80 {
            return Err(RawBlockIngestError::TruncatedBlockHeader);
        }
        let header_hash = double_sha256(&raw_block[..80]);
        if expected_header_hash_raw.is_some_and(|wanted| wanted != header_hash) {
            return Err(RawBlockIngestError::RequestedHeaderHashMismatch);
        }
        let header_merkle_root_raw: [u8; 32] =
            raw_block[36..68].try_into().expect("80-byte header checked");
        let (count, size_of_count) = compact_size_prefix(&raw_block[80..])
            .map_err(RawBlockIngestError::MissingCompactSize)?;
        if count == 0 {
            return Err(RawBlockIngestError::ZeroTransactions);
        }
        if count > crate::parallel_txindex::MAX_BLOCK_TRANSACTIONS as u64 {
            return Err(RawBlockIngestError::TooManyTransactions);
        }
        let mut offsets = Vec::with_capacity(count as usize);
        let mut offset = 80 + size_of_count;
        let mut stripped_size = offset;
        for index in 0..count as usize {
            let remaining = raw_block.get(offset..).ok_or(
                RawBlockIngestError::TransactionBoundary {
                    index,
                    source: ScanError::Truncated,
                },
            )?;
            let (length, base_length) = transaction_prefix_lengths(remaining)
                .map_err(|source| RawBlockIngestError::TransactionBoundary { index, source })?;
            let end = offset.checked_add(length).ok_or(
                RawBlockIngestError::TransactionBoundary {
                    index,
                    source: ScanError::TooLong,
                },
            )?;
            offsets.push(offset..end);
            offset = end;
            stripped_size = stripped_size
                .checked_add(base_length)
                .ok_or(RawBlockIngestError::BlockWeightExceeded)?;
        }
        if offset != raw_block.len() {
            return Err(RawBlockIngestError::TrailingRawBlockData);
        }
        let block_weight = stripped_size
            .checked_mul(3)
            .and_then(|base| base.checked_add(raw_block.len()))
            .and_then(|total| u64::try_from(total).ok())
            .ok_or(RawBlockIngestError::BlockWeightExceeded)?;
        if block_weight > MAX_UNVERIFIED_BLOCK_WEIGHT {
            return Err(RawBlockIngestError::BlockWeightExceeded);
        }
        Ok(Self {
            block_bytes: raw_block,
            header_hash_raw: header_hash,
            header_merkle_root_raw,
            transaction_ranges: offsets,
            stripped_block_bytes: stripped_size,
            block_weight,
        })
    }

    pub const fn header_hash_raw(&self) -> [u8; 32] {
        self.header_hash_raw
    }
    pub const fn header_merkle_root_raw(&self) -> [u8; 32] {
        self.header_merkle_root_raw
    }
    pub fn transaction_count(&self) -> usize {
        self.transaction_ranges.len()
    }
    pub fn transaction_ranges(&self) -> &[Range<usize>] {
        &self.transaction_ranges
    }
    pub fn raw_transaction(&self, index: usize) -> Option<&'a [u8]> {
        self.transaction_ranges
            .get(index)
            .and_then(|range| self.block_bytes.get(range.clone()))
    }
    pub fn source_block_bytes(&self) -> &'a [u8] {
        self.block_bytes
    }
    pub const fn block_weight(&self) -> u64 {
        self.block_weight
    }
    pub const fn stripped_block_bytes(&self) -> usize {
        self.stripped_block_bytes
    }

    fn index(
        &self,
        config: ParallelIndexConfig,
        parallel: bool,
    ) -> Result<UnverifiedRawBlockIndex, RawBlockIngestError> {
        let slices: Vec<&[u8]> = self.transaction_ranges.iter()
            .map(|range| &self.block_bytes[range.clone()])
            .collect();
        let report = if parallel {
            inspect_unverified_block_slices_parallel(&slices, config)
        } else {
            inspect_unverified_block_slices_serial(&slices)
        }
        .map_err(RawBlockIngestError::TransactionPreindex)?;
        let txids = report.ordered().iter()
            .map(|item| item.observation.txid).collect::<Vec<_>>();
        let root = merkle_root_unverified(&txids)?;
        if root != self.header_merkle_root_raw {
            return Err(RawBlockIngestError::HeaderMerkleRootMismatch);
        }
        Ok(UnverifiedRawBlockIndex {
            header_hash_raw: self.header_hash_raw,
            header_merkle_root_raw: root,
            block_weight: self.block_weight,
            stripped_block_bytes: self.stripped_block_bytes,
            inspected: report,
        })
    }

    pub fn inspect_parallel(
        &self,
        config: ParallelIndexConfig,
    ) -> Result<UnverifiedRawBlockIndex, RawBlockIngestError> {
        self.index(config, true)
    }
    pub fn inspect_serial(&self) -> Result<UnverifiedRawBlockIndex, RawBlockIngestError> {
        self.index(ParallelIndexConfig::serial(), false)
    }

    pub const fn require_full_bitcoin_consensus(&self) -> Result<(), RawBlockTrustError> {
        Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified)
    }
}

/// Bounded pairwise Bitcoin TXID Merkle SHA256d. This detects adjacent
/// identical nodes instead of accepting a mutated duplicated subtree.
/// At an odd level, Bitcoin repeats the final node only for calculation.
/// The helper is intentionally available to synthetic fixture generators;
/// matching a merkle root is not verification of PoW, coinbase or UTXO.
pub fn merkle_root_unverified(
    txids_raw: &[[u8; 32]],
) -> Result<[u8; 32], RawBlockIngestError> {
    if txids_raw.is_empty() {
        return Err(RawBlockIngestError::ZeroTransactions);
    }
    if txids_raw.len() > crate::parallel_txindex::MAX_BLOCK_TRANSACTIONS {
        return Err(RawBlockIngestError::TooManyTransactions);
    }
    let mut level = txids_raw.to_vec();
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            if pair.len() == 2 && pair[0] == pair[1] {
                return Err(RawBlockIngestError::InvalidTxidMerkleTree);
            }
            let mut concat = [0u8; 64];
            concat[..32].copy_from_slice(&pair[0]);
            concat[32..].copy_from_slice(pair.get(1).unwrap_or(&pair[0]));
            next.push(double_sha256(&concat));
        }
        level = next;
    }
    Ok(level[0])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedRawBlockIndex {
    header_hash_raw: [u8; 32],
    header_merkle_root_raw: [u8; 32],
    stripped_block_bytes: usize,
    block_weight: u64,
    inspected: UnverifiedOrderedBlockIndex,
}
impl UnverifiedRawBlockIndex {
    pub const fn header_hash_raw(&self) -> [u8; 32] {
        self.header_hash_raw
    }
    pub const fn txid_merkle_root_raw(&self) -> [u8; 32] {
        self.header_merkle_root_raw
    }
    pub const fn block_weight(&self) -> u64 {
        self.block_weight
    }
    pub const fn stripped_block_bytes(&self) -> usize {
        self.stripped_block_bytes
    }
    pub fn ordered_preindex(&self) -> &UnverifiedOrderedBlockIndex {
        &self.inspected
    }
    pub const fn require_authenticated_block(&self) -> Result<(), RawBlockTrustError> {
        Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified)
    }
    pub const fn require_script_and_utxo_consensus(&self) -> Result<(), RawBlockTrustError> {
        Err(RawBlockTrustError::ScriptAndUtxoStateUnverified)
    }
    pub const fn require_tradelayer_state_transition(&self) -> Result<(), RawBlockTrustError> {
        Err(RawBlockTrustError::TradeLayerStateAndSignedRevisionUnverified)
    }
}

/// Read-only mock RPC boundary: one getblock(blockhash,0) equivalent, with
/// hash equality checked against caller's requested header hash. Every
/// transaction thereafter is inspected as a borrow of that ONE buffer.
/// A successful return has NO validated chain or state authority.
pub fn fetch_and_inspect_unverified_block<F>(
    expected_header_hash_raw: [u8; 32],
    config: ParallelIndexConfig,
    fetch_raw_block: F,
) -> Result<UnverifiedRawBlockIndex, RawBlockIngestError>
where
    F: FnOnce([u8; 32]) -> Result<Vec<u8>, String>,
{
    let raw = fetch_raw_block(expected_header_hash_raw)
        .map_err(RawBlockIngestError::FetchFailed)?;
    let view = UnverifiedRawBlockView::parse(&raw, Some(expected_header_hash_raw))?;
    view.inspect_parallel(config)
}
