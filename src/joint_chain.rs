//! Stage 22: block-ordered, rollback-safe joint registry+spot accounting.
//!
//! Each independent Bitcoin/Litecoin/Dogecoin TradeLayer uses exactly one
//! shared token ledger, owned by JointProtocolState's SpotExchange, alongside
//! its own registry. No cross-chain state or settlement authority is created.
//! All blocks and actors remain UNVERIFIED caller assertions, never production
//! consensus inputs. Existing TLUC/TLSC snapshots remain unchanged.

use crate::chain::{BlockHeader, BlockId, Tip, MAX_REORG_HISTORY};
use crate::codec::{verify_crc, with_crc, Cursor, MAX_CHECKPOINT_BYTES};
use crate::joint::{
    JointError, JointProtocolState, JointSnapshotError, UnverifiedJointTx,
    MAX_JOINT_BLOCK_INPUTS,
};
use crate::ledger::{AccountId, Ledger, PropertyId};
use crate::network_bound::NetworkBoundSpot;
use crate::network_profile::{NetworkProfileError, TradeLayerMainnet};
use crate::registry::Registry;
use std::collections::{BTreeSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 5] = b"TLJC\x01";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedJointBlock {
    pub network: TradeLayerMainnet,
    pub header: BlockHeader,
    pub transactions: Vec<UnverifiedJointTx>,
}

#[derive(Debug)]
pub enum JointChainError {
    InvalidHistoryLimit,
    WrongNetwork,
    InvalidHeight,
    ParentMismatch,
    DuplicateBlockId,
    TooManyTransactions,
    Joint(JointError),
    WrongRollbackTip,
    EmptyRollbackHistory,
    Checkpoint(JointCheckpointError),
}
impl core::fmt::Display for JointChainError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for JointChainError {}
impl From<JointError> for JointChainError {
    fn from(value: JointError) -> Self {
        Self::Joint(value)
    }
}
impl From<JointCheckpointError> for JointChainError {
    fn from(value: JointCheckpointError) -> Self {
        Self::Checkpoint(value)
    }
}

#[derive(Debug)]
pub enum JointCheckpointError {
    Invalid(&'static str),
    Profile(NetworkProfileError),
    State(JointSnapshotError),
    Io(std::io::Error),
}
impl core::fmt::Display for JointCheckpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for JointCheckpointError {}
impl From<JointSnapshotError> for JointCheckpointError {
    fn from(value: JointSnapshotError) -> Self {
        Self::State(value)
    }
}
impl From<std::io::Error> for JointCheckpointError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
fn bad(reason: &'static str) -> JointCheckpointError {
    JointCheckpointError::Invalid(reason)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PriorJoint {
    tip: Tip,
    state: JointProtocolState,
}

/// A single evolving registry+ledger+spot state with one global input txid
/// replay set and bounded complete pre-block snapshot history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JointChainState {
    network: TradeLayerMainnet,
    anchor: Tip,
    tip: Tip,
    state: JointProtocolState,
    history: VecDeque<PriorJoint>,
    history_limit: usize,
}

impl JointChainState {
    pub fn new_unverified(
        network: TradeLayerMainnet,
        anchor: Tip,
        admin: AccountId,
        ledger: Ledger,
        permitted_spot_properties: BTreeSet<PropertyId>,
        history_limit: usize,
    ) -> Result<Self, JointChainError> {
        if !(1..=MAX_REORG_HISTORY).contains(&history_limit) {
            return Err(JointChainError::InvalidHistoryLimit);
        }
        network
            .check_genesis_if_applicable(anchor)
            .map_err(|_| JointChainError::WrongNetwork)?;
        let state = JointProtocolState::from_bootstrap_unverified(
            network,
            admin,
            ledger,
            permitted_spot_properties,
        )?;
        Ok(Self {
            network,
            anchor,
            tip: anchor,
            state,
            history: VecDeque::new(),
            history_limit,
        })
    }

    /// Explicit one-time Stage 20 spot migration. The registry is supplied
    /// separately by the caller; both histories still require independent
    /// chain provenance. Existing Stage 20 rollback frames are NOT copied.
    /// A new TLJC anchor is established at the old tip.
    pub fn migrate_from_spot_unverified(
        old: &NetworkBoundSpot,
        registry: Registry,
        history_limit: usize,
    ) -> Result<Self, JointChainError> {
        if !(1..=MAX_REORG_HISTORY).contains(&history_limit) {
            return Err(JointChainError::InvalidHistoryLimit);
        }
        let network = old.network();
        let tip = old.tip();
        if old.chain_id() != network.chain_id() {
            return Err(JointChainError::WrongNetwork);
        }
        let state = JointProtocolState::migrate_spot_components_unverified(
            network,
            registry,
            old.state().state().clone(),
            tip.height,
        )?;
        Ok(Self {
            network,
            anchor: tip,
            tip,
            state,
            history: VecDeque::new(),
            history_limit,
        })
    }

