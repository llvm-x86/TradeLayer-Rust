//! Stage 25D: combined spot+clearing *unverified* block journal.
//!
//! TLCC v1 stores every retained prior TLCL snapshot and the current TLCL
//! state. Chain headers, txids, actors, mark prices and economic operations
//! are all CALLER ASSERTIONS. This is NOT a chain validator or authorization
//! for payouts. Never connect to an actual node/wallet without a separately
//! reviewed authenticated adapter and ratified economic consensus policy.

use crate::chain::{BlockHeader, Tip, MAX_REORG_HISTORY};
use crate::clearing_sandbox::{
    ClearingError, ClearingSandbox, ClearingSnapshotError, CollateralPolicy, MarkTerms,
    MatchedContractTrade,
};
use crate::codec::{verify_crc, with_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::ledger::{AccountId, Operation, PropertyId, TxId};
use crate::network_profile::TradeLayerMainnet;
use crate::spot::UnverifiedSpotPlacement;
use std::collections::{BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const CHAIN_MAGIC: &[u8; 5] = b"TLCC\x01";
pub const MAX_CLEARING_BLOCK_EVENTS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnverifiedClearingEvent {
    Ordinary { txid: TxId, operation: Operation },
    SpotPlace(UnverifiedSpotPlacement),
    MarginDeposit {
        txid: TxId,
        owner: AccountId,
        atoms: u128,
    },
    MarginWithdraw {
        txid: TxId,
        owner: AccountId,
        atoms: u128,
    },
    MatchedTrades {
        txid: TxId,
        trades: Vec<MatchedContractTrade>,
    },
    Funding {
        txid: TxId,
        notional_atoms: u64,
        rate_bps_e4: i64,
    },
    /// Must appear last and may only use the block's declared height.
    /// No spontaneous funding/mark schedules are inferred.
    Mark { txid: TxId, new_price_e8: u64 },
    /// Unsupported economic settlement MUST remain default deny.
    UnsupportedSettlement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedClearingBlock {
    pub network: TradeLayerMainnet,
    pub header: BlockHeader,
    pub events: Vec<UnverifiedClearingEvent>,
}

#[derive(Debug)]
pub enum ClearingChainError {
    WrongNetwork,
    InvalidHeight,
    ParentMismatch,
    DuplicateBlockId,
    InvalidHistoryLimit,
    TooManyEvents,
    OperationHeightMismatch,
    MarkMustEndBlock,
    UnsupportedEconomicSettlement,
    Clearing(ClearingError),
    WrongRollbackTip,
    EmptyRollbackHistory,
    Checkpoint(ClearingChainCheckpointError),
}

impl core::fmt::Display for ClearingChainError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ClearingChainError {}
impl From<ClearingError> for ClearingChainError {
    fn from(error: ClearingError) -> Self {
        Self::Clearing(error)
    }
}
impl From<ClearingChainCheckpointError> for ClearingChainError {
    fn from(error: ClearingChainCheckpointError) -> Self {
        Self::Checkpoint(error)
    }
}

#[derive(Debug)]
pub enum ClearingChainCheckpointError {
    Invalid(&'static str),
    State(ClearingSnapshotError),
    Io(std::io::Error),
}

impl core::fmt::Display for ClearingChainCheckpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ClearingChainCheckpointError {}
impl From<ClearingSnapshotError> for ClearingChainCheckpointError {
    fn from(error: ClearingSnapshotError) -> Self {
        Self::State(error)
    }
}
impl From<std::io::Error> for ClearingChainCheckpointError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
fn cc_bad(text: &'static str) -> ClearingChainCheckpointError {
    ClearingChainCheckpointError::Invalid(text)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PriorClearing {
    tip: Tip,
    state: ClearingSandbox,
}

/// One owned combined state plus a bounded journal of complete pre-block
/// states. A block is committed only after every stage, spot close, and mark
/// passes, then an immutable checkpoint write if requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearingChainState {
    network: TradeLayerMainnet,
    contract: u32,
    collateral: PropertyId,
    anchor: Tip,
    tip: Tip,
    state: ClearingSandbox,
    history: VecDeque<PriorClearing>,
    history_limit: usize,
}

impl ClearingChainState {
    pub fn new_unverified(
        network: TradeLayerMainnet,
        anchor: Tip,
        state: ClearingSandbox,
        history_limit: usize,
    ) -> Result<Self, ClearingChainError> {
        if !(1..=MAX_REORG_HISTORY).contains(&history_limit) {
            return Err(ClearingChainError::InvalidHistoryLimit);
        }
        network
            .check_genesis_if_applicable(anchor)
            .map_err(|_| ClearingChainError::WrongNetwork)?;
        if state.network() != network || !state_height_compatible(&state, anchor, anchor) {
            return Err(ClearingChainError::WrongNetwork);
        }
        state.verify()?;
        // Verify this versioned format can preserve all state before putting
        // the source into a reversible history. CRC never authenticates it.
        state.snapshot_bytes()?;
        Ok(Self {
            network,
            contract: state.contract(),
            collateral: state.collateral(),
            anchor,
            tip: anchor,
            state,
            history: VecDeque::new(),
            history_limit,
        })
    }

    pub const fn network(&self) -> TradeLayerMainnet {
        self.network
    }
    pub const fn tip(&self) -> Tip {
        self.tip
    }
    pub const fn anchor(&self) -> Tip {
        self.anchor
    }
    pub fn state(&self) -> &ClearingSandbox {
        &self.state
    }
    pub fn rollback_depth(&self) -> usize {
        self.history.len()
    }

    /// Strict synthetic event ordering. The SpotExchange finalizes exactly
    /// once at the block height, BEFORE the optional final mark. Even empty
    /// blocks advance spot finalization. No late mark may skip due spot work.
    pub fn apply_unverified_block(
        &mut self,
        block: &UnverifiedClearingBlock,
    ) -> Result<(), ClearingChainError> {
        if block.network != self.network {
            return Err(ClearingChainError::WrongNetwork);
        }
        if self.tip.height.checked_add(1) != Some(block.header.height) {
            return Err(ClearingChainError::InvalidHeight);
        }
        if self.tip.id != block.header.parent {
            return Err(ClearingChainError::ParentMismatch);
        }
        if self.tip.id == block.header.id
            || self.history.iter().any(|prior| prior.tip.id == block.header.id)
        {
            return Err(ClearingChainError::DuplicateBlockId);
        }
        if block.events.len() > MAX_CLEARING_BLOCK_EVENTS {
            return Err(ClearingChainError::TooManyEvents);
        }

        let mut staged = self.state.clone();
        let mut final_mark = None;
        for (i, event) in block.events.iter().enumerate() {
            match event {
                UnverifiedClearingEvent::Ordinary { txid, operation } => {
                    staged.apply_ordinary_operation_unverified(*txid, operation.clone())?;
                }
                UnverifiedClearingEvent::SpotPlace(entry) => {
                    if entry.height != block.header.height {
                        return Err(ClearingChainError::OperationHeightMismatch);
                    }
                    staged.enqueue_spot_order_unverified(entry.clone())?;
                }
                UnverifiedClearingEvent::MarginDeposit { txid, owner, atoms } => {
                    staged.deposit_margin_unverified(*txid, owner.clone(), *atoms)?;
                }
                UnverifiedClearingEvent::MarginWithdraw { txid, owner, atoms } => {
                    staged.withdraw_margin_unverified(*txid, owner.clone(), *atoms)?;
                }
                UnverifiedClearingEvent::MatchedTrades { txid, trades } => {
                    staged.apply_matched_trades_unverified(*txid, trades)?;
                }
                UnverifiedClearingEvent::Funding {
                    txid,
                    notional_atoms,
                    rate_bps_e4,
                } => {
                    staged.settle_funding_unverified(*txid, *notional_atoms, *rate_bps_e4)?;
                }
                UnverifiedClearingEvent::Mark { txid, new_price_e8 } => {
                    if final_mark.is_some() || i + 1 != block.events.len() {
                        return Err(ClearingChainError::MarkMustEndBlock);
                    }
                    final_mark = Some((*txid, *new_price_e8));
                }
                UnverifiedClearingEvent::UnsupportedSettlement => {
                    return Err(ClearingChainError::UnsupportedEconomicSettlement);
                }
            }
        }
        staged.process_spot_block_unverified(block.header.height)?;
        if let Some((txid, price)) = final_mark {
            staged.settle_mark_to_market_unverified(txid, block.header.height, price)?;
        }
        staged.verify()?;
        self.history.push_back(PriorClearing {
            tip: self.tip,
            state: self.state.clone(),
        });
        while self.history.len() > self.history_limit {
            self.history.pop_front();
        }
        self.tip = Tip {
            height: block.header.height,
            id: block.header.id,
        };
        self.state = staged;
        Ok(())
    }

    pub fn rollback_tip(&mut self, expected: Tip) -> Result<(), ClearingChainError> {
        if self.tip != expected {
            return Err(ClearingChainError::WrongRollbackTip);
        }
        let prior = self
            .history
            .pop_back()
            .ok_or(ClearingChainError::EmptyRollbackHistory)?;
        self.tip = prior.tip;
        self.state = prior.state;
        Ok(())
    }

    /// An I/O failure, file collision or invalid event cannot mutate state.
    pub fn apply_unverified_block_and_checkpoint(
        &mut self,
        block: &UnverifiedClearingBlock,
        path: &Path,
    ) -> Result<(), ClearingChainError> {
        let mut staged = self.clone();
        staged.apply_unverified_block(block)?;
        staged.write_new_checkpoint(path)?;
        *self = staged;
        Ok(())
    }

    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, ClearingChainCheckpointError> {
        let mut out = CHAIN_MAGIC.to_vec();
        out.push(self.network.tag());
        out.extend_from_slice(&self.network.genesis().0);
        out.extend_from_slice(&self.network.chain_id().0);
        out.extend_from_slice(&self.contract.to_be_bytes());
        out.extend_from_slice(&self.collateral.get().to_be_bytes());
        cc_put_tip(&mut out, self.anchor);
        cc_put_tip(&mut out, self.tip);
        out.push(u8::try_from(self.history_limit).map_err(|_| cc_bad("history limit"))?);
        out.push(u8::try_from(self.history.len()).map_err(|_| cc_bad("history count"))?);
        for prior in &self.history {
            cc_put_tip(&mut out, prior.tip);
            cc_put_state(&mut out, &prior.state)?;
        }
        cc_put_state(&mut out, &self.state)?;
        with_crc(out).map_err(|_| cc_bad("combined checkpoint exceeds size limit"))
    }

    pub fn from_checkpoint_bytes(
        bytes: &[u8],
        expected_network: TradeLayerMainnet,
        expected_anchor: Tip,
        expected_contract: u32,
        expected_collateral: PropertyId,
        expected_permitted: &BTreeSet<PropertyId>,
        expected_collateral_policy: Option<CollateralPolicy>,
        expected_mark_terms: Option<MarkTerms>,
        expected_history_limit: usize,
    ) -> Result<Self, ClearingChainCheckpointError> {
        if !(1..=MAX_REORG_HISTORY).contains(&expected_history_limit) {
            return Err(cc_bad("invalid expected history limit"));
        }
        expected_network
            .check_genesis_if_applicable(expected_anchor)
            .map_err(|_| cc_bad("genesis anchor mismatch"))?;
        let payload = verify_crc(bytes).map_err(|_| cc_bad("checkpoint CRC or size"))?;
        let mut r = Cursor::new(payload);
        if r.fixed::<5>().map_err(|_| cc_bad("checkpoint magic"))? != *CHAIN_MAGIC {
            return Err(cc_bad("checkpoint version"));
        }
        let tag = r.u8().map_err(|_| cc_bad("network tag"))?;
        let genesis = r.fixed::<32>().map_err(|_| cc_bad("network genesis"))?;
        let chain = r.fixed::<32>().map_err(|_| cc_bad("chain ID"))?;
        let contract = r.u32().map_err(|_| cc_bad("contract"))?;
        let collateral = r.u32().map_err(|_| cc_bad("collateral"))?;
        if tag != expected_network.tag()
            || genesis != expected_network.genesis().0
            || chain != expected_network.chain_id().0
            || contract != expected_contract
            || collateral != expected_collateral.get()
        {
            return Err(cc_bad("wrong network contract collateral or genesis"));
        }
        let anchor = cc_read_tip(&mut r)?;
        let tip = cc_read_tip(&mut r)?;
        if anchor != expected_anchor {
            return Err(cc_bad("wrong checkpoint anchor"));
        }
        let limit = r.u8().map_err(|_| cc_bad("history limit missing"))? as usize;
        let count = r.u8().map_err(|_| cc_bad("history count missing"))? as usize;
        if limit != expected_history_limit || count > limit {
            return Err(cc_bad("invalid history policy"));
        }
        let mut history: VecDeque<PriorClearing> = VecDeque::new();
        for _ in 0..count {
            let prev_tip = cc_read_tip(&mut r)?;
            if prev_tip.height < anchor.height
                || history.back().is_some_and(|prev| {
                    prev.tip.height.checked_add(1) != Some(prev_tip.height)
                        || prev.tip.id == prev_tip.id
                })
            {
                return Err(cc_bad("discontinuous combined rollback history"));
            }
            let prior = cc_read_state(
                &mut r,
                expected_network,
                contract,
                expected_collateral,
                expected_permitted,
                expected_collateral_policy,
                expected_mark_terms,
            )?;
            if !state_height_compatible(&prior, prev_tip, anchor) {
                return Err(cc_bad("prior spot/mark height mismatch"));
            }
            history.push_back(PriorClearing {
                tip: prev_tip,
                state: prior,
            });
        }
        let state = cc_read_state(
            &mut r,
            expected_network,
            contract,
            expected_collateral,
            expected_permitted,
            expected_collateral_policy,
            expected_mark_terms,
        )?;
        r.end().map_err(|_| cc_bad("trailing checkpoint bytes"))?;
        if tip.height < anchor.height
            || (tip.height == anchor.height && (tip != anchor || count != 0))
            || (tip.height > anchor.height && count == 0)
            || history.back().is_some_and(|prev| {
                prev.tip.height.checked_add(1) != Some(tip.height) || prev.tip.id == tip.id
            })
            || !state_height_compatible(&state, tip, anchor)
        {
            return Err(cc_bad("combined checkpoint tip/state mismatch"));
        }
        let recovered = Self {
            network: expected_network,
            contract,
            collateral: expected_collateral,
            anchor,
            tip,
            state,
            history,
            history_limit: limit,
        };
        if recovered.checkpoint_bytes()?.as_slice() != bytes {
            return Err(cc_bad("noncanonical combined checkpoint"));
        }
        Ok(recovered)
    }

    /// The path is exclusively created once. Never overwrite an existing
    /// checkpoint. File sync alone does not guarantee parent-directory sync
    /// after sudden power loss; independently manage the release storage TCB.
    pub fn write_new_checkpoint(
        &self,
        path: &Path,
    ) -> Result<(), ClearingChainCheckpointError> {
        let bytes = self.checkpoint_bytes()?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(ClearingChainCheckpointError::Io(error));
        }
        Ok(())
    }

    pub fn load_checkpoint(
        path: &Path,
        network: TradeLayerMainnet,
        anchor: Tip,
        contract: u32,
        collateral: PropertyId,
        permitted: &BTreeSet<PropertyId>,
        collateral_policy: Option<CollateralPolicy>,
        mark_terms: Option<MarkTerms>,
        history_limit: usize,
    ) -> Result<Self, ClearingChainCheckpointError> {
        let file = File::open(path)?;
        let length = usize::try_from(file.metadata()?.len())
            .map_err(|_| cc_bad("checkpoint file size"))?;
        if length > MAX_CHECKPOINT_BYTES {
            return Err(cc_bad("checkpoint excessive size"));
        }
        let mut bytes = Vec::with_capacity(length);
        file.take((MAX_CHECKPOINT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(cc_bad("checkpoint read excessive size"));
        }
        Self::from_checkpoint_bytes(
            &bytes,
            network,
            anchor,
            contract,
            collateral,
            permitted,
            collateral_policy,
            mark_terms,
            history_limit,
        )
    }
}

/// Spot is finalized once per processed block. At a caller-supplied anchor,
/// an already finalized spot state may be imported without trusting that
/// anchor as independently consensus-verified. Marks cannot be in the future.
fn state_height_compatible(state: &ClearingSandbox, tip: Tip, anchor: Tip) -> bool {
    let spot_height = state.spot().processed_through();
    let spot_matches = if tip == anchor {
        spot_height.is_none() || spot_height == Some(tip.height)
    } else {
        spot_height == Some(tip.height)
    };
    spot_matches && state.mark_policy().is_none_or(|policy| policy.height <= tip.height)
}

fn cc_put_tip(out: &mut Vec<u8>, tip: Tip) {
    out.extend_from_slice(&tip.height.to_be_bytes());
    out.extend_from_slice(&tip.id.0);
}
fn cc_read_tip(r: &mut Cursor<'_>) -> Result<Tip, ClearingChainCheckpointError> {
    Ok(Tip {
        height: r.u64().map_err(|_| cc_bad("tip height"))?,
        id: crate::chain::BlockId(r.fixed::<32>().map_err(|_| cc_bad("tip ID"))?),
    })
}
fn cc_put_state(
    out: &mut Vec<u8>,
    state: &ClearingSandbox,
) -> Result<(), ClearingChainCheckpointError> {
    let bytes = state.snapshot_bytes()?;
    out.extend_from_slice(
        &u32::try_from(bytes.len())
            .map_err(|_| cc_bad("embedded clearing state length"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(&bytes);
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn cc_read_state(
    r: &mut Cursor<'_>,
    network: TradeLayerMainnet,
    contract: u32,
    collateral: PropertyId,
    permitted: &BTreeSet<PropertyId>,
    collateral_policy: Option<CollateralPolicy>,
    mark_terms: Option<MarkTerms>,
) -> Result<ClearingSandbox, ClearingChainCheckpointError> {
    let length = r.u32().map_err(|_| cc_bad("embedded state length"))? as usize;
    if length == 0 || length > MAX_CHECKPOINT_BYTES {
        return Err(cc_bad("embedded clearing state excessive size"));
    }
    let bytes = r.take(length).map_err(|_| cc_bad("embedded clearing state truncated"))?;
    ClearingSandbox::from_snapshot_bytes(
        bytes,
        network,
        contract,
        collateral,
        permitted,
        collateral_policy,
        mark_terms,
    )
    .map_err(ClearingChainCheckpointError::State)
}
