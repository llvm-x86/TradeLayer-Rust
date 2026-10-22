//! Stage 20: replay of a typed, **unverified** spot-only block stream with
//! atomic stage/commit, bounded rollback, and complete TLSP+ledger checkpoints.
//!
//! This deliberately does not join the canonical UnifiedChainState ledger:
//! that requires one shared transaction replay domain, native/vesting policy,
//! actual Litecoin chain authentication and a versioned TLUC migration.
//! Caller-controlled block IDs, actor accounts and txids are untrusted here.

use crate::chain::{
    BlockHeader, BlockId, ChainId, Tip, MAX_REORG_HISTORY, MAX_TRANSACTIONS_PER_BLOCK,
};
use crate::codec::{verify_crc, with_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::ledger::PropertyId;
use crate::spot::{
    SpotError, SpotExchange, SpotSnapshotError, UnverifiedSpotCancellation,
    UnverifiedSpotPlacement,
};
use std::collections::{BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 5] = b"TLSC\x01";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnverifiedSpotOperation {
    PlaceType5(UnverifiedSpotPlacement),
    CancelType6(UnverifiedSpotCancellation),
    /// Explicit fail-closed boundary for the unverified LTC-for-token path.
    Type3UtxoTrade,
}

/// Operations are decoded assertions, not raw signed L1 transactions.
/// Applications must never use this prototype to authorize a payout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedSpotBlock {
    pub chain: ChainId,
    pub header: BlockHeader,
    pub operations: Vec<UnverifiedSpotOperation>,
}

#[derive(Debug)]
pub enum SpotChainError {
    InvalidHistoryLimit,
    WrongChain,
    NonSequentialHeight,
    ParentMismatch,
    DuplicateBlockId,
    TransactionLimit,
    OperationHeightMismatch,
    Spot(SpotError),
    WrongRollbackTip,
    EmptyRollbackHistory,
    Checkpoint(SpotChainCheckpointError),
}
impl core::fmt::Display for SpotChainError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SpotChainError {}
impl From<SpotError> for SpotChainError {
    fn from(e: SpotError) -> Self {
        Self::Spot(e)
    }
}
impl From<SpotChainCheckpointError> for SpotChainError {
    fn from(e: SpotChainCheckpointError) -> Self {
        Self::Checkpoint(e)
    }
}

