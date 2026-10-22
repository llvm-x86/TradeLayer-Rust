//! Reversible block progression with an explicit trusted-input boundary.
//!
//! **Prototype, not blockchain consensus.** The TrustedChainAdapter trait is a
//! required external security dependency. A real adapter must verify the block
//! header/chain linkage, transaction bytes, on-chain sender authorization, and
//! the correspondence between signed bytes and decoded operations. This crate
//! cannot perform Bitcoin/Litecoin/Dogecoin signature or UTXO verification yet.
//!
//! Caller assertions alone must never count as authentication. Tests use an
//! intentionally artificial adapter, which MUST NOT be used in production.

use crate::codec::{with_crc, verify_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::ledger::{AccountId, Ledger, LedgerError, Operation, SnapshotError, TxId};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

pub const MAX_REORG_HISTORY: usize = 64;
pub const MAX_TRANSACTIONS_PER_BLOCK: usize = 10_000;
pub const MAX_TRANSACTION_BYTES: usize = 1_000_000;

/// A chain's immutable namespace, normally derived from verified genesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChainId(pub [u8; 32]);

/// Header commitment, supplied and validated by the trusted adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub [u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tip {
    pub height: u64,
    pub id: BlockId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    pub height: u64,
    pub id: BlockId,
    pub parent: BlockId,
}

/// Raw on-chain material. Every byte must be interpreted by a trusted adapter.
/// There is no sender, permission, or parsed operation supplied by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateBlock {
    pub chain: ChainId,
    pub header: BlockHeader,
    pub raw_transactions: Vec<Vec<u8>>,
}

/// Adapter output, valid ONLY when created from actual verified chain data.
/// The adapter is part of the trusted computing base (TCB), and can make an
/// unsafe system if it accepts metadata or unsigned actor claims as proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedOperation {
    pub txid: TxId,
    pub origin: AccountId,
    pub operation: Operation,
}

/// Implementations MUST derive txid, actor, and operation from verified bytes.
/// `verify_block` MUST prove the header and chain consensus as appropriate.
/// Neither function should trust caller-provided operations or arbitrary IDs.
pub trait TrustedChainAdapter {
    fn verify_block(&self, block: &CandidateBlock) -> Result<(), String>;

    fn authenticate_and_decode(
        &self,
        chain: ChainId,
        header: &BlockHeader,
        raw_transaction: &[u8],
    ) -> Result<AuthenticatedOperation, String>;
}

#[derive(Debug)]
pub enum ChainError {
    InvalidHistoryLimit,
    WrongChain,
    BadHeight,
    BadParent,
    DuplicateBlockId,
    TransactionLimit,
    TransactionSize,
    Authentication(String),
    ActorMismatch,
    Ledger(LedgerError),
    EmptyRollbackHistory,
    WrongRollbackTip,
    Checkpoint(CheckpointError),
}

impl core::fmt::Display for ChainError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ChainError {}

#[derive(Debug)]
pub enum CheckpointError {
    Invalid(&'static str),
    Ledger(SnapshotError),
    Io(std::io::Error),
}

impl core::fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for CheckpointError {}

impl From<LedgerError> for ChainError {
    fn from(value: LedgerError) -> Self { Self::Ledger(value) }
}

impl From<SnapshotError> for CheckpointError {
    fn from(value: SnapshotError) -> Self { Self::Ledger(value) }
}

impl From<std::io::Error> for CheckpointError {
    fn from(value: std::io::Error) -> Self { Self::Io(value) }
}

impl From<CheckpointError> for ChainError {
    fn from(value: CheckpointError) -> Self { Self::Checkpoint(value) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PriorState {
    tip: Tip,
    ledger: Ledger,
}

/// Single-chain projection with bounded reversible snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainState {
    chain: ChainId,
    anchor: Tip,
    tip: Tip,
    ledger: Ledger,
    history: VecDeque<PriorState>,
    history_limit: usize,
}

impl ChainState {
    /// Creates a projection beginning immediately after a trusted anchor.
    /// The anchor must come from independently verified chain configuration.
    pub fn new(chain: ChainId, anchor: Tip, history_limit: usize) -> Result<Self, ChainError> {
        if !(1..=MAX_REORG_HISTORY).contains(&history_limit) {
            return Err(ChainError::InvalidHistoryLimit);
        }
        Ok(Self {
            chain, anchor, tip: anchor, ledger: Ledger::new(),
            history: VecDeque::new(), history_limit,
        })
    }

    pub fn ledger(&self) -> &Ledger { &self.ledger }
    pub fn tip(&self) -> Tip { self.tip }
    pub fn chain_id(&self) -> ChainId { self.chain }
    pub fn rollback_depth(&self) -> usize { self.history.len() }

