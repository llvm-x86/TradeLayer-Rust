//! Atomically coordinates registry and balance operations in one replay domain.
//!
//! This is a *staged*, unverified protocol projection. It does not process
//! cryptographically authenticated Bitcoin/Litecoin/Dogecoin transactions.
//! Callers supply claimed actors, chain IDs, transaction IDs and block heights.
//! Never connect it to live funds without a independently audited chain adapter.

use crate::chain::ChainId;
use crate::ledger::{
    AccountId, ChannelSide, Ledger, LedgerError, Operation, PropertyId, PropertyKind, TxId,
};
use crate::registry::{Registry, RegistryAction, RegistryError, RegistryTx};
use std::collections::BTreeSet;

pub const MAX_UNVERIFIED_BATCH: usize = 10_000;
/// Legacy queue processing begins seven blocks after a type-21 withdrawal request.
pub const CHANNEL_WITHDRAWAL_DELAY_BLOCKS: u64 = 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolOperation {
    Registry(RegistryAction),
    Issue {
        id: PropertyId,
        kind: PropertyKind,
        initial_atoms: u128,
    },
    Send {
        id: PropertyId,
        recipient: AccountId,
        amount_atoms: u128,
    },
    Grant {
        id: PropertyId,
        recipient: AccountId,
        amount_atoms: u128,
    },
    Redeem {
        id: PropertyId,
        amount_atoms: u128,
    },
    /// Type 4, research subset: caller claims column membership; no pay/clearlist flags.
    ChannelCommit {
        id: PropertyId,
        channel: AccountId,
        side: ChannelSide,
        amount_atoms: u128,
    },
    /// Type 21, research subset: reserve one property's withdrawal for seven blocks.
    ChannelWithdraw {
        id: PropertyId,
        channel: AccountId,
        side: ChannelSide,
        amount_atoms: u128,
    },
    /// Type 22, research subset: actor is the source CHANNEL, not the owner.
    /// Destination inherits the current participant of the source column.
    ChannelTransfer {
        id: PropertyId,
        from_side: ChannelSide,
        to_channel: AccountId,
        to_side: ChannelSide,
        amount_atoms: u128,
    },
}

impl ProtocolOperation {
    pub const fn tx_type(&self) -> u8 {
        match self {
            Self::Registry(action) => action.tx_type(),
            Self::Issue { .. } => 1,
            Self::Send { .. } => 2,
            Self::Grant { .. } => 11,
            Self::Redeem { .. } => 12,
            Self::ChannelCommit { .. } => 4,
            Self::ChannelWithdraw { .. } => 21,
            Self::ChannelTransfer { .. } => 22,
        }
    }
}

/// Untrusted input for offline replay and deterministic transition tests only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedTx {
    pub chain: ChainId,
    pub txid: TxId,
    pub claimed_actor: AccountId,
    pub claimed_height: u64,
    pub operation: ProtocolOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionError {
    EmptyBatch,
    BatchTooLarge,
    WrongChain,
    InvalidHeight,
    HeightRegression,
    Replay(TxId),
    NotActivated(u8),
    SameBlockActivation(u8),
    Registry(RegistryError),
    Ledger(LedgerError),
}

impl core::fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ExecutionError {}

impl From<RegistryError> for ExecutionError {
    fn from(value: RegistryError) -> Self {
        Self::Registry(value)
    }
}
impl From<LedgerError> for ExecutionError {
    fn from(value: LedgerError) -> Self {
        Self::Ledger(value)
    }
}

/// One state machine with a global replay set and all-or-nothing mutations.
/// Registry and ledger are otherwise unmodified, to facilitate differential
/// testing against previous standalone fixtures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolState {
    chain: ChainId,
    registry: Registry,
    ledger: Ledger,
    processed: BTreeSet<TxId>,
    last_height: Option<u64>,
}

impl ProtocolState {
    /// Chain ID and protocol admin MUST ultimately come from independently
    /// validated chain configuration. This constructor does not verify either.
    pub fn new_unverified(chain: ChainId, protocol_admin: AccountId) -> Self {
        Self {
            chain,
            registry: Registry::new(protocol_admin),
            ledger: Ledger::new(),
            processed: BTreeSet::new(),
            last_height: None,
        }
    }

    pub fn chain(&self) -> ChainId {
        self.chain
    }
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    pub fn processed_count(&self) -> usize {
        self.processed.len()
    }
    pub fn last_height(&self) -> Option<u64> {
        self.last_height
    }

    /// A clone is the current experimental rollback mechanism. Production
    /// reorg journals must serialize the registry and ledger together.
    pub fn apply_unverified(&mut self, tx: UnverifiedTx) -> Result<(), ExecutionError> {
        self.apply_batch_unverified([tx])
    }

