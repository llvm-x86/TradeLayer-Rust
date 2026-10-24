//! Stage 25A: an isolated, zero-crate Rust margin and funding state machine.
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

use crate::funding_audit::{
    audit_funding, FundingAuditError, FundingMode, FundingPosition, FundingProjection,
};
use crate::hash256::double_sha256;
use crate::ledger::{AccountId, Ledger, LedgerError, Operation, PropertyId, TxId};
use crate::network_profile::TradeLayerMainnet;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub const MAX_CLEARING_OWNERS: usize = 128;
pub const MAX_CLEARING_INPUTS: usize = 4096;
pub const MAX_CLEARING_TRADES_PER_INPUT: usize = 1024;
const ESCROW_PREFIX: &str = "~clrm:";

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
    Replay(TxId),
    PositionOverflow,
    ArithmeticOverflow,
    InjectedFailure { before_leg: usize },
    Ledger(LedgerError),
    Funding(FundingAuditError),
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
impl From<FundingAuditError> for ClearingError {
    fn from(error: FundingAuditError) -> Self {
        Self::Funding(error)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FundingReceipt {
    pub nominal_atoms: u128,
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
    ledger: Ledger,
    margin_escrows: BTreeMap<AccountId, AccountId>,
    positions: BTreeMap<AccountId, i64>,
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
        let candidate = Self {
            network,
            contract,
            collateral,
            ledger,
            margin_escrows: BTreeMap::new(),
            positions: BTreeMap::new(),
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
        &self.ledger
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
            self.ledger.available_atoms(self.collateral, escrow)
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

    fn internal_txid(&self, outer: TxId, purpose: &[u8], leg: usize) -> Result<TxId, ClearingError> {
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
        if self.seen.contains(&external) || self.ledger.is_processed(external) {
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
        staged.verify()?;
        *self = staged;
        Ok(result)
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
                || staged.ledger.available_atoms(staged.collateral, &escrow) > 0
            {
                return Err(ClearingError::TooManyOwners);
            }
            staged.ledger.apply(
                staged.internal_txid(external, b"deposit", 0)?,
                Operation::Transfer {
                    id: staged.collateral,
                    from: owner.clone(),
                    to: escrow.clone(),
                    amount_atoms,
                },
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
            staged.ledger.apply(
                staged.internal_txid(external, b"withdraw", 0)?,
                Operation::Transfer {
                    id: staged.collateral,
                    from: escrow.clone(),
                    to: owner.clone(),
                    amount_atoms,
                },
            )?;
            if staged.ledger.available_atoms(staged.collateral, &escrow) == 0 {
                staged.margin_escrows.remove(&owner);
            }
            Ok(())
        })
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
                let size = i64::try_from(trade.contracts)
                    .map_err(|_| ClearingError::PositionOverflow)?;
                for (owner, delta) in [(&trade.long, size), (&trade.short, -size)] {
                    if staged.margin_atoms(owner) == 0 {
                        return Err(ClearingError::UnknownMarginOwner);
                    }
                    let before = staged.positions.get(owner).copied().unwrap_or(0);
                    let after = before.checked_add(delta).ok_or(ClearingError::PositionOverflow)?;
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
            let legs = staged.settlement_legs(&projection)?;
            for (leg_number, (from, to, amount)) in legs.iter().enumerate() {
                if failure_before_leg == Some(leg_number) {
                    return Err(ClearingError::InjectedFailure {
                        before_leg: leg_number,
                    });
                }
                staged.ledger.apply(
                    staged.internal_txid(external, b"funding", leg_number)?,
                    Operation::Transfer {
                        id: staged.collateral,
                        from: from.clone(),
                        to: to.clone(),
                        amount_atoms: *amount,
                    },
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

    fn settlement_legs(
        &self,
        projection: &FundingProjection,
    ) -> Result<Vec<(AccountId, AccountId, u128)>, ClearingError> {
        if projection.net_atoms != 0 {
            return Err(ClearingError::OpenInterestImbalance);
        }
        let mut payers = Vec::new();
        let mut receivers = Vec::new();
        for entry in &projection.deltas {
            if entry.delta_atoms == 0 {
                continue;
            }
            let owner = AccountId::try_from(entry.account.as_str())?;
            let escrow = self
                .margin_escrows
                .get(&owner)
                .ok_or(ClearingError::UnknownMarginOwner)?
                .clone();
            if entry.delta_atoms < 0 {
                payers.push((escrow, entry.delta_atoms.unsigned_abs()));
            } else {
                receivers.push((
                    escrow,
                    u128::try_from(entry.delta_atoms)
                        .map_err(|_| ClearingError::ArithmeticOverflow)?,
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
            legs.push((payers[payer].0.clone(), receivers[receiver].0.clone(), amount));
            payers[payer].1 -= amount;
            receivers[receiver].1 -= amount;
            if payers[payer].1 == 0 {
                payer += 1;
            }
            if receivers[receiver].1 == 0 {
                receiver += 1;
            }
        }
        if payer != payers.len() || receiver != receivers.len()
            || legs.len() > MAX_CLEARING_OWNERS * 2
        {
            return Err(ClearingError::OpenInterestImbalance);
        }
        Ok(legs)
    }

    /// Check every escrow against this *same* Ledger's inventory and net
    /// positions. This does not claim maintenance margin sufficiency.
    pub fn verify(&self) -> Result<(), ClearingError> {
        if !self.ledger.has_no_channel_or_settlement_liabilities_unverified() {
            return Err(ClearingError::UnsupportedExistingLiabilities);
        }
        self.ledger.verify_conservation()?;
        if self.ledger.property(self.collateral).is_none() {
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
        let mut escrow_addresses = BTreeSet::new();
        for (owner, escrow) in &self.margin_escrows {
            Self::validate_owner(owner)?;
            if *escrow != self.derived_escrow(owner)?
                || !escrow_addresses.insert(escrow.clone())
            {
                return Err(ClearingError::EscrowCollision);
            }
        }
        for (property, account, _) in self.ledger.balances_with_account_prefix("~") {
            if property != self.collateral || !escrow_addresses.contains(&account) {
                return Err(ClearingError::EscrowCollision);
            }
        }
        let mut net = 0i128;
        for (owner, size) in &self.positions {
            if *size == 0 || !self.margin_escrows.contains_key(owner) {
                return Err(ClearingError::UnknownMarginOwner);
            }
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
