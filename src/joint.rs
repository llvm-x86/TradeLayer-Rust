//! Stage 22: one registry and ONE spendable ledger per independent TradeLayer.
//!
//! The sole Ledger lives inside SpotExchange. Ordinary issue/send/mint/burn
//! operations and order reserves/fills/cancels all mutate that same ledger.
//! Registry activations and global input replay markers are co-snapshotted.
//! Typed inputs, actors and heights are still UNVERIFIED caller claims.
//!
//! This conservative vertical slice handles registry actions, token types
//! 1/2/11/12 and spot types 5/6. Type 3 and settlement types 23/31
//! explicitly deny. Other legacy channel/derivative operations are not
//! promoted to this experimental combined executor.

use crate::chain::ChainId;
use crate::execution::ProtocolState;
use crate::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use crate::network_profile::TradeLayerMainnet;
use crate::registry::{Registry, RegistryAction, RegistryError, RegistryTx};
use crate::spot::{
    CancelSelector, SpotError, SpotExchange, SpotIntent, UnverifiedSpotCancellation,
    UnverifiedSpotPlacement,
};
use std::collections::BTreeSet;

pub const MAX_JOINT_BLOCK_INPUTS: usize = 10_000;
pub const MAX_JOINT_REPLAY_MARKERS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JointOperation {
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
    SpotPlace(SpotIntent),
    SpotCancel {
        offered: PropertyId,
        desired: PropertyId,
        is_contract: bool,
        selector: CancelSelector,
    },
    /// Must obtain authenticated LTC payment evidence and fee/column rules.
    UtxoTradeType3,
    /// Rejected until an independent economic clearing verifier exists.
    EconomicSettlement {
        ty: u8,
    },
    /// No silent fallback from unsupported channel/derivatives opcodes.
    Unsupported {
        ty: u8,
    },
}

impl JointOperation {
    pub fn tx_type(&self) -> u8 {
        match self {
            Self::Registry(action) => action.tx_type(),
            Self::Issue { .. } => 1,
            Self::Send { .. } => 2,
            Self::Grant { .. } => 11,
            Self::Redeem { .. } => 12,
            Self::SpotPlace(_) => 5,
            Self::SpotCancel { .. } => 6,
            Self::UtxoTradeType3 => 3,
            Self::EconomicSettlement { ty } | Self::Unsupported { ty } => *ty,
        }
    }
}

/// The chain adapter must eventually derive these fields from raw L1 bytes.
/// This struct is intentionally named UNVERIFIED.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedJointTx {
    pub txid: TxId,
    pub actor: AccountId,
    pub operation: JointOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JointError {
    WrongChain,
    WrongNetwork,
    WrongAdministrator,
    InvalidHeight,
    FinalizedHeight,
    NoActivation(u8),
    ActivatedInSameBlock(u8),
    Replay(TxId),
    ReplayConflict,
    ReplayLimit,
    UnsupportedOperation(u8),
    UnsupportedLegacyLiabilities,
    SettlementAuthorizationRequired,
    UtxoAuthenticationRequired,
    InconsistentComponents,
    Registry(RegistryError),
    Spot(SpotError),
}
impl core::fmt::Display for JointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for JointError {}
impl From<RegistryError> for JointError {
    fn from(err: RegistryError) -> Self {
        Self::Registry(err)
    }
}
impl From<SpotError> for JointError {
    fn from(err: SpotError) -> Self {
        Self::Spot(err)
    }
}

/// Unlike a ProtocolState next to a SpotExchange, this object owns one
/// SpotExchange as the sole ledger authority. Registry state never carries a
/// second balance ledger. Every input txid is in exactly one component bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JointProtocolState {
    network: TradeLayerMainnet,
    registry: Registry,
    spot: SpotExchange,
    global_input_ids: BTreeSet<TxId>,
    ledger_input_ids: BTreeSet<TxId>,
    finalized_height: Option<u64>,
}

impl JointProtocolState {
    /// Explicit, unverified initial balances. The bootstrap ledger may have
    /// earlier issuance history, which remains externally unproven. The
    /// permitted spot properties must already exist in that same ledger.
    pub fn from_bootstrap_unverified(
        network: TradeLayerMainnet,
        admin: AccountId,
        bootstrap_ledger: Ledger,
        permitted: BTreeSet<PropertyId>,
    ) -> Result<Self, JointError> {
        if !bootstrap_ledger.has_no_channel_or_settlement_liabilities_unverified() {
            return Err(JointError::UnsupportedLegacyLiabilities);
        }
        let spot = SpotExchange::from_ledger_unverified(bootstrap_ledger, permitted)?;
        let state = Self {
            network,
            registry: Registry::new(admin),
            spot,
            global_input_ids: BTreeSet::new(),
            ledger_input_ids: BTreeSet::new(),
            finalized_height: None,
        };
        state.verify_components_unverified()?;
        Ok(state)
    }

