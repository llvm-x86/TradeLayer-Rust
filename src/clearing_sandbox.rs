//! Stage 25C: an isolated, zero-crate Rust margin, funding and mark-to-market state machine.
//!
//! A single owned Ledger holds every atom, including contract-margin escrows.
//! Signed positions and escrow ownership are *metadata*, never second balances.
//! Deposits, closing trades, withdrawals and funding legs are all staged on a
//! private clone and committed only after full invariant verification.
//!
//! This is an unverified, OFFLINE experiment: actors, trade matches, heights,
//! contract identifiers, and funding rates are all caller-supplied. It is not
//! wired to JointChainState, cannot process historical transactions, and is
//! NOT an executable authorization for derivative or type 23/31 settlement.
//! Initial margin requirements, liquidation and insurance are not implemented.

use crate::clearing_math::PnlRounding;
use crate::clearing_pnl::{project_pnl_unverified, PnlError, PnlProjection};
use crate::funding_audit::{audit_funding, FundingAuditError, FundingMode, FundingPosition};
use crate::hash256::double_sha256;
use crate::ledger::{AccountId, Ledger, LedgerError, Operation, PropertyId, TxId};
use crate::liquidation_audit::{
    assess_liquidation_risk_unverified, LiquidationAuditError, LiquidationRiskReport,
};
use crate::network_profile::TradeLayerMainnet;
use crate::spot::{SpotError, SpotExchange, UnverifiedSpotPlacement};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub const MAX_CLEARING_OWNERS: usize = 128;
pub const MAX_CLEARING_INPUTS: usize = 4096;
pub const MAX_CLEARING_TRADES_PER_INPUT: usize = 1024;
const ESCROW_PREFIX: &str = "~clrm:";
const INSURANCE_ESCROW_PREFIX: &str = "~ins:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearingError {
    InvalidContract,
    UnknownCollateral,
    UnsupportedExistingLiabilities,
    ReservedOwner,
    EscrowCollision,
    UnknownMarginOwner,
    NotFlat,
    ZeroAmount,
    ZeroContracts,
    SelfTrade,
    EmptyTradeBatch,
    OpenInterestImbalance,
    TooManyOwners,
    TooManyInputs,
    TooManyTrades,
    InvalidCollateralPolicy,
    CollateralPolicyFrozen,
    LiquidationPolicyMissing,
    InvalidInsuranceSponsor,
    InsurancePolicyMissing,
    InsurancePolicyFrozen,
    UnauthorizedInsuranceSponsor,
    InvalidMarkPolicy,
    MarkPolicyMissing,
    MarkPolicyFrozen,
    StaleMark,
    PendingSpotOrders,
    PnlRoundingImbalance(i128),
    NoDistress,
    MissingOppositePosition,
    NotStrictPartial,
    InsufficientInsurance {
        required_atoms: u128,
        available_atoms: u128,
    },
    InsufficientMargin {
        account: AccountId,
        required_atoms: u128,
        available_atoms: u128,
    },
    Replay(TxId),
    PositionOverflow,
    ArithmeticOverflow,
    InjectedFailure {
        before_leg: usize,
    },
    Ledger(LedgerError),
    Spot(SpotError),
    Funding(FundingAuditError),
    Pnl(PnlError),
    Liquidation(LiquidationAuditError),
}

impl core::fmt::Display for ClearingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ClearingError {}

impl From<LedgerError> for ClearingError {
    fn from(error: LedgerError) -> Self {
        Self::Ledger(error)
    }
}
impl From<SpotError> for ClearingError {
    fn from(error: SpotError) -> Self {
        match error {
            SpotError::Ledger(inner) => Self::Ledger(inner),
            other => Self::Spot(other),
        }
    }
}

impl From<FundingAuditError> for ClearingError {
    fn from(error: FundingAuditError) -> Self {
        Self::Funding(error)
    }
}

impl From<PnlError> for ClearingError {
    fn from(error: PnlError) -> Self {
        Self::Pnl(error)
    }
}
impl From<LiquidationAuditError> for ClearingError {
    fn from(error: LiquidationAuditError) -> Self {
        Self::Liquidation(error)
    }
}

/// A matched pair in a synthetic offline trade. Both participants are
/// explicitly specified and *not* independently authenticated by this type.
/// The long gains quantity; the short loses an equal quantity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedContractTrade {
    pub long: AccountId,
    pub short: AccountId,
    pub contracts: u64,
}

/// Explicit research-only collateral policy. This does NOT specify TradeLayer
/// economic consensus; the caller cannot change it after the first clearing
/// operation, and no historical contract price or leverage is inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollateralPolicy {
    pub initial_atoms_per_contract: u128,
    pub maintenance_atoms_per_contract: u128,
}

/// One immutable research sponsor for a contract's *actual* insurance escrow.
/// The sponsor address is explicitly caller-supplied, not signature-verified.
/// The insurer cannot withdraw or authorize liquidations in Stage 25E2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsurancePolicy {
    pub sponsor: AccountId,
}

