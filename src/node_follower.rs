//! Phase 3 live follower: advances a `JointChainState` by whole blocks taken
//! from a [`BlockSource`], and rolls back through the state's existing
//! rollback history when the source's chain diverges.
//!
//! **Not consensus.** This module does not validate proof of work, chainwork,
//! active-chain status, BIP141 witness commitments, UTXOs or signatures. The
//! block source (a Core node) is TRUSTED. A raw block is checked only for:
//! equality of its SHA256d header hash with the hash the source named, Merkle
//! and TXID consistency via `raw_block_ingest`, and linkage to the state tip
//! via `JointChainState::apply_unverified_block`. Transactions reach the state
//! only as caller-decoded, UNVERIFIED `UnverifiedJointTx` values produced by a
//! [`BlockDecoder`]. See `docs/NODE_FOLLOWER.md`.

use crate::chain::{BlockHeader, BlockId, Tip};
use crate::hash256::double_sha256;
use crate::joint::UnverifiedJointTx;
use crate::joint_chain::{JointChainError, JointChainState, UnverifiedJointBlock};
use crate::ledger::TxId;
use crate::network_profile::{PowFamily, TradeLayerMainnet};
use crate::raw_block_ingest::{
    merkle_root_unverified, RawBlockIngestError, UnverifiedRawBlockView,
};
use std::collections::BTreeMap;