    /// Explicit migration from an *existing* Stage 20 spot snapshot and an
    /// independently supplied registry history. No old rollback frames are
    /// inherited; the new chain anchor becomes the migration tip.
    ///
    /// Identical replay txids between component histories are rejected.
    pub fn migrate_spot_components_unverified(
        network: TradeLayerMainnet,
        registry: Registry,
        spot: SpotExchange,
        tip_height: u64,
    ) -> Result<Self, JointError> {
        if tip_height == 0
            || spot.processed_through() != Some(tip_height)
            || registry.last_height().is_some_and(|h| h > tip_height)
        {
            return Err(JointError::InconsistentComponents);
        }
        if !spot
            .ledger()
            .has_no_channel_or_settlement_liabilities_unverified()
        {
            return Err(JointError::UnsupportedLegacyLiabilities);
        }
        let mut global_input_ids = spot.spot_input_ids_unverified().clone();
        for id in registry.input_txids_unverified() {
            if !global_input_ids.insert(*id) {
                return Err(JointError::ReplayConflict);
            }
        }
        let state = Self {
            network,
            registry,
            spot,
            global_input_ids,
            ledger_input_ids: BTreeSet::new(),
            finalized_height: Some(tip_height),
        };
        state.verify_components_unverified()?;
        Ok(state)
    }

    /// A read-only import from a previously validated TLUC protocol snapshot.
    /// Its original global transaction markers become ordinary/registry
    /// input buckets in TLJP, and the registry and ledger are moved into one
    /// SpotExchange. Existing pending channels or settlement journals reject
    /// this migration because this joint executor cannot advance them.
    pub fn migrate_unified_components_unverified(
        network: TradeLayerMainnet,
        old: &ProtocolState,
        tip_height: u64,
        permitted: BTreeSet<PropertyId>,
    ) -> Result<Self, JointError> {
        if tip_height == 0 || old.finalized_height() != Some(tip_height) {
            return Err(JointError::InconsistentComponents);
        }
        if !old
            .ledger()
            .has_no_channel_or_settlement_liabilities_unverified()
        {
            return Err(JointError::UnsupportedLegacyLiabilities);
        }
        let mut spot = SpotExchange::from_ledger_unverified(old.ledger().clone(), permitted)?;
        spot.process_through_unverified(tip_height)?;
        let result = Self {
            network,
            registry: old.registry().clone(),
            spot,
            global_input_ids: old.input_txids_unverified().clone(),
            ledger_input_ids: old.ledger().input_txids_unverified().clone(),
            finalized_height: Some(tip_height),
        };
        result.verify_components_unverified()?;
        Ok(result)
    }