    /// Rejects unverified inputs, binds each operation actor to its origin,
    /// then commits a block atomically in transaction order.
    pub fn apply_block<A: TrustedChainAdapter>(
        &mut self,
        block: &CandidateBlock,
        adapter: &A,
    ) -> Result<(), ChainError> {
        if block.chain != self.chain { return Err(ChainError::WrongChain); }
        if Some(block.header.height) != self.tip.height.checked_add(1) {
            return Err(ChainError::BadHeight);
        }
        if block.header.parent != self.tip.id { return Err(ChainError::BadParent); }
        if block.header.id == self.tip.id { return Err(ChainError::DuplicateBlockId); }
        if block.raw_transactions.len() > MAX_TRANSACTIONS_PER_BLOCK {
            return Err(ChainError::TransactionLimit);
        }
        adapter.verify_block(block).map_err(ChainError::Authentication)?;
        let mut transactions = Vec::with_capacity(block.raw_transactions.len());
        for raw in &block.raw_transactions {
            if raw.is_empty() || raw.len() > MAX_TRANSACTION_BYTES {
                return Err(ChainError::TransactionSize);
            }
            let authenticated = adapter.authenticate_and_decode(self.chain, &block.header, raw)
                .map_err(ChainError::Authentication)?;
            if !origin_matches_operation(&authenticated.origin, &authenticated.operation) {
                return Err(ChainError::ActorMismatch);
            }
            transactions.push((authenticated.txid, authenticated.operation));
        }
        let mut pending = self.ledger.clone();
        pending.apply_batch(transactions)?;
        let previous = PriorState { tip: self.tip, ledger: self.ledger.clone() };
        self.history.push_back(previous);
        while self.history.len() > self.history_limit {
            self.history.pop_front();
        }
        self.ledger = pending;
        self.tip = Tip { height: block.header.height, id: block.header.id };
        Ok(())
    }

    /// Undo exactly the current tip, including all replay markers introduced
    /// by that block. The expected tip prevents accidentally rolling back a
    /// different branch. Retention bounds the maximum supported reorg.
    pub fn rollback_tip(&mut self, expected: Tip) -> Result<(), ChainError> {
        if self.tip != expected { return Err(ChainError::WrongRollbackTip); }
        let previous = self.history.pop_back().ok_or(ChainError::EmptyRollbackHistory)?;
        self.tip = previous.tip;
        self.ledger = previous.ledger;
        Ok(())
    }

    /// Apply, durably save a NEW checkpoint, then publish the in-memory state.
    /// Checkpoint paths are immutable: a pre-existing destination is rejected.
    pub fn apply_block_and_checkpoint<A: TrustedChainAdapter>(
        &mut self, block: &CandidateBlock, adapter: &A, path: &Path,
    ) -> Result<(), ChainError> {
        let mut proposed = self.clone();
        proposed.apply_block(block, adapter)?;
        proposed.write_new_checkpoint(path)?;
        *self = proposed;
        Ok(())
    }

    /// Deterministic, CRC-protected serialization of tip, ledger, and bounded
    /// rollback history. CRC32 detects accidents, not hostile tampering.
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, CheckpointError> {
        let mut data = Vec::new();
        data.extend_from_slice(b"TLCS\x01");
        data.extend_from_slice(&self.chain.0);
        write_tip(&mut data, self.anchor);
        write_tip(&mut data, self.tip);
        data.push(self.history_limit as u8);
        data.push(self.history.len() as u8);
        for previous in &self.history {
            write_tip(&mut data, previous.tip);
            write_ledger(&mut data, &previous.ledger)?;
        }
        write_ledger(&mut data, &self.ledger)?;
        with_crc(data).map_err(|_| CheckpointError::Invalid("checkpoint too large"))
    }