#[derive(Debug)]
pub enum SpotChainCheckpointError {
    Invalid(&'static str),
    Spot(SpotSnapshotError),
    Io(std::io::Error),
}
impl core::fmt::Display for SpotChainCheckpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SpotChainCheckpointError {}
impl From<SpotSnapshotError> for SpotChainCheckpointError {
    fn from(e: SpotSnapshotError) -> Self {
        Self::Spot(e)
    }
}
impl From<std::io::Error> for SpotChainCheckpointError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
fn bad(reason: &'static str) -> SpotChainCheckpointError {
    SpotChainCheckpointError::Invalid(reason)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PriorSpot {
    tip: Tip,
    state: SpotExchange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotChainState {
    chain: ChainId,
    anchor: Tip,
    tip: Tip,
    state: SpotExchange,
    history: VecDeque<PriorSpot>,
    history_limit: usize,
}

impl SpotChainState {
    /// Starts at an externally asserted chain anchor with prefunded *offline*
    /// test properties. State must be pristine; history begins only at anchor.
    pub fn new_unverified(
        chain: ChainId,
        anchor: Tip,
        state: SpotExchange,
        history_limit: usize,
    ) -> Result<Self, SpotChainError> {
        if !(1..=MAX_REORG_HISTORY).contains(&history_limit) {
            return Err(SpotChainError::InvalidHistoryLimit);
        }
        if !state.is_pristine_unverified() {
            return Err(SpotChainError::Checkpoint(
                bad("initial spot state must be pristine"),
            ));
        }
        state.snapshot_bytes().map_err(SpotChainCheckpointError::Spot)?;
        Ok(Self {
            chain,
            anchor,
            tip: anchor,
            state,
            history: VecDeque::new(),
            history_limit,
        })
    }
    pub fn tip(&self) -> Tip {
        self.tip
    }
    pub fn chain_id(&self) -> ChainId {
        self.chain
    }
    pub fn state(&self) -> &SpotExchange {
        &self.state
    }
    pub fn rollback_depth(&self) -> usize {
        self.history.len()
    }

    /// All mutations stage on a clone. Type 6 cancellation sees previously
    /// finalized orders, while type-5 orders enter the block-end queue.
    /// Empty blocks still advance finalization and snapshot history.
    pub fn apply_unverified_block(
        &mut self,
        block: &UnverifiedSpotBlock,
    ) -> Result<(), SpotChainError> {
        if block.chain != self.chain {
            return Err(SpotChainError::WrongChain);
        }
        if self.tip.height.checked_add(1) != Some(block.header.height) {
            return Err(SpotChainError::NonSequentialHeight);
        }
        if block.header.parent != self.tip.id {
            return Err(SpotChainError::ParentMismatch);
        }
        if block.header.id == self.tip.id {
            return Err(SpotChainError::DuplicateBlockId);
        }
        if block.operations.len() > MAX_TRANSACTIONS_PER_BLOCK {
            return Err(SpotChainError::TransactionLimit);
        }
        let mut candidate = self.state.clone();
        for operation in &block.operations {
            match operation {
                UnverifiedSpotOperation::PlaceType5(placement) => {
                    if placement.height != block.header.height {
                        return Err(SpotChainError::OperationHeightMismatch);
                    }
                    candidate.enqueue_type5_unverified(placement.clone())?;
                }
                UnverifiedSpotOperation::CancelType6(cancel) => {
                    if cancel.height != block.header.height {
                        return Err(SpotChainError::OperationHeightMismatch);
                    }
                    candidate.cancel_type6_unverified(cancel.clone())?;
                }
                UnverifiedSpotOperation::Type3UtxoTrade => {
                    return Err(SpotChainError::Spot(
                        SpotError::AuthenticatedUtxoProofRequired,
                    ));
                }
            }
        }
        candidate.process_through_unverified(block.header.height)?;
        self.history.push_back(PriorSpot {
            tip: self.tip,
            state: self.state.clone(),
        });
        while self.history.len() > self.history_limit {
            self.history.pop_front();
        }
        self.state = candidate;
        self.tip = Tip {
            height: block.header.height,
            id: block.header.id,
        };
        Ok(())
    }

    /// Restores the complete ledger, open/queued orders, fills and replay
    /// markers from the last observed block. Alternate fork IDs are caller
    /// claims; this routine does not choose the authentic Litecoin fork.
    pub fn rollback_tip(&mut self, expected: Tip) -> Result<(), SpotChainError> {
        if expected != self.tip {
            return Err(SpotChainError::WrongRollbackTip);
        }
        let prior = self
            .history
            .pop_back()
            .ok_or(SpotChainError::EmptyRollbackHistory)?;
        self.tip = prior.tip;
        self.state = prior.state;
        Ok(())
    }

    pub fn apply_unverified_block_and_checkpoint(
        &mut self,
        block: &UnverifiedSpotBlock,
        path: &Path,
    ) -> Result<(), SpotChainError> {
        let mut candidate = self.clone();
        candidate.apply_unverified_block(block)?;
        candidate.write_new_checkpoint(path)?;
        *self = candidate;
        Ok(())
    }

    /// TLSC v1 includes the full TLSP state for *each* rollback snapshot.
    /// The result is bounded by the common 64 MiB checkpoint size ceiling.
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, SpotChainCheckpointError> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&self.chain.0);
        put_tip(&mut out, self.anchor);
        put_tip(&mut out, self.tip);
        out.push(u8::try_from(self.history_limit).map_err(|_| bad("history policy"))?);
        out.push(u8::try_from(self.history.len()).map_err(|_| bad("history count"))?);
        for prior in &self.history {
            put_tip(&mut out, prior.tip);
            put_state(&mut out, &prior.state)?;
        }
        put_state(&mut out, &self.state)?;
        with_crc(out).map_err(|_| bad("spot chain checkpoint too large"))
    }

    pub fn from_checkpoint_bytes(
        bytes: &[u8],
        expected_chain: ChainId,
        expected_anchor: Tip,
        expected_history_limit: usize,
        expected_permitted: &BTreeSet<PropertyId>,
    ) -> Result<Self, SpotChainCheckpointError> {
        if !(1..=MAX_REORG_HISTORY).contains(&expected_history_limit) {
            return Err(bad("invalid history policy"));
        }
        let payload = verify_crc(bytes).map_err(|_| bad("spot chain CRC/size"))?;
        let mut r = Cursor::new(payload);
        if r.fixed::<5>().map_err(|_| bad("magic truncated"))? != *MAGIC {
            return Err(bad("spot chain version"));
        }
        let chain = ChainId(r.fixed::<32>().map_err(|_| bad("chain truncated"))?);
        if chain != expected_chain {
            return Err(bad("wrong chain"));
        }
        let anchor = read_tip(&mut r)?;
        let tip = read_tip(&mut r)?;
        if anchor != expected_anchor {
            return Err(bad("wrong anchor"));
        }
        let history_limit = r.u8().map_err(|_| bad("policy truncated"))? as usize;
        let count = r.u8().map_err(|_| bad("history truncated"))? as usize;
        if history_limit != expected_history_limit || count > history_limit {
            return Err(bad("invalid history limit/count"));
        }
        let mut history: VecDeque<PriorSpot> = VecDeque::new();
        for _ in 0..count {
            let prev_tip = read_tip(&mut r)?;
            if prev_tip.height < anchor.height
                || history.back().is_some_and(|old| {
                    old.tip.height.checked_add(1) != Some(prev_tip.height)
                })
            {
                return Err(bad("noncontiguous rollback history"));
            }
            let state = read_state(&mut r, expected_permitted)?;
            if state.processed_through()
                != if prev_tip == anchor {
                    None
                } else {
                    Some(prev_tip.height)
                }
            {
                return Err(bad("rollback height mismatch"));
            }
            history.push_back(PriorSpot {
                tip: prev_tip,
                state,
            });
        }
        let state = read_state(&mut r, expected_permitted)?;
        r.end().map_err(|_| bad("trailing checkpoint bytes"))?;
        if tip.height < anchor.height
            || (tip.height == anchor.height && (tip != anchor || count != 0))
            || (tip.height > anchor.height && count == 0)
            || history.back().is_some_and(|old| {
                old.tip.height.checked_add(1) != Some(tip.height)
                    || old.tip.id == tip.id
            })
            || state.processed_through()
                != if tip == anchor {
                    None
                } else {
                    Some(tip.height)
                }
            || (tip == anchor && !state.is_pristine_unverified())
        {
            return Err(bad("tip/state continuity mismatch"));
        }
        let result = Self {
            chain,
            anchor,
            tip,
            state,
            history,
            history_limit,
        };
        if result.checkpoint_bytes()?.as_slice() != bytes {
            return Err(bad("noncanonical chain checkpoint"));
        }
        Ok(result)
    }

    /// create_new prevents replacement of a previous checkpoint on collision.
    /// File fsync is used; directory fsync/authenticated roots are future work.
    pub fn write_new_checkpoint(
        &self,
        path: &Path,
    ) -> Result<(), SpotChainCheckpointError> {
        let bytes = self.checkpoint_bytes()?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        if let Err(e) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(SpotChainCheckpointError::Io(e));
        }
        Ok(())
    }