impl InsurancePolicy {
    fn validate(&self) -> Result<(), ClearingError> {
        if self.sponsor.as_str().starts_with('~') {
            return Err(ClearingError::InvalidInsuranceSponsor);
        }
        Ok(())
    }
}

impl CollateralPolicy {
    fn validate(self) -> Result<Self, ClearingError> {
        if self.initial_atoms_per_contract == 0
            || self.maintenance_atoms_per_contract == 0
            || self.maintenance_atoms_per_contract > self.initial_atoms_per_contract
        {
            return Err(ClearingError::InvalidCollateralPolicy);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FundingReceipt {
    pub nominal_atoms: u128,
    pub ledger_transfer_legs: usize,
    pub active_positions: usize,
}

/// One trusted-by-caller research configuration for a synthetic contract.
/// The height and price are not authenticated oracle observations. Only the
/// price/height advance after a successful mark settlement; the notional,
/// inverse convention and rounding model stay frozen for this sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkPolicy {
    pub height: u64,
    pub price_e8: u64,
    pub notional_e8: u64,
    pub inverse: bool,
    pub rounding: PnlRounding,
}

impl MarkPolicy {
    fn validate(self) -> Result<Self, ClearingError> {
        if self.height == 0 || self.price_e8 == 0 || self.notional_e8 == 0 {
            return Err(ClearingError::InvalidMarkPolicy);
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkReceipt {
    pub from_price_e8: u64,
    pub to_price_e8: u64,
    pub new_height: u64,
    pub gross_transfer_atoms: u128,
    pub ledger_transfer_legs: usize,
    pub active_positions: usize,
}

/// The only ledger is inside this owner. No separate shadow balance map.
/// Escrow addresses are domain-separated by network genesis, contract,
/// collateral ID and participant; they cannot be supplied as transaction
/// actors. The hash is only a prototype namespace, not a proof of authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearingSandbox {
    network: TradeLayerMainnet,
    contract: u32,
    collateral: PropertyId,
    spot: SpotExchange,
    margin_escrows: BTreeMap<AccountId, AccountId>,
    positions: BTreeMap<AccountId, i64>,
    collateral_policy: Option<CollateralPolicy>,
    mark_policy: Option<MarkPolicy>,
    insurance_policy: Option<InsurancePolicy>,
    seen: BTreeSet<TxId>,
}

impl ClearingSandbox {
    /// Requires the caller to transfer sole ownership of an existing ledger.
    /// Foreign spot escrows, channels and settlement journals are not
    /// automatically importable into this narrow clearing experiment.
    pub fn from_ledger_unverified(
        network: TradeLayerMainnet,
        contract: u32,
        collateral: PropertyId,
        ledger: Ledger,
    ) -> Result<Self, ClearingError> {
        if contract == 0 {
            return Err(ClearingError::InvalidContract);
        }
        if ledger.property(collateral).is_none() {
            return Err(ClearingError::UnknownCollateral);
        }
        if !ledger.has_no_channel_or_settlement_liabilities_unverified() {
            return Err(ClearingError::UnsupportedExistingLiabilities);
        }
        if !ledger.balances_with_account_prefix("~").is_empty() {
            return Err(ClearingError::EscrowCollision);
        }
        ledger.verify_conservation()?;
        let spot = SpotExchange::from_ledger_unverified(ledger, BTreeSet::new())?;
        Self::from_spot_unverified(network, contract, collateral, spot)
    }

    /// Consume a preexisting spot exchange and its SOLE owned token ledger.
    /// This is an offline research lane and not a JointChainState migration.
    pub fn from_spot_unverified(
        network: TradeLayerMainnet,
        contract: u32,
        collateral: PropertyId,
        spot: SpotExchange,
    ) -> Result<Self, ClearingError> {
        if contract == 0 {
            return Err(ClearingError::InvalidContract);
        }
        if spot.ledger().property(collateral).is_none() {
            return Err(ClearingError::UnknownCollateral);
        }
        if !spot
            .ledger()
            .has_no_channel_or_settlement_liabilities_unverified()
        {
            return Err(ClearingError::UnsupportedExistingLiabilities);
        }
        spot.verify_reserves()?;
        if spot.has_clearing_activity_unverified() {
            return Err(ClearingError::EscrowCollision);
        }
        if spot
            .ledger()
            .balances_with_account_prefix("~")
            .iter()
            .any(|(_, account, _)| !account.as_str().starts_with("~spot:"))
        {
            return Err(ClearingError::EscrowCollision);
        }
        let candidate = Self {
            network,
            contract,
            collateral,
            spot,
            margin_escrows: BTreeMap::new(),
            positions: BTreeMap::new(),
            collateral_policy: None,
            mark_policy: None,
            insurance_policy: None,
            seen: BTreeSet::new(),
        };
        candidate.verify()?;
        Ok(candidate)
    }

    pub const fn network(&self) -> TradeLayerMainnet {
        self.network
    }

    pub const fn contract(&self) -> u32 {
        self.contract
    }

    pub const fn collateral(&self) -> PropertyId {
        self.collateral
    }

    pub fn ledger(&self) -> &Ledger {
        self.spot.ledger()
    }

    pub fn spot(&self) -> &SpotExchange {
        &self.spot
    }

    /// Caller assertions, not authenticated chain inputs. Ordinary ledger
    /// mutations cannot touch either spot or clearing reserved addresses.
    pub fn apply_ordinary_operation_unverified(
        &mut self,
        external: TxId,
        operation: Operation,
    ) -> Result<(), ClearingError> {
        self.transaction(external, move |staged| {
            staged
                .spot
                .apply_joint_ledger_operation_unverified(external, operation)?;
            Ok(())
        })
    }

    /// Spot orders and margin obligations share one available balance.
    pub fn enqueue_spot_order_unverified(
        &mut self,
        entry: UnverifiedSpotPlacement,
    ) -> Result<(), ClearingError> {
        if self.seen.contains(&entry.txid)
            || self.ledger().is_processed(entry.txid)
            || self.spot.spot_input_ids_unverified().contains(&entry.txid)
        {
            return Err(ClearingError::Replay(entry.txid));
        }
        if self
            .mark_policy
            .is_some_and(|mark| entry.height <= mark.height)
        {
            return Err(ClearingError::StaleMark);
        }
        let mut staged = self.clone();
        staged.spot.enqueue_type5_unverified(entry)?;
        staged.verify()?;
        *self = staged;
        Ok(())
    }

    pub fn process_spot_block_unverified(&mut self, height: u64) -> Result<usize, ClearingError> {
        if self.mark_policy.is_some_and(|mark| height <= mark.height) {
            return Err(ClearingError::StaleMark);
        }
        let mut staged = self.clone();
        let count = staged.spot.process_through_unverified(height)?;
        staged.verify()?;
        *self = staged;
        Ok(count)
    }

    /// Configure explicit, bounded synthetic initial/maintenance amounts
    /// once, before any clearing event. No production consensus activation.
    pub fn with_collateral_policy_unverified(
        mut self,
        policy: CollateralPolicy,
    ) -> Result<Self, ClearingError> {
        if self.collateral_policy.is_some()
            || !self.seen.is_empty()
            || !self.positions.is_empty()
            || self.spot.has_clearing_activity_unverified()
        {
            return Err(ClearingError::CollateralPolicyFrozen);
        }
        self.collateral_policy = Some(policy.validate()?);
        self.verify()?;
        Ok(self)
    }

    pub const fn collateral_policy(&self) -> Option<CollateralPolicy> {
        self.collateral_policy
    }

    /// Select one explicit economic rounding/price convention before any
    /// clearing event, never inferred from a client's raw transaction.
    pub fn with_mark_policy_unverified(
        mut self,
        policy: MarkPolicy,
    ) -> Result<Self, ClearingError> {
        if self.mark_policy.is_some()
            || !self.seen.is_empty()
            || !self.positions.is_empty()
            || self.spot.has_clearing_activity_unverified()
        {
            return Err(ClearingError::MarkPolicyFrozen);
        }
        let policy = policy.validate()?;
        // The initial mark cannot predate a previously finalized spot block;
        // a pending spot queue also has no authenticated ordering yet.
        if self.spot.pending_count() > 0 {
            return Err(ClearingError::MarkPolicyFrozen);
        }
        if self
            .spot
            .processed_through()
            .is_some_and(|height| policy.height < height)
        {
            return Err(ClearingError::StaleMark);
        }
        self.mark_policy = Some(policy);
        self.verify()?;
        Ok(self)
    }

    pub const fn mark_policy(&self) -> Option<MarkPolicy> {
        self.mark_policy
    }

    /// Opt in to a sole-ledger insurance escrow BEFORE clearing activity.
    /// The policy itself is persisted by TLCL v2; older TLSP/TLJP/TLCC v1
    /// cannot drop it even if no insurance funds have yet been donated.
    pub fn with_insurance_policy_unverified(
        mut self,
        policy: InsurancePolicy,
    ) -> Result<Self, ClearingError> {
        if self.insurance_policy.is_some()
            || !self.seen.is_empty()
            || !self.positions.is_empty()
            || self.spot.has_clearing_activity_unverified()
        {
            return Err(ClearingError::InsurancePolicyFrozen);
        }
        policy.validate()?;
        self.insurance_policy = Some(policy);
        self.spot.note_clearing_event_unverified();
        self.verify()?;
        Ok(self)
    }

    pub fn insurance_policy(&self) -> Option<&InsurancePolicy> {
        self.insurance_policy.as_ref()
    }

    /// The insurer holds REAL available property atoms in the existing
    /// SpotExchange-owned Ledger. This is never a duplicated account map.
    pub fn insurance_reserve_atoms(&self) -> u128 {
        self.insurance_escrow_for_audit().map_or(0, |escrow| {
            self.spot.ledger().available_atoms(self.collateral, &escrow)
        })
    }

    pub fn insurance_escrow_for_audit(&self) -> Option<AccountId> {
        self.insurance_policy
            .as_ref()
            .and_then(|_| self.derived_insurance_escrow().ok())
    }

    /// Observe the per-owner signed PnL, including any independent-rounding
    /// residual, WITHOUT writing balances. Imbalanced projections may be
    /// diagnosed here but are never accepted by settle_mark_to_market.
    pub fn preview_mark_to_market_unverified(
        &self,
        new_height: u64,
        new_price_e8: u64,
    ) -> Result<PnlProjection, ClearingError> {
        let policy = self.mark_policy.ok_or(ClearingError::MarkPolicyMissing)?;
        if new_height <= policy.height
            || self
                .spot
                .processed_through()
                .is_some_and(|height| new_height < height)
        {
            return Err(ClearingError::StaleMark);
        }
        if new_price_e8 == 0 {
            return Err(ClearingError::InvalidMarkPolicy);
        }
        if self
            .spot
            .has_queued_spot_at_or_before_unverified(new_height)
        {
            return Err(ClearingError::PendingSpotOrders);
        }
        Ok(project_pnl_unverified(
            &self.positions,
            policy.price_e8,
            new_price_e8,
            policy.notional_e8,
            policy.inverse,
            policy.rounding,
        )?)
    }

    /// Read-only liquidation preflight at a proposed mark. Uses only real
    /// margin escrows owned by this sole SpotExchange ledger, never free
    /// available balances, pending spot order reserves, or a separate JS
    /// insurance DB. It may report risk when actual mark settlement is denied.
    /// This API NEVER authorizes an AMM match, margin seizure or insurance.
    pub fn preview_liquidation_unverified(
        &self,
        new_height: u64,
        new_price_e8: u64,
    ) -> Result<LiquidationRiskReport, ClearingError> {
        let policy = self
            .collateral_policy
            .ok_or(ClearingError::LiquidationPolicyMissing)?;
        let mark = self.preview_mark_to_market_unverified(new_height, new_price_e8)?;
        let posted_margins = self
            .positions
            .keys()
            .map(|owner| (owner.clone(), self.margin_atoms(owner)))
            .collect::<BTreeMap<_, _>>();
        Ok(assess_liquidation_risk_unverified(
            &self.positions,
            &posted_margins,
            &mark,
            policy.maintenance_atoms_per_contract,
        )?)
    }

    pub fn processed_count(&self) -> usize {
        self.seen.len()
    }

    pub fn positions(&self) -> &BTreeMap<AccountId, i64> {
        &self.positions
    }

    /// Margin is an actual balance at a reserved address in the SOLE ledger.
    /// No independent numeric margin ledger is maintained.
    pub fn margin_atoms(&self, owner: &AccountId) -> u128 {
        self.margin_escrows.get(owner).map_or(0, |escrow| {
            self.spot.ledger().available_atoms(self.collateral, escrow)
        })
    }

    pub fn margin_escrow_for_audit(&self, owner: &AccountId) -> Option<&AccountId> {
        self.margin_escrows.get(owner)
    }

    fn validate_owner(owner: &AccountId) -> Result<(), ClearingError> {
        if owner.as_str().starts_with('~') {
            return Err(ClearingError::ReservedOwner);
        }
        Ok(())
    }

    fn derived_escrow(&self, owner: &AccountId) -> Result<AccountId, ClearingError> {
        let mut domain = Vec::with_capacity(160);
        domain.extend_from_slice(b"TradeLayer/clearing-margin/stage25a");
        domain.extend_from_slice(&self.network.genesis().0);
        domain.extend_from_slice(&self.contract.to_be_bytes());
        domain.extend_from_slice(&self.collateral.get().to_be_bytes());
        domain.extend_from_slice(&(owner.as_str().len() as u16).to_be_bytes());
        domain.extend_from_slice(owner.as_str().as_bytes());
        let hash = double_sha256(&domain);
        let mut address = String::from(ESCROW_PREFIX);
        for byte in hash {
            write!(&mut address, "{byte:02x}").map_err(|_| ClearingError::ArithmeticOverflow)?;
        }
        Ok(AccountId::try_from(address.as_str())?)
    }

    /// Network/contract/collateral and sponsor-separated *account identity*.
    /// Does not independently attest that the sponsor is authorized.
    fn derived_insurance_escrow(&self) -> Result<AccountId, ClearingError> {
        let policy = self
            .insurance_policy
            .as_ref()
            .ok_or(ClearingError::InsurancePolicyMissing)?;
        policy.validate()?;
        let mut domain = Vec::with_capacity(160);
        domain.extend_from_slice(b"TradeLayer/insurance-reserve/stage25e2");
        domain.extend_from_slice(&self.network.genesis().0);
        domain.extend_from_slice(&self.contract.to_be_bytes());
        domain.extend_from_slice(&self.collateral.get().to_be_bytes());
        let sponsor_bytes = policy.sponsor.as_str().as_bytes();
        let sponsor_len =
            u16::try_from(sponsor_bytes.len()).map_err(|_| ClearingError::ArithmeticOverflow)?;
        domain.extend_from_slice(&sponsor_len.to_be_bytes());
        domain.extend_from_slice(sponsor_bytes);
        let mut address = String::from(INSURANCE_ESCROW_PREFIX);
        for byte in double_sha256(&domain) {
            write!(&mut address, "{byte:02x}").map_err(|_| ClearingError::ArithmeticOverflow)?;
        }
        AccountId::try_from(address.as_str()).map_err(ClearingError::Ledger)
    }

    fn internal_txid(
        &self,
        outer: TxId,
        purpose: &[u8],
        leg: usize,
    ) -> Result<TxId, ClearingError> {
        let leg = u32::try_from(leg).map_err(|_| ClearingError::ArithmeticOverflow)?;
        let mut input = Vec::with_capacity(128);
        input.extend_from_slice(b"TradeLayer/clearing-internal/stage25a");
        input.extend_from_slice(&self.network.genesis().0);
        input.extend_from_slice(&self.contract.to_be_bytes());
        input.extend_from_slice(&self.collateral.get().to_be_bytes());
        input.extend_from_slice(&outer.as_bytes());
        input.extend_from_slice(&(purpose.len() as u16).to_be_bytes());
        input.extend_from_slice(purpose);
        input.extend_from_slice(&leg.to_be_bytes());
        Ok(TxId::from_bytes(double_sha256(&input)))
    }

    fn guard_event(&self, external: TxId) -> Result<(), ClearingError> {
        if self.seen.contains(&external)
            || self.spot.ledger().is_processed(external)
            || self.spot.spot_input_ids_unverified().contains(&external)
        {
            return Err(ClearingError::Replay(external));
        }
        if self.seen.len() >= MAX_CLEARING_INPUTS {
            return Err(ClearingError::TooManyInputs);
        }
        Ok(())
    }

    /// Generic clone/check/swap transaction: a failure at ANY step restores
    /// the original ledger, positions, escrow ownership and replay markers.
    fn transaction<T>(
        &mut self,
        external: TxId,
        execute: impl FnOnce(&mut Self) -> Result<T, ClearingError>,
    ) -> Result<T, ClearingError> {
        self.guard_event(external)?;
        let mut staged = self.clone();
        let result = execute(&mut staged)?;
        if !staged.seen.insert(external) {
            return Err(ClearingError::Replay(external));
        }
        // Even a no-op funding or zero-position mark must permanently fence
        // older spot-only snapshots which omit clearing replay identifiers.
        staged.spot.note_clearing_event_unverified();
        staged.verify()?;
        *self = staged;
        Ok(result)
    }

    /// Sponsor-funded deposit only. There is deliberately no withdrawal,
    /// loss payout, insurance claim or liquidation action in this stage.
    /// Every atom moves from the sponsor to the one derived ~ins: escrow.
    pub fn deposit_insurance_unverified(
        &mut self,
        external: TxId,
        sponsor: AccountId,
        amount_atoms: u128,
    ) -> Result<(), ClearingError> {
        Self::validate_owner(&sponsor)?;
        if amount_atoms == 0 {
            return Err(ClearingError::ZeroAmount);
        }
        self.transaction(external, move |staged| {
            let policy = staged
                .insurance_policy
                .as_ref()
                .ok_or(ClearingError::InsurancePolicyMissing)?;
            if sponsor != policy.sponsor {
                return Err(ClearingError::UnauthorizedInsuranceSponsor);
            }
            let escrow = staged.derived_insurance_escrow()?;
            staged.spot.deposit_insurance_reserve_unverified(
                staged.internal_txid(external, b"insurance-deposit", 0)?,
                staged.collateral,
                sponsor,
                escrow,
                amount_atoms,
            )?;
            Ok(())
        })
    }

    pub fn deposit_margin_unverified(
        &mut self,
        external: TxId,
        owner: AccountId,
        amount_atoms: u128,
    ) -> Result<(), ClearingError> {
        Self::validate_owner(&owner)?;
        if amount_atoms == 0 {
            return Err(ClearingError::ZeroAmount);
        }
        self.transaction(external, move |staged| {
            let escrow = staged.derived_escrow(&owner)?;
            if let Some(existing) = staged.margin_escrows.get(&owner) {
                if existing != &escrow {
                    return Err(ClearingError::EscrowCollision);
                }
            } else if staged.margin_escrows.len() >= MAX_CLEARING_OWNERS
                || staged
                    .spot
                    .ledger()
                    .available_atoms(staged.collateral, &escrow)
                    > 0
            {
                return Err(ClearingError::TooManyOwners);
            }
            staged.spot.apply_clearing_transfer_unverified(
                staged.internal_txid(external, b"deposit", 0)?,
                staged.collateral,
                owner.clone(),
                escrow.clone(),
                amount_atoms,
            )?;
            staged.margin_escrows.insert(owner, escrow);
            Ok(())
        })
    }

    /// Withdrawal is only possible while a participant is flat. No order,
    /// liquidation or leverage policy is inferred from this research method.
    pub fn withdraw_margin_unverified(
        &mut self,
        external: TxId,
        owner: AccountId,
        amount_atoms: u128,
    ) -> Result<(), ClearingError> {
        Self::validate_owner(&owner)?;
        if amount_atoms == 0 {
            return Err(ClearingError::ZeroAmount);
        }
        self.transaction(external, move |staged| {
            if staged.positions.contains_key(&owner) {
                return Err(ClearingError::NotFlat);
            }
            let escrow = staged
                .margin_escrows
                .get(&owner)
                .ok_or(ClearingError::UnknownMarginOwner)?
                .clone();
            staged.spot.apply_clearing_transfer_unverified(
                staged.internal_txid(external, b"withdraw", 0)?,
                staged.collateral,
                escrow.clone(),
                owner.clone(),
                amount_atoms,
            )?;
            if staged
                .spot
                .ledger()
                .available_atoms(staged.collateral, &escrow)
                == 0
            {
                staged.margin_escrows.remove(&owner);
            }
            Ok(())
        })
    }

    fn check_collateral(
        &self,
        owner: &AccountId,
        contracts: i64,
        for_increasing_exposure: bool,
    ) -> Result<(), ClearingError> {
        if contracts == 0 {
            return Ok(());
        }
        if let Some(policy) = self.collateral_policy {
            let per_contract = if for_increasing_exposure {
                policy.initial_atoms_per_contract
            } else {
                policy.maintenance_atoms_per_contract
            };
            let required = u128::from(contracts.unsigned_abs())
                .checked_mul(per_contract)
                .ok_or(ClearingError::ArithmeticOverflow)?;
            let available = self.margin_atoms(owner);
            if available < required {
                return Err(ClearingError::InsufficientMargin {
                    account: owner.clone(),
                    required_atoms: required,
                    available_atoms: available,
                });
            }
        }
        Ok(())
    }

    /// Matches are only simulated by a caller; no exchange evidence is
    /// established. Equal/opposite updates preserve global signed OI.
    pub fn apply_matched_trades_unverified(
        &mut self,
        external: TxId,
        trades: &[MatchedContractTrade],
    ) -> Result<(), ClearingError> {
        if trades.is_empty() {
            return Err(ClearingError::EmptyTradeBatch);
        }
        if trades.len() > MAX_CLEARING_TRADES_PER_INPUT {
            return Err(ClearingError::TooManyTrades);
        }
        self.transaction(external, |staged| {
            for trade in trades {
                Self::validate_owner(&trade.long)?;
                Self::validate_owner(&trade.short)?;
                if trade.long == trade.short {
                    return Err(ClearingError::SelfTrade);
                }
                if trade.contracts == 0 {
                    return Err(ClearingError::ZeroContracts);
                }
                let size =
                    i64::try_from(trade.contracts).map_err(|_| ClearingError::PositionOverflow)?;
                for (owner, delta) in [(&trade.long, size), (&trade.short, -size)] {
                    if staged.margin_atoms(owner) == 0 {
                        return Err(ClearingError::UnknownMarginOwner);
                    }
                    let before = staged.positions.get(owner).copied().unwrap_or(0);
                    let after = before
                        .checked_add(delta)
                        .ok_or(ClearingError::PositionOverflow)?;
                    let flipped =
                        before != 0 && after != 0 && before.is_negative() != after.is_negative();
                    staged.check_collateral(
                        owner,
                        after,
                        flipped || after.unsigned_abs() > before.unsigned_abs(),
                    )?;
                    if after == 0 {
                        staged.positions.remove(owner);
                    } else {
                        staged.positions.insert(owner.clone(), after);
                    }
                }
            }
            Ok(())
        })
    }

    fn funding_inner(
        &mut self,
        external: TxId,
        notional_atoms: u64,
        rate_bps_e4: i64,
        failure_before_leg: Option<usize>,
    ) -> Result<FundingReceipt, ClearingError> {
        self.transaction(external, |staged| {
            let observations: Vec<FundingPosition> = staged
                .positions
                .iter()
                .map(|(owner, &contracts)| FundingPosition {
                    account: owner.as_str().to_owned(),
                    contracts,
                    available_atoms: staged.margin_atoms(owner),
                })
                .collect();
            let projection = audit_funding(
                &observations,
                notional_atoms,
                rate_bps_e4,
                FundingMode::ConservingProposal,
            )?;
            let signed: Vec<(AccountId, i128)> = projection
                .deltas
                .iter()
                .map(|delta| {
                    Ok((
                        AccountId::try_from(delta.account.as_str())?,
                        delta.delta_atoms,
                    ))
                })
                .collect::<Result<_, ClearingError>>()?;
            let legs = staged.settlement_legs(&signed, projection.net_atoms)?;
            for (leg_number, (from, to, amount)) in legs.iter().enumerate() {
                if failure_before_leg == Some(leg_number) {
                    return Err(ClearingError::InjectedFailure {
                        before_leg: leg_number,
                    });
                }
                staged.spot.apply_clearing_transfer_unverified(
                    staged.internal_txid(external, b"funding", leg_number)?,
                    staged.collateral,
                    from.clone(),
                    to.clone(),
                    *amount,
                )?;
            }
            if failure_before_leg == Some(legs.len()) {
                return Err(ClearingError::InjectedFailure {
                    before_leg: legs.len(),
                });
            }
            Ok(FundingReceipt {
                nominal_atoms: projection.nominal_atoms,
                ledger_transfer_legs: legs.len(),
                active_positions: observations.len(),
            })
        })
    }

    pub fn settle_funding_unverified(
        &mut self,
        external: TxId,
        notional_atoms: u64,
        rate_bps_e4: i64,
    ) -> Result<FundingReceipt, ClearingError> {
        self.funding_inner(external, notional_atoms, rate_bps_e4, None)
    }

    /// Fault-injection API for offline state-machine regression tests only.
    /// All work takes place on a clone and is discarded on the injected fault.
    pub fn settle_funding_fault_injected_unverified(
        &mut self,
        external: TxId,
        notional_atoms: u64,
        rate_bps_e4: i64,
        fail_before_leg: usize,
    ) -> Result<FundingReceipt, ClearingError> {
        self.funding_inner(external, notional_atoms, rate_bps_e4, Some(fail_before_leg))
    }

    fn mark_inner(
        &mut self,
        external: TxId,
        new_height: u64,
        new_price_e8: u64,
        failure_before_leg: Option<usize>,
    ) -> Result<MarkReceipt, ClearingError> {
        self.transaction(external, |staged| {
            let preview = staged.preview_mark_to_market_unverified(new_height, new_price_e8)?;
            if preview.net_atoms != 0 {
                // Never mint, burn or silently redistribute one-atom residuals.
                // Policy makers must independently specify an apportionment rule
                // before such multi-party scenarios become executable.
                return Err(ClearingError::PnlRoundingImbalance(preview.net_atoms));
            }
            let signed: Vec<_> = preview
                .deltas
                .iter()
                .map(|entry| (entry.account.clone(), entry.delta_atoms))
                .collect();
            let legs = staged.settlement_legs(&signed, preview.net_atoms)?;
            for (leg_number, (from, to, amount)) in legs.iter().enumerate() {
                if failure_before_leg == Some(leg_number) {
                    return Err(ClearingError::InjectedFailure {
                        before_leg: leg_number,
                    });
                }
                staged.spot.apply_clearing_transfer_unverified(
                    staged.internal_txid(external, b"mark-pnl", leg_number)?,
                    staged.collateral,
                    from.clone(),
                    to.clone(),
                    *amount,
                )?;
            }
            if failure_before_leg == Some(legs.len()) {
                return Err(ClearingError::InjectedFailure {
                    before_leg: legs.len(),
                });
            }
            let policy = staged.mark_policy.ok_or(ClearingError::MarkPolicyMissing)?;
            staged.mark_policy = Some(MarkPolicy {
                height: new_height,
                price_e8: new_price_e8,
                ..policy
            });
            // transaction() checks maintenance floors, supply conservation,
            // reserve ownership and replay markers before the atomic swap.
            Ok(MarkReceipt {
                from_price_e8: preview.from_price_e8,
                to_price_e8: preview.to_price_e8,
                new_height,
                gross_transfer_atoms: preview.payer_atoms,
                ledger_transfer_legs: legs.len(),
                active_positions: preview.deltas.len(),
            })
        })
    }

    /// Apply one unverified mark step to the sole token ledger. No arbitrary
    /// external payer or recipient is accepted; the signed position map
    /// determines every margin escrow debited or credited.
    pub fn settle_mark_to_market_unverified(
        &mut self,
        external: TxId,
        new_height: u64,
        new_price_e8: u64,
    ) -> Result<MarkReceipt, ClearingError> {
        self.mark_inner(external, new_height, new_price_e8, None)
    }

    /// Research-only fault injection at every margin transfer, including the
    /// boundary after the final leg but before the complete state is swapped.
    pub fn settle_mark_fault_injected_unverified(
        &mut self,
        external: TxId,
        new_height: u64,
        new_price_e8: u64,
        fail_before_leg: usize,
    ) -> Result<MarkReceipt, ClearingError> {
        self.mark_inner(external, new_height, new_price_e8, Some(fail_before_leg))
    }

    /// One deterministic transfer graph for signed funding or mark PnL.
    /// All edges remain inside this sandbox's owned clearing-margin accounts.
    fn settlement_legs(
        &self,
        deltas: &[(AccountId, i128)],
        net_atoms: i128,
    ) -> Result<Vec<(AccountId, AccountId, u128)>, ClearingError> {
        if net_atoms != 0 {
            return Err(ClearingError::OpenInterestImbalance);
        }
        let mut payers = Vec::new();
        let mut receivers = Vec::new();
        for (owner, delta_atoms) in deltas {
            if *delta_atoms == 0 {
                continue;
            }
            let escrow = self
                .margin_escrows
                .get(owner)
                .ok_or(ClearingError::UnknownMarginOwner)?
                .clone();
            if *delta_atoms < 0 {
                payers.push((escrow, delta_atoms.unsigned_abs()));
            } else {
                receivers.push((
                    escrow,
                    u128::try_from(*delta_atoms).map_err(|_| ClearingError::ArithmeticOverflow)?,
                ));
            }
        }
        let mut legs = Vec::new();
        let mut payer = 0;
        let mut receiver = 0;
        while payer < payers.len() && receiver < receivers.len() {
            let amount = payers[payer].1.min(receivers[receiver].1);
            if amount == 0 || payers[payer].0 == receivers[receiver].0 {
                return Err(ClearingError::EscrowCollision);
            }
            legs.push((
                payers[payer].0.clone(),
                receivers[receiver].0.clone(),
                amount,
            ));
            payers[payer].1 -= amount;
            receivers[receiver].1 -= amount;
            if payers[payer].1 == 0 {
                payer += 1;
            }
            if receivers[receiver].1 == 0 {
                receiver += 1;
            }
        }
        if payer != payers.len()
            || receiver != receivers.len()
            || legs.len() > MAX_CLEARING_OWNERS * 2
        {
            return Err(ClearingError::OpenInterestImbalance);
        }
        Ok(legs)
    }

    /// Check every escrow against this *same* Ledger's inventory and net
    /// positions. This does not claim maintenance margin sufficiency.
    pub fn verify(&self) -> Result<(), ClearingError> {
        if !self
            .spot
            .ledger()
            .has_no_channel_or_settlement_liabilities_unverified()
        {
            return Err(ClearingError::UnsupportedExistingLiabilities);
        }
        self.spot.ledger().verify_conservation()?;
        self.spot.verify_reserves()?;
        if let Some(mark) = self.mark_policy {
            mark.validate()?;
        }
        if let Some(insurance) = &self.insurance_policy {
            insurance.validate()?;
            if !self.spot.has_clearing_activity_unverified() {
                return Err(ClearingError::EscrowCollision);
            }
        }
        if self.spot.ledger().property(self.collateral).is_none() {
            return Err(ClearingError::UnknownCollateral);
        }
        if self.margin_escrows.len() > MAX_CLEARING_OWNERS
            || self.positions.len() > MAX_CLEARING_OWNERS
        {
            return Err(ClearingError::TooManyOwners);
        }
        if self.seen.len() > MAX_CLEARING_INPUTS {
            return Err(ClearingError::TooManyInputs);
        }
        // Clearing replay markers are not part of the old spot format. A
        // restored combined state MUST retain their permanent history fence.
        if (!self.seen.is_empty() || !self.margin_escrows.is_empty() || !self.positions.is_empty())
            && !self.spot.has_clearing_activity_unverified()
        {
            return Err(ClearingError::EscrowCollision);
        }
        for id in &self.seen {
            // Ordinary operations intentionally use the SAME external ID in
            // this clearing event journal and the underlying Ledger's
            // processed set. That overlap is valid and must survive TLCL.
            // An external spot order ID, however, is disjoint from every
            // outer clearing/ordinary replay marker.
            if self.spot.spot_input_ids_unverified().contains(id) {
                return Err(ClearingError::Replay(*id));
            }
        }
        let mut escrow_addresses = BTreeSet::new();
        for (owner, escrow) in &self.margin_escrows {
            Self::validate_owner(owner)?;
            if *escrow != self.derived_escrow(owner)? || !escrow_addresses.insert(escrow.clone()) {
                return Err(ClearingError::EscrowCollision);
            }
        }
        let insurance_address = match &self.insurance_policy {
            Some(_) => Some(self.derived_insurance_escrow()?),
            None => None,
        };
        for (property, account, _) in self.spot.ledger().balances_with_account_prefix("~") {
            // All spot-order reserves are separately matched against their
            // actual orderbook by SpotExchange::verify_reserves.
            if account.as_str().starts_with("~spot:") {
                continue;
            }
            if property != self.collateral
                || (!escrow_addresses.contains(&account)
                    && insurance_address.as_ref() != Some(&account))
            {
                return Err(ClearingError::EscrowCollision);
            }
        }
        let mut net = 0i128;
        for (owner, size) in &self.positions {
            if *size == 0 || !self.margin_escrows.contains_key(owner) {
                return Err(ClearingError::UnknownMarginOwner);
            }
            self.check_collateral(owner, *size, false)?;
            net = net
                .checked_add(i128::from(*size))
                .ok_or(ClearingError::PositionOverflow)?;
        }
        if net != 0 {
            return Err(ClearingError::OpenInterestImbalance);
        }
        Ok(())
    }
}

// Stage 25D codec shares private escrow/position ownership with this type.
include!("clearing_snapshot_impl.rs");

// Stage25E2B: private full-book insured closeout under one atomic ledger.
include!("clearing_distress_impl.rs");

// Stage25F: strict synthetic bilateral reduction, not whole-book liquidation.
include!("clearing_partial_impl.rs");