    /// On restore, the expected chain, anchor, and rollback policy MUST be
    /// supplied independently of checkpoint bytes.
    pub fn from_checkpoint_bytes(
        bytes: &[u8], expected_chain: ChainId, expected_anchor: Tip,
        expected_history_limit: usize,
    ) -> Result<Self, CheckpointError> {
        if !(1..=MAX_REORG_HISTORY).contains(&expected_history_limit) {
            return Err(CheckpointError::Invalid("invalid history limit"));
        }
        let payload = verify_crc(bytes).map_err(|_| CheckpointError::Invalid("checkpoint CRC/size"))?;
        let mut cursor = Cursor::new(payload);
        if cursor.fixed::<5>().map_err(|_| CheckpointError::Invalid("checkpoint truncated"))?
            != *b"TLCS\x01" {
            return Err(CheckpointError::Invalid("checkpoint magic/version"));
        }
        let chain = ChainId(cursor.fixed::<32>().map_err(|_| CheckpointError::Invalid("chain id"))?);
        let anchor = read_tip(&mut cursor)?;
        let tip = read_tip(&mut cursor)?;
        if chain != expected_chain || anchor != expected_anchor {
            return Err(CheckpointError::Invalid("checkpoint belongs to different chain/anchor"));
        }
        let history_limit = usize::from(cursor.u8().map_err(|_| CheckpointError::Invalid("history limit"))?);
        if history_limit != expected_history_limit {
            return Err(CheckpointError::Invalid("checkpoint history policy changed"));
        }
        let count = usize::from(cursor.u8().map_err(|_| CheckpointError::Invalid("history count"))?);
        if count > history_limit { return Err(CheckpointError::Invalid("too many rollback entries")); }
        let mut history: VecDeque<PriorState> = VecDeque::new();
        let mut last_height = None;
        for _ in 0..count {
            let preceding_tip = read_tip(&mut cursor)?;
            if preceding_tip.height < anchor.height ||
                last_height.is_some_and(|height: u64| height.checked_add(1) != Some(preceding_tip.height)) {
                return Err(CheckpointError::Invalid("noncontiguous rollback history"));
            }
            last_height = Some(preceding_tip.height);
            let preceding_ledger = read_ledger(&mut cursor)?;
            history.push_back(PriorState { tip: preceding_tip, ledger: preceding_ledger });
        }
        if tip.height < anchor.height ||
            (tip.height == anchor.height && tip.id != anchor.id) ||
            (tip.height == anchor.height && count != 0) ||
            (tip.height > anchor.height && count == 0) {
            return Err(CheckpointError::Invalid("invalid tip"));
        }
        if let Some(last) = history.back() {
            if last.tip.height.checked_add(1) != Some(tip.height) || last.tip.id == tip.id {
                return Err(CheckpointError::Invalid("rollback tip mismatch"));
            }
        }
        let ledger = read_ledger(&mut cursor)?;
        cursor.end().map_err(|_| CheckpointError::Invalid("trailing checkpoint bytes"))?;
        Ok(Self { chain, anchor, tip, ledger, history, history_limit })
    }

    /// Save an immutable checkpoint using create_new + sync_all. Keeping prior
    /// checkpoint files untouched provides a restart fallback after crashes.
    /// A filesystem crash can still leave a truncated *new* file, which the
    /// decoder rejects; this API does not provide directory fsync portability.
    pub fn write_new_checkpoint(&self, path: &Path) -> Result<(), CheckpointError> {
        let bytes = self.checkpoint_bytes()?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        if let Err(err) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(CheckpointError::Io(err));
        }
        Ok(())
    }

    pub fn load_checkpoint(
        path: &Path, expected_chain: ChainId, expected_anchor: Tip,
        expected_history_limit: usize,
    ) -> Result<Self, CheckpointError> {
        let mut f = File::open(path)?;
        let len = usize::try_from(f.metadata()?.len())
            .map_err(|_| CheckpointError::Invalid("checkpoint file too large"))?;
        if len > MAX_CHECKPOINT_BYTES {
            return Err(CheckpointError::Invalid("checkpoint file too large"));
        }
        let mut bytes = Vec::with_capacity(len);
        f.take((MAX_CHECKPOINT_BYTES + 1) as u64).read_to_end(&mut bytes)?;
        Self::from_checkpoint_bytes(&bytes, expected_chain, expected_anchor, expected_history_limit)
    }
}

fn origin_matches_operation(origin: &AccountId, operation: &Operation) -> bool {
    match operation {
        Operation::CreateProperty { issuer, .. } => origin == issuer,
        Operation::Transfer { from, .. } => origin == from,
        Operation::Mint { issuer, .. } => origin == issuer,
        Operation::Burn { issuer, .. } => origin == issuer,
    }
}

fn write_tip(dst: &mut Vec<u8>, tip: Tip) {
    dst.extend_from_slice(&tip.height.to_be_bytes());
    dst.extend_from_slice(&tip.id.0);
}

fn read_tip(cursor: &mut Cursor<'_>) -> Result<Tip, CheckpointError> {
    Ok(Tip {
        height: cursor.u64().map_err(|_| CheckpointError::Invalid("tip height"))?,
        id: BlockId(cursor.fixed::<32>().map_err(|_| CheckpointError::Invalid("tip hash"))?),
    })
}

fn write_ledger(dst: &mut Vec<u8>, ledger: &Ledger) -> Result<(), CheckpointError> {
    let bytes = ledger.snapshot_bytes()?;
    let len = u32::try_from(bytes.len()).map_err(|_| CheckpointError::Invalid("ledger too large"))?;
    dst.extend_from_slice(&len.to_be_bytes());
    dst.extend_from_slice(&bytes);
    Ok(())
}

fn read_ledger(cursor: &mut Cursor<'_>) -> Result<Ledger, CheckpointError> {
    let len = cursor.u32().map_err(|_| CheckpointError::Invalid("ledger length"))? as usize;
    if len > MAX_CHECKPOINT_BYTES { return Err(CheckpointError::Invalid("ledger too large")); }
    let bytes = cursor.take(len).map_err(|_| CheckpointError::Invalid("ledger truncated"))?;
    Ledger::from_snapshot_bytes(bytes).map_err(CheckpointError::Ledger)
}