    pub const fn network(&self) -> TradeLayerMainnet {
        self.network
    }
    pub fn chain_id(&self) -> ChainId {
        self.network.chain_id()
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    pub fn ledger(&self) -> &Ledger {
        self.spot.ledger()
    }
    pub fn spot(&self) -> &SpotExchange {
        &self.spot
    }
    pub fn processed_count(&self) -> usize {
        self.global_input_ids.len()
    }
    pub fn finalized_height(&self) -> Option<u64> {
        self.finalized_height
    }

    fn check_activated(&self, ty: u8, height: u64) -> Result<(), JointError> {
        let record = self
            .registry
            .activation(ty)
            .ok_or(JointError::NoActivation(ty))?;
        if height <= record.height {
            return Err(JointError::ActivatedInSameBlock(ty));
        }
        Ok(())
    }

    /// Called only on a staged JointChainState. All input ids share one replay
    /// namespace even when the component stores additional internal transfer
    /// fingerprints for escrow bookkeeping.
    pub(crate) fn apply_one_unverified(
        &mut self,
        height: u64,
        transaction: UnverifiedJointTx,
    ) -> Result<(), JointError> {
        if height == 0 {
            return Err(JointError::InvalidHeight);
        }
        if self.finalized_height.is_some_and(|last| height <= last) {
            return Err(JointError::FinalizedHeight);
        }
        let txid = transaction.txid;
        if self.global_input_ids.contains(&txid)
            || self.spot.ledger().is_processed(txid)
            || self.registry.is_processed(txid)
        {
            return Err(JointError::Replay(txid));
        }
        if self.global_input_ids.len() >= MAX_JOINT_REPLAY_MARKERS {
            return Err(JointError::ReplayLimit);
        }
        let actor = transaction.actor;
        match transaction.operation {
            JointOperation::Registry(action) => {
                self.registry.apply(RegistryTx {
                    txid,
                    actor,
                    height,
                    action,
                })?;
            }
            JointOperation::Issue {
                id,
                kind,
                initial_atoms,
            } => {
                self.check_activated(1, height)?;
                self.spot.apply_joint_ledger_operation_unverified(
                    txid,
                    Operation::CreateProperty {
                        id,
                        issuer: actor,
                        kind,
                        initial_atoms,
                    },
                )?;
                self.ledger_input_ids.insert(txid);
            }
            JointOperation::Send {
                id,
                recipient,
                amount_atoms,
            } => {
                self.check_activated(2, height)?;
                self.spot.apply_joint_ledger_operation_unverified(
                    txid,
                    Operation::Transfer {
                        id,
                        from: actor,
                        to: recipient,
                        amount_atoms,
                    },
                )?;
                self.ledger_input_ids.insert(txid);
            }
            JointOperation::Grant {
                id,
                recipient,
                amount_atoms,
            } => {
                self.check_activated(11, height)?;
                self.spot.apply_joint_ledger_operation_unverified(
                    txid,
                    Operation::Mint {
                        id,
                        issuer: actor,
                        to: recipient,
                        amount_atoms,
                    },
                )?;
                self.ledger_input_ids.insert(txid);
            }
            JointOperation::Redeem { id, amount_atoms } => {
                self.check_activated(12, height)?;
                self.spot.apply_joint_ledger_operation_unverified(
                    txid,
                    Operation::Burn {
                        id,
                        issuer: actor,
                        amount_atoms,
                    },
                )?;
                self.ledger_input_ids.insert(txid);
            }
            JointOperation::SpotPlace(intent) => {
                self.check_activated(5, height)?;
                self.spot
                    .enqueue_type5_unverified(UnverifiedSpotPlacement {
                        txid,
                        actor,
                        height,
                        intent,
                    })?;
            }
            JointOperation::SpotCancel {
                offered,
                desired,
                is_contract,
                selector,
            } => {
                self.check_activated(6, height)?;
                self.spot
                    .cancel_type6_unverified(UnverifiedSpotCancellation {
                        txid,
                        actor,
                        height,
                        offered,
                        desired,
                        is_contract,
                        selector,
                    })?;
            }
            JointOperation::UtxoTradeType3 => return Err(JointError::UtxoAuthenticationRequired),
            JointOperation::EconomicSettlement { .. } => {
                return Err(JointError::SettlementAuthorizationRequired);
            }
            JointOperation::Unsupported { ty } => return Err(JointError::UnsupportedOperation(ty)),
        }
        self.global_input_ids.insert(txid);
        Ok(())
    }

    /// End-of-block queue matching spends the exact same ledger balances
    /// touched by preceding ordinary token operations in the block.
    pub(crate) fn finalize_block_unverified(&mut self, height: u64) -> Result<(), JointError> {
        if height == 0 || self.finalized_height.is_some_and(|prior| height <= prior) {
            return Err(JointError::InvalidHeight);
        }
        self.spot.process_through_unverified(height)?;
        self.finalized_height = Some(height);
        self.verify_components_unverified()
    }

    pub fn verify_components_unverified(&self) -> Result<(), JointError> {
        self.spot
            .ledger()
            .verify_conservation()
            .map_err(SpotError::Ledger)?;
        self.spot.verify_reserves()?;
        self.spot.reject_foreign_reserves_for_joint_unverified()?;
        if self.spot.processed_through() != self.finalized_height {
            return Err(JointError::InconsistentComponents);
        }
        if self
            .registry
            .last_height()
            .is_some_and(|h| self.finalized_height.is_none_or(|f| h > f))
        {
            return Err(JointError::InconsistentComponents);
        }
        let mut union = self.ledger_input_ids.clone();
        if self
            .ledger_input_ids
            .iter()
            .any(|id| !self.ledger().is_processed(*id))
        {
            return Err(JointError::ReplayConflict);
        }
        for id in self.registry.input_txids_unverified() {
            if self.ledger().is_processed(*id) || !union.insert(*id) {
                return Err(JointError::ReplayConflict);
            }
        }
        for id in self.spot.spot_input_ids_unverified() {
            if self.ledger().is_processed(*id) || !union.insert(*id) {
                return Err(JointError::ReplayConflict);
            }
        }
        if union != self.global_input_ids || union.len() > MAX_JOINT_REPLAY_MARKERS {
            return Err(JointError::ReplayConflict);
        }
        Ok(())
    }
}

// Full canonical registry + sole-ledger spot snapshot lives in this module
// to preserve the private global input-ID audit invariants.
include!("joint_snapshot_impl.rs");
