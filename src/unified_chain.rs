//! Reversible *combined* registry and token-ledger projection.
//!
//! All on-chain proof obligations live in `TrustedProtocolAdapter`. There is no
//! Bitcoin/Litecoin/Dogecoin implementation in this crate; test adapters are
//! artificial. This is not an authenticated consensus implementation.
use crate::chain::{
    BlockHeader, BlockId, CandidateBlock, ChainId, Tip, MAX_REORG_HISTORY,
    MAX_TRANSACTIONS_PER_BLOCK, MAX_TRANSACTION_BYTES,
};
use crate::codec::{verify_crc, with_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::execution::{
    ExecutionError, ProtocolOperation, ProtocolSnapshotError, ProtocolState, UnverifiedTx,
};
use crate::ledger::{AccountId, TxId};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 5] = b"TLUC\x01";

/// Output of an independently verifying chain adapter. The adapter is the
/// trusted computing base and MUST derive every field from authenticated
/// chain data rather than treating caller-supplied actor or txid as proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedProtocolOperation {
    pub txid: TxId,
    pub actor: AccountId,
    pub operation: ProtocolOperation,
}

/// `None` denotes an authenticated ordinary L1 transaction that carries no
/// TradeLayer protocol operation; the block still advances the chain tip.
///
/// A real implementation must verify PoW/PoS chain rules, the exact block
/// membership and ordering, UTXO spend authorization, and operation parsing.
/// Such an implementation is deliberately ABSENT from this crate.
pub trait TrustedProtocolAdapter {
    fn verify_block(&self, block: &CandidateBlock) -> Result<(), String>;
    fn authenticate_and_decode(
        &self,
        chain: ChainId,
        header: &BlockHeader,
        raw_transaction: &[u8],
    ) -> Result<Option<AuthenticatedProtocolOperation>, String>;
}

#[derive(Debug)]
pub enum UnifiedChainError {
    InvalidHistoryLimit,
    WrongChain,
    NonSequentialHeight,
    ParentMismatch,
    DuplicateBlockId,
    TransactionLimit,
    TransactionSize,
    Authentication(String),
    Execution(ExecutionError),
    EmptyRollbackHistory,
    WrongRollbackTip,
    Checkpoint(UnifiedCheckpointError),
}
impl core::fmt::Display for UnifiedChainError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for UnifiedChainError {}
impl From<ExecutionError> for UnifiedChainError {
    fn from(e: ExecutionError) -> Self {
        Self::Execution(e)
    }
}
impl From<UnifiedCheckpointError> for UnifiedChainError {
    fn from(e: UnifiedCheckpointError) -> Self {
        Self::Checkpoint(e)
    }
}