    pub fn apply_batch_unverified(
        &mut self,
        transactions: impl IntoIterator<Item = UnverifiedTx>,
    ) -> Result<(), ExecutionError> {
        let mut pending = self.clone();
        let mut count = 0;
        for tx in transactions {
            count += 1;
            if count > MAX_UNVERIFIED_BATCH {
                return Err(ExecutionError::BatchTooLarge);
            }
            if tx.chain != pending.chain {
                return Err(ExecutionError::WrongChain);
            }
            if tx.claimed_height == 0 {
                return Err(ExecutionError::InvalidHeight);
            }
            if pending.last_height.is_some_and(|h| tx.claimed_height < h) {
                return Err(ExecutionError::HeightRegression);
            }
            if !pending.processed.insert(tx.txid) {
                return Err(ExecutionError::Replay(tx.txid));
            }
            pending.apply_one(tx)?;
        }
        if count == 0 {
            return Err(ExecutionError::EmptyBatch);
        }
        *self = pending;
        Ok(())
    }

    fn assert_activated(&self, ty: u8, height: u64) -> Result<(), ExecutionError> {
        let activation = self
            .registry
            .activation(ty)
            .ok_or(ExecutionError::NotActivated(ty))?;
        if height <= activation.height {
            return Err(ExecutionError::SameBlockActivation(ty));
        }
        Ok(())
    }

    fn apply_one(&mut self, tx: UnverifiedTx) -> Result<(), ExecutionError> {
        let ty = tx.operation.tx_type();
        let actor = tx.claimed_actor;
        let height = tx.claimed_height;
        let txid = tx.txid;
        match tx.operation {
            ProtocolOperation::Registry(action) => {
                self.registry.apply(RegistryTx {
                    txid,
                    actor,
                    height,
                    action,
                })?;
            }
            ProtocolOperation::Issue {
                id,
                kind,
                initial_atoms,
            } => {
                self.assert_activated(ty, height)?;
                self.ledger.apply(
                    txid,
                    Operation::CreateProperty {
                        id,
                        issuer: actor,
                        kind,
                        initial_atoms,
                    },
                )?;
            }
            ProtocolOperation::Send {
                id,
                recipient,
                amount_atoms,
            } => {
                self.assert_activated(ty, height)?;
                self.ledger.apply(
                    txid,
                    Operation::Transfer {
                        id,
                        from: actor,
                        to: recipient,
                        amount_atoms,
                    },
                )?;
            }
            ProtocolOperation::Grant {
                id,
                recipient,
                amount_atoms,
            } => {
                self.assert_activated(ty, height)?;
                self.ledger.apply(
                    txid,
                    Operation::Mint {
                        id,
                        issuer: actor,
                        to: recipient,
                        amount_atoms,
                    },
                )?;
            }
            ProtocolOperation::Redeem { id, amount_atoms } => {
                self.assert_activated(ty, height)?;
                self.ledger.apply(
                    txid,
                    Operation::Burn {
                        id,
                        issuer: actor,
                        amount_atoms,
                    },
                )?;
            }
            ProtocolOperation::ChannelCommit {
                id,
                channel,
                side,
                amount_atoms,
            } => {
                self.assert_activated(ty, height)?;
                self.ledger.apply(
                    txid,
                    Operation::ChannelCommit {
                        id,
                        owner: actor,
                        channel,
                        side,
                        amount_atoms,
                    },
                )?;
            }
            ProtocolOperation::ChannelWithdraw {
                id,
                channel,
                side,
                amount_atoms,
            } => {
                self.assert_activated(ty, height)?;
                let unlock_height = height
                    .checked_add(CHANNEL_WITHDRAWAL_DELAY_BLOCKS)
                    .ok_or(ExecutionError::InvalidHeight)?;
                self.ledger.apply(
                    txid,
                    Operation::ChannelWithdraw {
                        id,
                        owner: actor,
                        channel,
                        side,
                        amount_atoms,
                        unlock_height,
                    },
                )?;
            }
            ProtocolOperation::ChannelTransfer {
                id,
                from_side,
                to_channel,
                to_side,
                amount_atoms,
            } => {
                self.assert_activated(ty, height)?;
                // For legacy type 22 the signed actor is the source channel.
                self.ledger.apply(
                    txid,
                    Operation::ChannelTransfer {
                        id,
                        from_channel: actor,
                        from_side,
                        to_channel,
                        to_side,
                        amount_atoms,
                    },
                )?;
            }
        }
        self.last_height = Some(height);
        Ok(())
    }

    /// After processing all operations for a block, mature withdrawals even
    /// when the block contains no TradeLayer transactions. Reorgs restore the
    /// pre-block unified snapshot including pending reserves.
    pub fn finalize_block_unverified(&mut self, height: u64) -> Result<usize, ExecutionError> {
        if height == 0 {
            return Err(ExecutionError::InvalidHeight);
        }
        if self.last_height.is_some_and(|previous| height < previous) {
            return Err(ExecutionError::HeightRegression);
        }
        self.ledger
            .settle_due_withdrawals_unverified(height)
            .map_err(ExecutionError::Ledger)
    }
}

// Canonical full-registry+ledger snapshots with shared replay-set checks.
include!("execution_snapshot_impl.rs");