    pub const fn network(&self) -> TradeLayerMainnet {
        self.network
    }
    pub fn tip(&self) -> Tip {
        self.tip
    }
    pub fn anchor(&self) -> Tip {
        self.anchor
    }
    pub fn state(&self) -> &JointProtocolState {
        &self.state
    }
    pub fn rollback_depth(&self) -> usize {
        self.history.len()
    }

    /// Entire mixed block commits atomically, including registry activations,
    /// ordinary balances, spot queues/fills, and all replay input IDs.
    /// End-of-block matching uses the same Ledger as in-block token sends.
    pub fn apply_unverified_block(
        &mut self,
        block: &UnverifiedJointBlock,
    ) -> Result<(), JointChainError> {
        if block.network != self.network {
            return Err(JointChainError::WrongNetwork);
        }
        if self.tip.height.checked_add(1) != Some(block.header.height) {
            return Err(JointChainError::InvalidHeight);
        }
        if block.header.parent != self.tip.id {
            return Err(JointChainError::ParentMismatch);
        }
        if block.header.id == self.tip.id
            || self.history.iter().any(|prior| prior.tip.id == block.header.id)
        {
            return Err(JointChainError::DuplicateBlockId);
        }
        if block.transactions.len() > MAX_JOINT_BLOCK_INPUTS {
            return Err(JointChainError::TooManyTransactions);
        }
        let mut staged = self.state.clone();
        for tx in &block.transactions {
            staged.apply_one_unverified(block.header.height, tx.clone())?;
        }
        staged.finalize_block_unverified(block.header.height)?;
        self.history.push_back(PriorJoint {
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

    pub fn rollback_tip(&mut self, expected: Tip) -> Result<(), JointChainError> {
        if expected != self.tip {
            return Err(JointChainError::WrongRollbackTip);
        }
        let previous = self
            .history
            .pop_back()
            .ok_or(JointChainError::EmptyRollbackHistory)?;
        self.tip = previous.tip;
        self.state = previous.state;
        Ok(())
    }

    pub fn apply_unverified_block_and_checkpoint(
        &mut self,
        block: &UnverifiedJointBlock,
        path: &Path,
    ) -> Result<(), JointChainError> {
        let mut staged = self.clone();
        staged.apply_unverified_block(block)?;
        staged.write_new_checkpoint(path)?;
        *self = staged;
        Ok(())
    }

    /// TLJC v1 stores genesis, network identity, tip and every retained
    /// TLJP (one-ledger, full registry+spot) checkpoint, including rollback.
    pub fn checkpoint_bytes(&self) -> Result<Vec<u8>, JointCheckpointError> {
        let mut bytes = MAGIC.to_vec();
        bytes.push(self.network.tag());
        bytes.extend_from_slice(&self.network.genesis().0);
        bytes.extend_from_slice(&self.network.chain_id().0);
        write_tip(&mut bytes, self.anchor);
        write_tip(&mut bytes, self.tip);
        bytes.push(u8::try_from(self.history_limit).map_err(|_| bad("history policy"))?);
        bytes.push(u8::try_from(self.history.len()).map_err(|_| bad("history count"))?);
        for prior in &self.history {
            write_tip(&mut bytes, prior.tip);
            write_state(&mut bytes, &prior.state)?;
        }
        write_state(&mut bytes, &self.state)?;
        with_crc(bytes).map_err(|_| bad("joint checkpoint too large"))
    }

    pub fn from_checkpoint_bytes(
        bytes: &[u8],
        expected_network: TradeLayerMainnet,
        expected_anchor: Tip,
        expected_admin: &AccountId,
        expected_permitted: &BTreeSet<PropertyId>,
        expected_history_limit: usize,
    ) -> Result<Self, JointCheckpointError> {
        if !(1..=MAX_REORG_HISTORY).contains(&expected_history_limit) {
            return Err(bad("invalid history policy"));
        }
        expected_network
            .check_genesis_if_applicable(expected_anchor)
            .map_err(JointCheckpointError::Profile)?;
        let raw = verify_crc(bytes).map_err(|_| bad("joint checkpoint CRC/size"))?;
        let mut reader = Cursor::new(raw);
        if reader.fixed::<5>().map_err(|_| bad("joint magic"))? != *MAGIC {
            return Err(bad("joint checkpoint version"));
        }
        let network = TradeLayerMainnet::from_tag(
            reader.u8().map_err(|_| bad("network tag missing"))?,
        )
        .map_err(JointCheckpointError::Profile)?;
        let genesis = reader.fixed::<32>().map_err(|_| bad("genesis truncated"))?;
        let chain_id = reader.fixed::<32>().map_err(|_| bad("chain ID truncated"))?;
        if network != expected_network
            || genesis != expected_network.genesis().0
            || chain_id != expected_network.chain_id().0
        {
            return Err(bad("wrong mainnet identity"));
        }
        let anchor = read_tip(&mut reader)?;
        let tip = read_tip(&mut reader)?;
        if anchor != expected_anchor {
            return Err(bad("wrong checkpoint anchor"));
        }
        let history_limit = reader.u8().map_err(|_| bad("history limit"))? as usize;
        let count = reader.u8().map_err(|_| bad("history count"))? as usize;
        if history_limit != expected_history_limit || count > history_limit {
            return Err(bad("invalid rollback policy/count"));
        }
        let mut history: VecDeque<PriorJoint> = VecDeque::new();
        for _ in 0..count {
            let prev_tip = read_tip(&mut reader)?;
            if prev_tip.height < anchor.height
                || history.back().is_some_and(|prev| {
                    prev.tip.height.checked_add(1) != Some(prev_tip.height)
                })
            {
                return Err(bad("discontinuous rollback history"));
            }
            let state = read_state(
                &mut reader,
                expected_network,
                expected_admin,
                expected_permitted,
            )?;
            if !state_height_compatible(&state, prev_tip, anchor) {
                return Err(bad("prior joint state height mismatch"));
            }
            history.push_back(PriorJoint {
                tip: prev_tip,
                state,
            });
        }
        let state = read_state(
            &mut reader,
            expected_network,
            expected_admin,
            expected_permitted,
        )?;
        reader.end().map_err(|_| bad("trailing joint checkpoint bytes"))?;
        if tip.height < anchor.height
            || (tip.height == anchor.height && (tip != anchor || count != 0))
            || (tip.height > anchor.height && count == 0)
            || history.back().is_some_and(|prior| {
                prior.tip.height.checked_add(1) != Some(tip.height)
                    || prior.tip.id == tip.id
            })
            || !state_height_compatible(&state, tip, anchor)
        {
            return Err(bad("joint tip/state mismatch"));
        }
        let result = Self {
            network,
            anchor,
            tip,
            state,
            history,
            history_limit,
        };
        if result.checkpoint_bytes()?.as_slice() != bytes {
            return Err(bad("noncanonical joint checkpoint"));
        }
        Ok(result)
    }

    pub fn write_new_checkpoint(&self, path: &Path) -> Result<(), JointCheckpointError> {
        let bytes = self.checkpoint_bytes()?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        if let Err(err) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(JointCheckpointError::Io(err));
        }
        Ok(())
    }

    pub fn load_checkpoint(
        path: &Path,
        network: TradeLayerMainnet,
        anchor: Tip,
        admin: &AccountId,
        permitted: &BTreeSet<PropertyId>,
        history_limit: usize,
    ) -> Result<Self, JointCheckpointError> {
        let file = File::open(path)?;
        let size = usize::try_from(file.metadata()?.len())
            .map_err(|_| bad("checkpoint file size"))?;
        if size > MAX_CHECKPOINT_BYTES {
            return Err(bad("checkpoint file excessive"));
        }
        let mut bytes = Vec::with_capacity(size);
        file.take((MAX_CHECKPOINT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(bad("checkpoint read excessive"));
        }
        Self::from_checkpoint_bytes(
            &bytes,
            network,
            anchor,
            admin,
            permitted,
            history_limit,
        )
    }
}

fn state_height_compatible(state: &JointProtocolState, tip: Tip, anchor: Tip) -> bool {
    if tip == anchor {
        state.finalized_height().is_none()
            || state.finalized_height() == Some(anchor.height)
    } else {
        state.finalized_height() == Some(tip.height)
    }
}
fn write_tip(out: &mut Vec<u8>, tip: Tip) {
    out.extend_from_slice(&tip.height.to_be_bytes());
    out.extend_from_slice(&tip.id.0);
}
fn read_tip(reader: &mut Cursor<'_>) -> Result<Tip, JointCheckpointError> {
    let height = reader.u64().map_err(|_| bad("tip height"))?;
    let id = BlockId(reader.fixed::<32>().map_err(|_| bad("tip id"))?);
    Ok(Tip { height, id })
}
fn write_state(
    out: &mut Vec<u8>,
    state: &JointProtocolState,
) -> Result<(), JointCheckpointError> {
    let snapshot = state.snapshot_bytes()?;
    out.extend_from_slice(
        &u32::try_from(snapshot.len())
            .map_err(|_| bad("embedded TLJP size"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(&snapshot);
    Ok(())
}
fn read_state(
    reader: &mut Cursor<'_>,
    network: TradeLayerMainnet,
    admin: &AccountId,
    permitted: &BTreeSet<PropertyId>,
) -> Result<JointProtocolState, JointCheckpointError> {
    let size = reader.u32().map_err(|_| bad("embedded TLJP length"))? as usize;
    if size == 0 || size > MAX_CHECKPOINT_BYTES {
        return Err(bad("invalid embedded TLJP length"));
    }
    JointProtocolState::from_snapshot_bytes(
        reader.take(size).map_err(|_| bad("embedded TLJP truncated"))?,
        network,
        admin,
        permitted,
    )
    .map_err(JointCheckpointError::State)
}
