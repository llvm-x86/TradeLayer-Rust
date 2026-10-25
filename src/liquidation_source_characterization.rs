//! Read-only Stage25F characterization of pinned legacy liquidation inputs.
//!
//! Source: tradelayer.js@dca0888b, clearing.js, orderbook.js, marginMap.js.
//! NO custody, signed-order authority, oracle authentication, whitelist
//! attestation, market matching, insurance or bankruptcy settlement here.
//! This deliberately has no entry point into JointChainState or the E2B
//! whole-book closeout. Each quote is a TEST FIXTURE, never proven liquidity.

use crate::clearing_math::{mark_to_mark_pnl_atoms, ClearingMathError, PnlRounding};
use std::collections::BTreeSet;

pub const SOURCE_FUNDING_LOOKBACK_BLOCKS: u64 = 192;
pub const MAX_DIAGNOSTIC_BOOK_LEVELS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceContractKind {
    Native,
    Oracle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkRoute {
    NativeVolumeIndexBlockPrice,
    OraclePriceEventAtBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundingReferenceRoute {
    NativeVolumeWeightedVwap,
    OracleEqualWeightTwap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePriceRoutes {
    /// clearing.js::isPriceUpdatedForBlockHeight. Not a signed proof.
    pub mark: MarkRoute,
    /// clearing.js::calculateFundingRate, separate from mark update.
    pub funding_reference: FundingReferenceRoute,
    pub funding_lookback_blocks: u64,
}

pub const fn characterize_price_routes(kind: SourceContractKind) -> SourcePriceRoutes {
    match kind {
        SourceContractKind::Native => SourcePriceRoutes {
            mark: MarkRoute::NativeVolumeIndexBlockPrice,
            funding_reference: FundingReferenceRoute::NativeVolumeWeightedVwap,
            funding_lookback_blocks: SOURCE_FUNDING_LOOKBACK_BLOCKS,
        },
        SourceContractKind::Oracle => SourcePriceRoutes {
            mark: MarkRoute::OraclePriceEventAtBlock,
            funding_reference: FundingReferenceRoute::OracleEqualWeightTwap,
            funding_lookback_blocks: SOURCE_FUNDING_LOOKBACK_BLOCKS,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidationSide {
    /// Closing a long: sell liquidation into bids.
    SellLong,
    /// Closing a short: buy liquidation against asks.
    BuyShort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticLevelOrigin {
    Amm,
    OrdinaryOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntheticDepthLevel {
    /// Research-only distinct ID, not a txid/proof.
    pub id: u64,
    pub origin: SyntheticLevelOrigin,
    pub price_e8: u64,
    pub quantity_contracts: u64,
    pub block_time: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EstimatedAmmTake {
    pub level_id: u64,
    pub quantity_contracts: u64,
    pub declared_price_e8: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmmFirstEstimate {
    pub estimated_amm_contracts: u64,
    pub unmet_contracts: u64,
    pub attempted_levels: Vec<EstimatedAmmTake>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAuditError {
    MissingPrice,
    ZeroQuantity,
    TooManyLevels,
    DuplicateLevelId,
    ArithmeticOverflow,
    ClearingMath(ClearingMathError),
}
impl From<ClearingMathError> for SourceAuditError {
    fn from(err: ClearingMathError) -> Self {
        Self::ClearingMath(err)
    }
}

/// Diagnostic equivalent of clearing.js::_sweepAmmLiquidityFirst's safe test:
///
/// linear: SELL at/above bankruptcy boundary, BUY at/below boundary.
/// inverse: SELL at/below boundary, BUY at/above boundary.
///
/// This does not authorize a quote, select an unwilling counterparty or
/// implement the later ordinary orderbook sweep.
pub const fn historical_safe_price(
    side: LiquidationSide,
    inverse: bool,
    level_price_e8: u64,
    boundary_e8: u64,
) -> bool {
    if level_price_e8 == 0 || boundary_e8 == 0 {
        return false;
    }
    match (side, inverse) {
        (LiquidationSide::SellLong, false) => level_price_e8 >= boundary_e8,
        (LiquidationSide::BuyShort, false) => level_price_e8 <= boundary_e8,
        (LiquidationSide::SellLong, true) => level_price_e8 <= boundary_e8,
        (LiquidationSide::BuyShort, true) => level_price_e8 >= boundary_e8,
    }
}

fn price_time_cmp(
    left: &SyntheticDepthLevel,
    right: &SyntheticDepthLevel,
    side: LiquidationSide,
) -> std::cmp::Ordering {
    let price_order = match side {
        LiquidationSide::SellLong => right.price_e8.cmp(&left.price_e8),
        LiquidationSide::BuyShort => left.price_e8.cmp(&right.price_e8),
    };
    price_order.then_with(|| left.block_time.cmp(&right.block_time))
}

/// Source-level AMM-first priority as a bounded, pure, test-only estimate.
/// It explicitly IGNOREs ordinary orders rather than synthesizing a fill.
/// Time ties retain source input order, matching JS Array.sort's stability.
/// The input is never modified. All amounts are whole contracts; the
/// historical JS supports fractional amounts, excluded from this slice.
pub fn estimate_amm_first_synthetic(
    levels: &[SyntheticDepthLevel],
    side: LiquidationSide,
    inverse: bool,
    bankruptcy_boundary_e8: u64,
    max_requested_contracts: u64,
) -> Result<AmmFirstEstimate, SourceAuditError> {
    if bankruptcy_boundary_e8 == 0 {
        return Err(SourceAuditError::MissingPrice);
    }
    if max_requested_contracts == 0 {
        return Err(SourceAuditError::ZeroQuantity);
    }
    if levels.len() > MAX_DIAGNOSTIC_BOOK_LEVELS {
        return Err(SourceAuditError::TooManyLevels);
    }
    let mut seen = BTreeSet::new();
    for level in levels {
        if !seen.insert(level.id) {
            return Err(SourceAuditError::DuplicateLevelId);
        }
    }
    let mut amm_levels: Vec<(usize, &SyntheticDepthLevel)> = levels
        .iter()
        .enumerate()
        .filter(|(_, level)| {
            level.origin == SyntheticLevelOrigin::Amm
                && level.price_e8 > 0
                && level.quantity_contracts > 0
        })
        .collect();
    amm_levels.sort_by(|(_, a), (_, b)| price_time_cmp(a, b, side));
    let mut remaining = max_requested_contracts;
    let mut attempted = Vec::new();
    for (_, level) in amm_levels {
        if remaining == 0 {
            break;
        }
        if !historical_safe_price(side, inverse, level.price_e8, bankruptcy_boundary_e8) {
            continue;
        }
        let quantity = remaining.min(level.quantity_contracts);
        if quantity > 0 {
            attempted.push(EstimatedAmmTake {
                level_id: level.id,
                quantity_contracts: quantity,
                declared_price_e8: level.price_e8,
            });
            remaining -= quantity;
        }
    }
    Ok(AmmFirstEstimate {
        estimated_amm_contracts: max_requested_contracts - remaining,
        unmet_contracts: remaining,
        attempted_levels: attempted,
    })
}

/// A sort-only characterization of the ordinary maker price/time priority.
/// Never infers an executable match, fee, role, authorization or transfer.
pub fn synthetic_order_priority(
    levels: &[SyntheticDepthLevel],
    side: LiquidationSide,
) -> Result<Vec<u64>, SourceAuditError> {
    if levels.len() > MAX_DIAGNOSTIC_BOOK_LEVELS {
        return Err(SourceAuditError::TooManyLevels);
    }
    let mut seen = BTreeSet::new();
    if levels.iter().any(|level| !seen.insert(level.id)) {
        return Err(SourceAuditError::DuplicateLevelId);
    }
    let mut selected: Vec<_> = levels
        .iter()
        .filter(|level| level.origin == SyntheticLevelOrigin::OrdinaryOrder)
        .collect();
    selected.sort_by(|a, b| price_time_cmp(a, b, side));
    Ok(selected.into_iter().map(|level| level.id).collect())
}

/// Weighted-average entry price for an INCREASING same-direction position.
/// Pinned marginMap.js::updateAveragePrice: eight-decimal HALF_UP result.
/// Reductions retain the old average (caller must not call this method).
/// No per-lot cost-basis data is assumed. Contracts are whole quantities.
pub fn characterize_weighted_avg_entry_e8(
    old_qty: u64,
    old_avg_e8: u64,
    added_qty: u64,
    added_price_e8: u64,
) -> Result<u64, SourceAuditError> {
    if old_qty == 0 || added_qty == 0 {
        return Err(SourceAuditError::ZeroQuantity);
    }
    if old_avg_e8 == 0 || added_price_e8 == 0 {
        return Err(SourceAuditError::MissingPrice);
    }
    let total_qty = u128::from(old_qty)
        .checked_add(u128::from(added_qty))
        .ok_or(SourceAuditError::ArithmeticOverflow)?;
    let old_cost = u128::from(old_qty)
        .checked_mul(u128::from(old_avg_e8))
        .ok_or(SourceAuditError::ArithmeticOverflow)?;
    let added_cost = u128::from(added_qty)
        .checked_mul(u128::from(added_price_e8))
        .ok_or(SourceAuditError::ArithmeticOverflow)?;
    let sum = old_cost
        .checked_add(added_cost)
        .ok_or(SourceAuditError::ArithmeticOverflow)?;
    let q = sum / total_qty;
    let rem = sum % total_qty;
    let rounded = if rem >= total_qty - rem {
        q.checked_add(1).ok_or(SourceAuditError::ArithmeticOverflow)?
    } else {
        q
    };
    u64::try_from(rounded).map_err(|_| SourceAuditError::ArithmeticOverflow)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoricalPnlComparison {
    /// Accounting/realized formula: avgEntry -> execution, in atoms.
    pub avg_entry_accounting_pnl_atoms: i128,
    /// Incremental mark-to-mark settlement: lastMark -> execution, in atoms.
    pub incremental_mark_pnl_atoms: i128,
    /// Pinned marginMap.settlePNL linear branch omits notional multiplier.
    /// Characterization ONLY. For inverse, this is None.
    pub js_linear_settle_pnl_atoms: Option<i128>,
}

/// Pinned source has TWO distinct calculations, NOT per-lot bookkeeping:
/// marginMap.realizePnl uses weighted avg entry for accounting PnL;
/// marginMap.settlePNL uses lastMark (fallback avg) for settlement PnL.
/// Historic marginMap.settlePNL's linear branch has a notional omission.
/// The Stage25F synthetic test model currently uses incremental mark math,
/// which MUST NOT be presented as avg-entry realized accounting parity.
pub fn compare_avg_accounting_to_mark_pnl(
    signed_contracts: i64,
    avg_entry_e8: u64,
    last_mark_e8: u64,
    execution_e8: u64,
    notional_e8: u64,
    inverse: bool,
    rounding: PnlRounding,
) -> Result<HistoricalPnlComparison, SourceAuditError> {
    if signed_contracts == 0 {
        return Err(SourceAuditError::ZeroQuantity);
    }
    if [avg_entry_e8, last_mark_e8, execution_e8, notional_e8]
        .contains(&0)
    {
        return Err(SourceAuditError::MissingPrice);
    }
    let avg_entry_accounting_pnl_atoms = mark_to_mark_pnl_atoms(
        signed_contracts, avg_entry_e8, execution_e8, notional_e8, inverse, rounding,
    )?;
    let incremental_mark_pnl_atoms = mark_to_mark_pnl_atoms(
        signed_contracts, last_mark_e8, execution_e8, notional_e8, inverse, rounding,
    )?;
    let js_linear_settle_pnl_atoms = if inverse {
        None
    } else {
        let delta = i128::from(execution_e8) - i128::from(last_mark_e8);
        Some(delta
            .checked_mul(i128::from(signed_contracts))
            .ok_or(SourceAuditError::ArithmeticOverflow)?)
    };
    Ok(HistoricalPnlComparison {
        avg_entry_accounting_pnl_atoms,
        incremental_mark_pnl_atoms,
        js_linear_settle_pnl_atoms,
    })
}