    pub fn load_checkpoint(
        path: &Path,
        expected_chain: ChainId,
        expected_anchor: Tip,
        expected_history_limit: usize,
        expected_permitted: &BTreeSet<PropertyId>,
    ) -> Result<Self, SpotChainCheckpointError> {
        let file = File::open(path)?;
        let length = usize::try_from(file.metadata()?.len())
            .map_err(|_| bad("checkpoint file size"))?;
        if length > MAX_CHECKPOINT_BYTES {
            return Err(bad("checkpoint file too large"));
        }
        let mut bytes = Vec::with_capacity(length);
        file.take((MAX_CHECKPOINT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        Self::from_checkpoint_bytes(
            &bytes,
            expected_chain,
            expected_anchor,
            expected_history_limit,
            expected_permitted,
        )
    }
}

fn put_tip(out: &mut Vec<u8>, tip: Tip) {
    out.extend_from_slice(&tip.height.to_be_bytes());
    out.extend_from_slice(&tip.id.0);
}
fn read_tip(r: &mut Cursor<'_>) -> Result<Tip, SpotChainCheckpointError> {
    let height = r.u64().map_err(|_| bad("tip height"))?;
    let id = BlockId(r.fixed::<32>().map_err(|_| bad("tip hash"))?);
    Ok(Tip { height, id })
}
fn put_state(out: &mut Vec<u8>, state: &SpotExchange) -> Result<(), SpotChainCheckpointError> {
    let snapshot = state.snapshot_bytes()?;
    out.extend_from_slice(
        &u32::try_from(snapshot.len())
            .map_err(|_| bad("embedded spot snapshot length"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(&snapshot);
    Ok(())
}
fn read_state(
    r: &mut Cursor<'_>,
    expected_permitted: &BTreeSet<PropertyId>,
) -> Result<SpotExchange, SpotChainCheckpointError> {
    let length = r.u32().map_err(|_| bad("embedded length"))? as usize;
    if length > MAX_CHECKPOINT_BYTES {
        return Err(bad("embedded state too large"));
    }
    let bytes = r.take(length).map_err(|_| bad("embedded state truncated"))?;
    SpotExchange::from_snapshot_bytes(bytes, expected_permitted)
        .map_err(SpotChainCheckpointError::Spot)
}