#[derive(Debug)]
pub enum UnifiedCheckpointError {
    Invalid(&'static str),
    Protocol(ProtocolSnapshotError),
    Io(std::io::Error),
}
impl core::fmt::Display for UnifiedCheckpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for UnifiedCheckpointError {}
impl From<ProtocolSnapshotError> for UnifiedCheckpointError {
    fn from(e: ProtocolSnapshotError) -> Self {
        Self::Protocol(e)
    }
}
impl From<std::io::Error> for UnifiedCheckpointError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
fn invalid(msg: &'static str) -> UnifiedCheckpointError {
    UnifiedCheckpointError::Invalid(msg)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Prior {
    tip: Tip,
    state: ProtocolState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedChainState {
    chain: ChainId,
    admin: AccountId,
    anchor: Tip,
    tip: Tip,
    state: ProtocolState,
    history: VecDeque<Prior>,
    history_limit: usize,
}

impl UnifiedChainState {
    /// The chain and anchor are externally configured, NOT proved here.
    pub fn new_unverified(
        chain: ChainId,
        anchor: Tip,
        admin: AccountId,
        history_limit: usize,
    ) -> Result<Self, UnifiedChainError> {
        if !(1..=MAX_REORG_HISTORY).contains(&history_limit) {
            return Err(UnifiedChainError::InvalidHistoryLimit);
        }
        Ok(Self {
            chain,
            admin: admin.clone(),
            anchor,
            tip: anchor,
            state: ProtocolState::new_unverified(chain, admin),
            history: VecDeque::new(),
            history_limit,
        })
    }
    pub fn state(&self) -> &ProtocolState {
        &self.state
    }
    pub fn tip(&self) -> Tip {
        self.tip
    }
    pub fn chain_id(&self) -> ChainId {
        self.chain
    }
    pub fn rollback_depth(&self) -> usize {
        self.history.len()
    }

    /// The adapter supplies authenticated operations. It is a trusted external
    /// implementation, not something checked by this Rust module on its own.
    pub fn apply_block<A: TrustedProtocolAdapter>(
        &mut self,
        block: &CandidateBlock,
        adapter: &A,
    ) -> Result<(), UnifiedChainError> {
        if block.chain != self.chain {
            return Err(UnifiedChainError::WrongChain);
        }
        if self.tip.height.checked_add(1) != Some(block.header.height) {
            return Err(UnifiedChainError::NonSequentialHeight);
        }
        if block.header.parent != self.tip.id {
            return Err(UnifiedChainError::ParentMismatch);
        }
        if block.header.id == self.tip.id {
            return Err(UnifiedChainError::DuplicateBlockId);
        }
        if block.raw_transactions.len() > MAX_TRANSACTIONS_PER_BLOCK {
            return Err(UnifiedChainError::TransactionLimit);
        }
        adapter
            .verify_block(block)
            .map_err(UnifiedChainError::Authentication)?;
        let mut transactions = Vec::new();
        for raw in &block.raw_transactions {
            if raw.is_empty() || raw.len() > MAX_TRANSACTION_BYTES {
                return Err(UnifiedChainError::TransactionSize);
            }
            if let Some(decoded) = adapter
                .authenticate_and_decode(self.chain, &block.header, raw)
                .map_err(UnifiedChainError::Authentication)?
            {
                transactions.push(UnverifiedTx {
                    chain: self.chain,
                    txid: decoded.txid,
                    claimed_actor: decoded.actor,
                    claimed_height: block.header.height,
                    operation: decoded.operation,
                });
            }
        }
        let mut staged = self.state.clone();
        if !transactions.is_empty() {
            staged.apply_batch_unverified(transactions)?;
        }
        self.history.push_back(Prior {
            tip: self.tip,
            state: self.state.clone(),
        });
        while self.history.len() > self.history_limit {
            self.history.pop_front();
        }
        self.state = staged;
        self.tip = Tip {
            height: block.header.height,
            id: block.header.id,
        };
        Ok(())
    }

    pub fn rollback_tip(&mut self, expected: Tip) -> Result<(), UnifiedChainError> {
        if expected != self.tip {
            return Err(UnifiedChainError::WrongRollbackTip);
        }
        let prior = self
            .history
            .pop_back()
            .ok_or(UnifiedChainError::EmptyRollbackHistory)?;
        self.tip = prior.tip;
        self.state = prior.state;
        Ok(())
    }

    pub fn apply_block_and_checkpoint<A: TrustedProtocolAdapter>(
        &mut self,
        block: &CandidateBlock,
        adapter: &A,
        path: &Path,
    ) -> Result<(), UnifiedChainError> {
        let mut staged = self.clone();
        staged.apply_block(block, adapter)?;
        staged.write_new_checkpoint(path)?;
        *self = staged;
        Ok(())
    }

    /// Immutable checkpoint, including complete registry state and all replay
    /// markers across each retained pre-block snapshot.
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, UnifiedCheckpointError> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&self.chain.0);
        let admin = self.admin.as_str().as_bytes();
        out.push(u8::try_from(admin.len()).map_err(|_| invalid("admin too long"))?);
        out.extend_from_slice(admin);
        push_tip(&mut out, self.anchor);
        push_tip(&mut out, self.tip);
        out.push(
            u8::try_from(self.history_limit).map_err(|_| invalid("history policy too large"))?,
        );
        out.push(u8::try_from(self.history.len()).map_err(|_| invalid("history too long"))?);
        for prior in &self.history {
            push_tip(&mut out, prior.tip);
            push_protocol(&mut out, &prior.state)?;
        }
        push_protocol(&mut out, &self.state)?;
        with_crc(out).map_err(|_| invalid("checkpoint limit exceeded"))
    }

    pub fn from_checkpoint_bytes(
        bytes: &[u8],
        expected_chain: ChainId,
        expected_anchor: Tip,
        expected_admin: &AccountId,
        expected_history_limit: usize,
    ) -> Result<Self, UnifiedCheckpointError> {
        if !(1..=MAX_REORG_HISTORY).contains(&expected_history_limit) {
            return Err(invalid("invalid rollback policy"));
        }
        let body = verify_crc(bytes).map_err(|_| invalid("checkpoint CRC or size"))?;
        let mut r = Cursor::new(body);
        if r.fixed::<5>().map_err(|_| invalid("truncated magic"))? != *MAGIC {
            return Err(invalid("checkpoint magic/version"));
        }
        let chain = ChainId(r.fixed::<32>().map_err(|_| invalid("truncated chain id"))?);
        if chain != expected_chain {
            return Err(invalid("wrong checkpoint chain"));
        }
        let admin_size = r.u8().map_err(|_| invalid("missing admin size"))? as usize;
        if admin_size == 0 || admin_size > crate::ledger::MAX_ACCOUNT_BYTES {
            return Err(invalid("invalid admin size"));
        }
        let admin_bytes = r.take(admin_size).map_err(|_| invalid("truncated admin"))?;
        let admin_text = core::str::from_utf8(admin_bytes).map_err(|_| invalid("admin utf8"))?;
        let admin = AccountId::try_from(admin_text).map_err(|_| invalid("invalid admin"))?;
        if &admin != expected_admin {
            return Err(invalid("wrong checkpoint admin"));
        }
        let anchor = read_tip(&mut r)?;
        let tip = read_tip(&mut r)?;
        if anchor != expected_anchor {
            return Err(invalid("wrong checkpoint anchor"));
        }
        let history_limit = r.u8().map_err(|_| invalid("missing history limit"))? as usize;
        let count = r.u8().map_err(|_| invalid("missing history count"))? as usize;
        if history_limit != expected_history_limit || count > history_limit {
            return Err(invalid("invalid history length/policy"));
        }
        let mut history = VecDeque::new();
        for _ in 0..count {
            let previous_tip = read_tip(&mut r)?;
            if previous_tip.height < anchor.height
                || history.back().is_some_and(|old: &Prior| {
                    old.tip.height.checked_add(1) != Some(previous_tip.height)
                })
            {
                return Err(invalid("discontinuous history"));
            }
            let prior_state = read_protocol(&mut r, expected_chain, expected_admin)?;
            if prior_state
                .last_height()
                .is_some_and(|height| height > previous_tip.height)
            {
                return Err(invalid("state ahead of history tip"));
            }
            history.push_back(Prior {
                tip: previous_tip,
                state: prior_state,
            });
        }
        let state = read_protocol(&mut r, expected_chain, expected_admin)?;
        r.end().map_err(|_| invalid("trailing checkpoint data"))?;
        if tip.height < anchor.height
            || (tip.height == anchor.height && (tip.id != anchor.id || count != 0))
            || (tip.height > anchor.height && count == 0)
            || history.back().is_some_and(|old: &Prior| {
                old.tip.height.checked_add(1) != Some(tip.height) || old.tip.id == tip.id
            })
            || state
                .last_height()
                .is_some_and(|height| height > tip.height)
        {
            return Err(invalid("invalid checkpoint tip"));
        }
        let decoded = Self {
            chain,
            admin,
            anchor,
            tip,
            state,
            history,
            history_limit,
        };
        if decoded.checkpoint_bytes()?.as_slice() != bytes {
            return Err(invalid("noncanonical checkpoint"));
        }
        Ok(decoded)
    }

    /// Uses create_new and fsync for immutable files. A directory fsync and
    /// authenticated root commitment are left to a production storage backend.
    pub fn write_new_checkpoint(&self, path: &Path) -> Result<(), UnifiedCheckpointError> {
        let bytes = self.checkpoint_bytes()?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(UnifiedCheckpointError::Io(error));
        }
        Ok(())
    }
    pub fn load_checkpoint(
        path: &Path,
        expected_chain: ChainId,
        expected_anchor: Tip,
        expected_admin: &AccountId,
        expected_history_limit: usize,
    ) -> Result<Self, UnifiedCheckpointError> {
        let f = File::open(path)?;
        let len = usize::try_from(f.metadata()?.len()).map_err(|_| invalid("checkpoint length"))?;
        if len > MAX_CHECKPOINT_BYTES {
            return Err(invalid("checkpoint file too large"));
        }
        let mut bytes = Vec::with_capacity(len);
        f.take((MAX_CHECKPOINT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        Self::from_checkpoint_bytes(
            &bytes,
            expected_chain,
            expected_anchor,
            expected_admin,
            expected_history_limit,
        )
    }
}

fn push_tip(out: &mut Vec<u8>, tip: Tip) {
    out.extend_from_slice(&tip.height.to_be_bytes());
    out.extend_from_slice(&tip.id.0);
}
fn read_tip(r: &mut Cursor<'_>) -> Result<Tip, UnifiedCheckpointError> {
    let height = r.u64().map_err(|_| invalid("tip height truncated"))?;
    let id = BlockId(r.fixed::<32>().map_err(|_| invalid("tip hash truncated"))?);
    Ok(Tip { height, id })
}
fn push_protocol(
    out: &mut Vec<u8>,
    protocol: &ProtocolState,
) -> Result<(), UnifiedCheckpointError> {
    let bytes = protocol.snapshot_bytes()?;
    let len = u32::try_from(bytes.len()).map_err(|_| invalid("protocol too large"))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&bytes);
    Ok(())
}
fn read_protocol(
    r: &mut Cursor<'_>,
    chain: ChainId,
    admin: &AccountId,
) -> Result<ProtocolState, UnifiedCheckpointError> {
    let length = r.u32().map_err(|_| invalid("protocol length truncated"))? as usize;
    if length > MAX_CHECKPOINT_BYTES {
        return Err(invalid("protocol too large"));
    }
    let bytes = r.take(length).map_err(|_| invalid("protocol truncated"))?;
    ProtocolState::from_snapshot_bytes(bytes, chain, admin)
        .map_err(UnifiedCheckpointError::Protocol)
}