/// Errors from a block source. No variant carries node-supplied text or
/// credentials; `MissingCredential` names an environment variable only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    Config(&'static str),
    MissingCredential(&'static str),
    NonLoopbackRefused,
    Io(std::io::ErrorKind),
    Timeout,
    Unauthorized,
    HttpStatus(u16),
    MalformedHttp(&'static str),
    ResponseTooLarge,
    ShortResponse,
    MalformedJson(&'static str),
    RpcError(i64),
    RpcIdMismatch,
    UnexpectedResult(&'static str),
    BlockTooLarge,
    MalformedHex,
    MissingBlock(u64),
    Mock(&'static str),
}

impl core::fmt::Display for SourceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Config(reason) => write!(f, "invalid block source configuration: {reason}"),
            Self::MissingCredential(var) => {
                write!(f, "environment variable {var} is unset or not unicode")
            }
            Self::NonLoopbackRefused => {
                write!(
                    f,
                    "non-loopback RPC endpoint refused without explicit opt-in"
                )
            }
            Self::Io(kind) => write!(f, "I/O error: {kind:?}"),
            Self::Timeout => write!(f, "RPC connection timed out"),
            Self::Unauthorized => write!(f, "RPC authentication was rejected"),
            Self::HttpStatus(code) => write!(f, "unexpected HTTP status {code}"),
            Self::MalformedHttp(reason) => write!(f, "malformed HTTP response: {reason}"),
            Self::ResponseTooLarge => write!(f, "RPC response exceeds the size bound"),
            Self::ShortResponse => write!(f, "RPC response ended before its declared length"),
            Self::MalformedJson(reason) => write!(f, "malformed JSON-RPC response: {reason}"),
            Self::RpcError(code) => write!(f, "node returned JSON-RPC error code {code}"),
            Self::RpcIdMismatch => write!(f, "JSON-RPC response id mismatch"),
            Self::UnexpectedResult(what) => write!(f, "unexpected {what} in RPC result"),
            Self::BlockTooLarge => write!(f, "raw block exceeds the unverified size bound"),
            Self::MalformedHex => write!(f, "malformed hexadecimal in RPC result"),
            Self::MissingBlock(height) => write!(f, "source has no block at height {height}"),
            Self::Mock(reason) => write!(f, "mock source: {reason}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Read-only access to a trusted node's active chain.
///
/// Heights follow Core conventions: genesis is height 0 and `best_height` is
/// the height of the best block (`getblockcount`). Hashes use display
/// (big-endian hex) order, the same order as [`BlockId`] and `getblockhash`.
pub trait BlockSource {
    /// `getblockcount`.
    fn best_height(&self) -> Result<u64, SourceError>;
    /// `getblockhash(height)`.
    fn block_hash_at(&self, height: u64) -> Result<BlockId, SourceError>;
    /// `getblock(hash, 0)`: the serialized block, not yet verified.
    fn raw_block(&self, hash: BlockId) -> Result<Vec<u8>, SourceError>;
}

/// One structurally verified transaction of a block, with its TXID computed
/// from the raw bytes (SHA256d in internal byte order, as `raw_block_ingest`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodableTx<'a> {
    pub txid: TxId,
    pub raw: &'a [u8],
}

/// Caller-supplied translation of raw transactions into typed operations.
///
/// The crate has no authorized raw-transaction decoder, so none is provided
/// here. Returned operations are UNVERIFIED. Every returned `txid` must occur
/// in `transactions`, in increasing position order, or the block is refused.
pub trait BlockDecoder {
    fn decode_block(
        &self,
        height: u64,
        transactions: &[DecodableTx<'_>],
    ) -> Result<Vec<UnverifiedJointTx>, String>;
}

#[derive(Debug)]
pub enum FollowerError {
    /// Block hashes are verified with SHA256d, which is Bitcoin-only. Litecoin
    /// and Dogecoin block hashes use Scrypt, which this crate does not implement.
    HeaderHashUnavailable(TradeLayerMainnet),
    /// The source's height-0 hash is not the network profile's pinned genesis.
    GenesisMismatch,
    /// The state's anchor is not on the source's active chain.
    AnchorMismatch {
        height: u64,
    },
    /// The divergence is deeper than the state's retained rollback history.
    ReorgExceedsHistory,
    /// More rollbacks and applied blocks than `max_blocks` in one call.
    BudgetExceeded {
        limit: u64,
    },
    Source(SourceError),
    Raw(RawBlockIngestError),
    Decoder(String),
    DecodedTxNotInBlock {
        height: u64,
    },
    DecodedTxOutOfOrder {
        height: u64,
    },
    Rollback(JointChainError),
    Apply(JointChainError),
}

impl core::fmt::Display for FollowerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for FollowerError {}

impl From<SourceError> for FollowerError {
    fn from(value: SourceError) -> Self {
        Self::Source(value)
    }
}

/// Successful outcome of one [`sync_to_source`] call. The state is committed
/// only when the whole call succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub rolled_back: u64,
    pub applied: u64,
    pub tip: Tip,
}

/// Advances `state` to the source's best block, rolling back first if the
/// state's tip is not on the source's active chain.
///
/// All work happens on a clone. The clone replaces `state` only after every
/// rollback and every block has applied. A failure at any point leaves `state`
/// byte-identical to its value before the call. `max_blocks` bounds rollbacks
/// plus applied blocks per call, which also bounds a source that lies about
/// its height.
pub fn sync_to_source<S, D>(
    state: &mut JointChainState,
    source: &S,
    decoder: &D,
    max_blocks: u64,
) -> Result<SyncReport, FollowerError>
where
    S: BlockSource + ?Sized,
    D: BlockDecoder + ?Sized,
{
    let network = state.network();
    if network.pow() != PowFamily::BitcoinSha256d {
        return Err(FollowerError::HeaderHashUnavailable(network));
    }
    if source.block_hash_at(0)? != network.genesis() {
        return Err(FollowerError::GenesisMismatch);
    }
    let best = source.best_height()?;
    let anchor = state.anchor();
    let mut working = state.clone();
    let mut budget = max_blocks;
    let mut rolled_back = 0u64;

    // Walk back until the tip is the source's block at the same height. The
    // stored tip hash is compared with the source's hash, so a reorg is any
    // mismatch at or below the source's best height.
    loop {
        let tip = working.tip();
        if tip.height <= best && source.block_hash_at(tip.height)? == tip.id {
            break;
        }
        if tip.height == anchor.height {
            return Err(FollowerError::AnchorMismatch { height: tip.height });
        }
        spend(&mut budget, max_blocks)?;
        working.rollback_tip(tip).map_err(|err| match err {
            JointChainError::EmptyRollbackHistory => FollowerError::ReorgExceedsHistory,
            other => FollowerError::Rollback(other),
        })?;
        rolled_back += 1;
    }

    let mut applied = 0u64;
    for height in (working.tip().height + 1)..=best {
        spend(&mut budget, max_blocks)?;
        let block = fetch_block(source, decoder, network, height)?;
        working
            .apply_unverified_block(&block)
            .map_err(FollowerError::Apply)?;
        applied += 1;
    }

    let tip = working.tip();
    *state = working;
    Ok(SyncReport {
        rolled_back,
        applied,
        tip,
    })
}

fn spend(budget: &mut u64, limit: u64) -> Result<(), FollowerError> {
    if *budget == 0 {
        return Err(FollowerError::BudgetExceeded { limit });
    }
    *budget -= 1;
    Ok(())
}

fn fetch_block<S, D>(
    source: &S,
    decoder: &D,
    network: TradeLayerMainnet,
    height: u64,
) -> Result<UnverifiedJointBlock, FollowerError>
where
    S: BlockSource + ?Sized,
    D: BlockDecoder + ?Sized,
{
    let hash = source.block_hash_at(height)?;
    let raw = source.raw_block(hash)?;
    // Hash equality with the source's claim, then structural checks through
    // the existing preindex path (TXID Merkle root, duplicate TXIDs, bounds).
    let view = UnverifiedRawBlockView::parse(&raw, Some(display_to_raw(hash)))
        .map_err(FollowerError::Raw)?;
    let index = view.inspect_serial().map_err(FollowerError::Raw)?;
    let txids = index
        .ordered_preindex()
        .ordered()
        .iter()
        .map(|tx| TxId::from_bytes(tx.observation.txid))
        .collect::<Vec<_>>();
    let mut decodable = Vec::with_capacity(txids.len());
    let mut positions = BTreeMap::new();
    for (position, &txid) in txids.iter().enumerate() {
        let raw_tx = view.raw_transaction(position).ok_or(FollowerError::Raw(
            RawBlockIngestError::TruncatedBlockHeader,
        ))?;
        decodable.push(DecodableTx { txid, raw: raw_tx });
        positions.insert(txid, position);
    }
    let transactions = decoder
        .decode_block(height, &decodable)
        .map_err(FollowerError::Decoder)?;
    let mut last: Option<usize> = None;
    for tx in &transactions {
        let position = *positions
            .get(&tx.txid)
            .ok_or(FollowerError::DecodedTxNotInBlock { height })?;
        if last.is_some_and(|previous| position <= previous) {
            return Err(FollowerError::DecodedTxOutOfOrder { height });
        }
        last = Some(position);
    }
    let parent_raw: [u8; 32] = raw
        .get(4..36)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(FollowerError::Raw(
            RawBlockIngestError::TruncatedBlockHeader,
        ))?;
    Ok(UnverifiedJointBlock {
        network,
        header: BlockHeader {
            height,
            id: hash,
            parent: raw_to_display(parent_raw),
        },
        transactions,
    })
}

/// Converts between internal (SHA256d output) and display byte order.
pub(crate) fn display_to_raw(id: BlockId) -> [u8; 32] {
    let mut bytes = id.0;
    bytes.reverse();
    bytes
}

pub(crate) fn raw_to_display(raw: [u8; 32]) -> BlockId {
    let mut bytes = raw;
    bytes.reverse();
    BlockId(bytes)
}

// ---------------------------------------------------------------------------
// In-memory mock source. Test double only: it is never a node and performs no
// consensus checks beyond building linked blocks with correct hashes.
// ---------------------------------------------------------------------------

/// Deliberate misbehaviour for negative tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockFault {
    /// `best_height` reports this height instead of the real tip.
    LyingBestHeight(u64),
    /// `raw_block` at this height returns only the first half of the bytes.
    ShortBlock(u64),
    /// `raw_block` at this height returns bytes beyond the 4 MB raw bound.
    OversizedBlock(u64),
    /// `raw_block` for the block at `height` returns the bytes of block `with`.
    SwappedBlockBytes { height: u64, with: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockBlock {
    id: BlockId,
    raw: Vec<u8>,
    transactions: Vec<Vec<u8>>,
    txids: Vec<TxId>,
}

impl MockBlock {
    pub fn id(&self) -> BlockId {
        self.id
    }
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }
    pub fn transactions(&self) -> &[Vec<u8>] {
        &self.transactions
    }
    pub fn txids(&self) -> &[TxId] {
        &self.txids
    }
}

/// Linked in-memory chain: block `h` sits at index `h - 1`, and genesis is the
/// pinned hash supplied at construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockBlockSource {
    genesis: BlockId,
    blocks: Vec<MockBlock>,
    fault: Option<MockFault>,
}

/// Largest raw block the mock will produce for `MockFault::OversizedBlock`.
const MOCK_OVERSIZED_BYTES: usize = 4_000_001;

impl MockBlockSource {
    pub fn new(genesis: BlockId) -> Self {
        Self {
            genesis,
            blocks: Vec::new(),
            fault: None,
        }
    }

    pub fn for_network(network: TradeLayerMainnet) -> Self {
        Self::new(network.genesis())
    }

    pub fn tip_height(&self) -> u64 {
        self.blocks.len() as u64
    }

    pub fn tip_id(&self) -> BlockId {
        self.blocks.last().map_or(self.genesis, |block| block.id)
    }

    pub fn block(&self, height: u64) -> Option<&MockBlock> {
        let index = usize::try_from(height.checked_sub(1)?).ok()?;
        self.blocks.get(index)
    }

    pub fn set_fault(&mut self, fault: Option<MockFault>) {
        self.fault = fault;
    }

    /// Drops every block above `height`, which is how tests fork a chain.
    pub fn truncate_to(&mut self, height: u64) {
        let keep = usize::try_from(height).unwrap_or(usize::MAX);
        self.blocks.truncate(keep);
    }

    /// Appends one block with the given raw transactions on top of the tip.
    /// `nonce` distinguishes otherwise identical blocks on different branches.
    pub fn push_block(
        &mut self,
        transactions: Vec<Vec<u8>>,
        nonce: u32,
    ) -> Result<BlockId, SourceError> {
        if transactions.is_empty() || transactions.len() >= 0xfd {
            return Err(SourceError::Mock("unsupported transaction count"));
        }
        let txids = transactions
            .iter()
            .map(|tx| TxId::from_bytes(double_sha256(tx)))
            .collect::<Vec<_>>();
        let raw_txids = txids.iter().map(|txid| txid.as_bytes()).collect::<Vec<_>>();
        let merkle =
            merkle_root_unverified(&raw_txids).map_err(|_| SourceError::Mock("merkle root"))?;
        let height = self.tip_height() + 1;
        let time = u32::try_from(1_700_000_000u64 + height).unwrap_or(u32::MAX);
        let mut header = Vec::with_capacity(80);
        header.extend_from_slice(&0x2000_0000u32.to_le_bytes());
        header.extend_from_slice(&display_to_raw(self.tip_id()));
        header.extend_from_slice(&merkle);
        header.extend_from_slice(&time.to_le_bytes());
        header.extend_from_slice(&0x1d00_ffffu32.to_le_bytes());
        header.extend_from_slice(&nonce.to_le_bytes());
        let id = raw_to_display(double_sha256(&header));
        let mut raw = header;
        raw.push(transactions.len() as u8);
        for tx in &transactions {
            raw.extend_from_slice(tx);
        }
        self.blocks.push(MockBlock {
            id,
            raw,
            transactions,
            txids,
        });
        Ok(id)
    }
}

impl BlockSource for MockBlockSource {
    fn best_height(&self) -> Result<u64, SourceError> {
        match self.fault {
            Some(MockFault::LyingBestHeight(height)) => Ok(height),
            _ => Ok(self.tip_height()),
        }
    }

    fn block_hash_at(&self, height: u64) -> Result<BlockId, SourceError> {
        if height == 0 {
            return Ok(self.genesis);
        }
        self.block(height)
            .map(|block| block.id)
            .ok_or(SourceError::MissingBlock(height))
    }

    fn raw_block(&self, hash: BlockId) -> Result<Vec<u8>, SourceError> {
        let index = self
            .blocks
            .iter()
            .position(|block| block.id == hash)
            .ok_or(SourceError::Mock("unknown block hash"))?;
        let height = index as u64 + 1;
        let raw = &self.blocks[index].raw;
        match self.fault {
            Some(MockFault::ShortBlock(at)) if at == height => Ok(raw[..raw.len() / 2].to_vec()),
            Some(MockFault::OversizedBlock(at)) if at == height => {
                Ok(vec![0u8; MOCK_OVERSIZED_BYTES])
            }
            Some(MockFault::SwappedBlockBytes { height: at, with }) if at == height => self
                .block(with)
                .map(|block| block.raw.clone())
                .ok_or(SourceError::MissingBlock(with)),
            _ => Ok(raw.clone()),
        }
    }
}

/// A 60-byte, non-witness transaction with one input and one output. Its
/// `code` byte sits in the prevout hash and `nonce` in the prevout index, so a
/// decoder can recover both. Equal `(code, nonce)` pairs yield equal bytes.
pub fn synthetic_transaction(code: u8, nonce: u32) -> Vec<u8> {
    let mut tx = Vec::with_capacity(60);
    tx.extend_from_slice(&2u32.to_le_bytes());
    tx.push(1);
    tx.extend_from_slice(&[code; 32]);
    tx.extend_from_slice(&nonce.to_le_bytes());
    tx.push(0);
    tx.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    tx.push(1);
    tx.extend_from_slice(&0u64.to_le_bytes());
    tx.push(0);
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx
}

/// Recovers `(code, nonce)` from a transaction built by [`synthetic_transaction`].
pub fn synthetic_fields(raw: &[u8]) -> Option<(u8, u32)> {
    if raw.len() != 60 || raw[4] != 1 || raw[5..37].iter().any(|&b| b != raw[5]) {
        return None;
    }
    let nonce = u32::from_le_bytes(raw[37..41].try_into().ok()?);
    Some((raw[5], nonce))
}
