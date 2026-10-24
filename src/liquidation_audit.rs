//! Stage 25E1: read-only liquidation and loss-pool preflight.
//!
//! This module moves NO tokens and never closes or liquidates a position.
//! It describes post-mark margin risk from the exact signed PnL projection
//! and separately compares hypothetical available/margin/insurance funding
//! policies. External reserve claims and market prices are NOT trusted.
//! A source-level JavaScript insurance balance is a disconnected DB value;
//! this Rust port must eventually put real reserves in the sole Ledger.

use crate::clearing_pnl::PnlProjection;
use crate::ledger::AccountId;
use std::collections::BTreeMap;

pub const MAX_RISK_POSITIONS: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiquidationAuditError {
    InvalidMaintenance,
    TooManyPositions,
    ZeroPosition,
    MissingMargin(AccountId),
    PositionProjectionMismatch,
    UnbalancedOpenInterest,
    ArithmeticOverflow,
}
impl core::fmt::Display for LiquidationAuditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for LiquidationAuditError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginRisk {
    Healthy,
    BelowMaintenance,
    Insolvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountLiquidationRisk {
    pub account: AccountId,
    pub contracts: i64,
    /// Real margin escrow atoms, supplied by the one owned Ledger.
    pub posted_margin_atoms: u128,
    pub projected_pnl_atoms: i128,
    /// Signed mathematical result. May be negative; NOT a spendable balance.
    pub post_mark_equity_atoms: i128,
    pub maintenance_required_atoms: u128,
    pub maintenance_deficit_atoms: u128,
    /// Part of a predicted mark loss not payable from posted margin.
    /// No insurer or counterparty is charged by computing this number.
    pub uncovered_mark_loss_atoms: u128,
    pub risk: MarginRisk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidationRiskReport {
    /// Stable account-name ordering for inspecting every position.
    pub accounts: Vec<AccountLiquidationRisk>,
    /// Nonhealthy owner IDs: insolvency first, then greatest maintenance gap,
    /// then lexical owner ID. A *diagnostic* priority, not AMM/ADL execution.
    pub candidates_by_severity: Vec<AccountId>,
    pub total_maintenance_deficit_atoms: u128,
    pub total_uncovered_mark_loss_atoms: u128,
    pub rounded_pnl_residual_atoms: i128,
}

/// An immutable, bounded, input-complete risk projection. No caller-supplied
/// available balance, spot escrow or insurance budget counts as pledged margin.
/// The only margin values should come from a real owned clearing-margin escrow.
///
/// The rounding residual is reported verbatim; unlike successful mark posting,
/// this diagnostic can analyze a nonzero residual, but cannot authorize it.
pub fn assess_liquidation_risk_unverified(
    positions: &BTreeMap<AccountId, i64>,
    posted_margins: &BTreeMap<AccountId, u128>,
    mark: &PnlProjection,
    maintenance_atoms_per_contract: u128,
) -> Result<LiquidationRiskReport, LiquidationAuditError> {
    if maintenance_atoms_per_contract == 0 {
        return Err(LiquidationAuditError::InvalidMaintenance);
    }
    if positions.len() > MAX_RISK_POSITIONS {
        return Err(LiquidationAuditError::TooManyPositions);
    }
    if mark.deltas.len() != positions.len() {
        return Err(LiquidationAuditError::PositionProjectionMismatch);
    }
    let mut net_open_interest = 0i128;
    let mut accounts = Vec::with_capacity(positions.len());
    let mut total_maintenance_deficit_atoms = 0u128;
    let mut total_uncovered_mark_loss_atoms = 0u128;
    for ((owner, &contracts), delta) in positions.iter().zip(&mark.deltas) {
        if owner != &delta.account {
            return Err(LiquidationAuditError::PositionProjectionMismatch);
        }
        if contracts == 0 {
            return Err(LiquidationAuditError::ZeroPosition);
        }
        net_open_interest = net_open_interest
            .checked_add(i128::from(contracts))
            .ok_or(LiquidationAuditError::ArithmeticOverflow)?;
        let margin = *posted_margins
            .get(owner)
            .ok_or_else(|| LiquidationAuditError::MissingMargin(owner.clone()))?;
        let margin_signed =
            i128::try_from(margin).map_err(|_| LiquidationAuditError::ArithmeticOverflow)?;
        let maintenance = u128::from(contracts.unsigned_abs())
            .checked_mul(maintenance_atoms_per_contract)
            .ok_or(LiquidationAuditError::ArithmeticOverflow)?;
        let maintenance_signed =
            i128::try_from(maintenance).map_err(|_| LiquidationAuditError::ArithmeticOverflow)?;
        let equity = margin_signed
            .checked_add(delta.delta_atoms)
            .ok_or(LiquidationAuditError::ArithmeticOverflow)?;
        let maintenance_deficit = if equity < maintenance_signed {
            maintenance_signed
                .checked_sub(equity)
                .ok_or(LiquidationAuditError::ArithmeticOverflow)? as u128
        } else {
            0
        };
        let mark_loss = if delta.delta_atoms < 0 {
            delta.delta_atoms.unsigned_abs()
        } else {
            0
        };
        let uncovered = mark_loss.saturating_sub(margin);
        total_maintenance_deficit_atoms = total_maintenance_deficit_atoms
            .checked_add(maintenance_deficit)
            .ok_or(LiquidationAuditError::ArithmeticOverflow)?;
        total_uncovered_mark_loss_atoms = total_uncovered_mark_loss_atoms
            .checked_add(uncovered)
            .ok_or(LiquidationAuditError::ArithmeticOverflow)?;
        let risk = if equity < 0 {
            MarginRisk::Insolvent
        } else if equity < maintenance_signed {
            MarginRisk::BelowMaintenance
        } else {
            MarginRisk::Healthy
        };
        accounts.push(AccountLiquidationRisk {
            account: owner.clone(),
            contracts,
            posted_margin_atoms: margin,
            projected_pnl_atoms: delta.delta_atoms,
            post_mark_equity_atoms: equity,
            maintenance_required_atoms: maintenance,
            maintenance_deficit_atoms: maintenance_deficit,
            uncovered_mark_loss_atoms: uncovered,
            risk,
        });
    }
    if net_open_interest != 0 {
        return Err(LiquidationAuditError::UnbalancedOpenInterest);
    }
    let mut candidates: Vec<_> = accounts
        .iter()
        .filter(|entry| entry.risk != MarginRisk::Healthy)
        .collect();
    candidates.sort_by(|a, b| {
        (b.risk == MarginRisk::Insolvent)
            .cmp(&(a.risk == MarginRisk::Insolvent))
            .then_with(|| {
                b.maintenance_deficit_atoms
                    .cmp(&a.maintenance_deficit_atoms)
            })
            .then_with(|| a.account.cmp(&b.account))
    });
    let candidates_by_severity = candidates
        .into_iter()
        .map(|entry| entry.account.clone())
        .collect();
    Ok(LiquidationRiskReport {
        accounts,
        candidates_by_severity,
        total_maintenance_deficit_atoms,
        total_uncovered_mark_loss_atoms,
        rounded_pnl_residual_atoms: mark.net_atoms,
    })
}

/// Source-informed, read-only comparison of possible loss waterfalls.
/// The legacy JavaScript loss-pool drain consumes available then margin;
/// margin-only reserve isolation would use posted margin first. Either
/// version uses a *hypothetical* observed insurance amount and does not
/// debit a Ledger/insurer, open an AMM trade or allocate ADL counterparty PnL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HypotheticalLossRule {
    LegacyAvailableThenMargin,
    PostedMarginOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LossBudgetPreview {
    pub demanded_atoms: u128,
    pub available_debit_atoms: u128,
    pub margin_debit_atoms: u128,
    pub hypothetical_insurance_debit_atoms: u128,
    pub unresolved_atoms: u128,
}
impl LossBudgetPreview {
    /// A pure arithmetic identity. This is NOT a proof that money exists.
    pub fn fully_accounted(self) -> bool {
        self.available_debit_atoms
            .checked_add(self.margin_debit_atoms)
            .and_then(|v| v.checked_add(self.hypothetical_insurance_debit_atoms))
            .and_then(|v| v.checked_add(self.unresolved_atoms))
            == Some(self.demanded_atoms)
    }
}

pub fn preview_loss_budget_unverified(
    demanded_atoms: u128,
    free_available_atoms: u128,
    posted_margin_atoms: u128,
    hypothetical_insurance_atoms: u128,
    rule: HypotheticalLossRule,
) -> LossBudgetPreview {
    let mut remainder = demanded_atoms;
    let from_available = if rule == HypotheticalLossRule::LegacyAvailableThenMargin {
        let debit = remainder.min(free_available_atoms);
        remainder -= debit;
        debit
    } else {
        0
    };
    let from_margin = remainder.min(posted_margin_atoms);
    remainder -= from_margin;
    let from_insurance = remainder.min(hypothetical_insurance_atoms);
    remainder -= from_insurance;
    LossBudgetPreview {
        demanded_atoms,
        available_debit_atoms: from_available,
        margin_debit_atoms: from_margin,
        hypothetical_insurance_debit_atoms: from_insurance,
        unresolved_atoms: remainder,
    }
}
