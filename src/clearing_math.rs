//! Stage 24A: read-only accounting formulas extracted from legacy clearing.js.
//!
//! Implements a bounded INTEGER-input subset of three pure calculations:
//! (1) loss coverage with half-atom precision;
//! (2) 5-basis-point funding deadband;
//! (3) linear/inverse mark-to-mark PnL with either pinned BigNumber 20dp
//!     intermediate division or exact rational intermediate division.
//!
//! The legacy JavaScript uses decimal BigNumber, and may accept fractional
//! contract counts, dynamically configured precision, or non-atomic prices.
//! Those cases are outside this integer-input research slice. No state,
//! ownership, margin debits, funding transfers, or settlement authorization.
//! This module is NOT connected to JointChainState or Ledger.

pub const ATOMS_PER_UNIT: u128 = 100_000_000;
const BIGNUMBER_DIVISION_SCALE: u128 = 100_000_000_000_000_000_000;
const RECIPROCAL_NUMERATOR: u128 = ATOMS_PER_UNIT * BIGNUMBER_DIVISION_SCALE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearingMathError {
    ArithmeticOverflow,
}

impl core::fmt::Display for ClearingMathError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ClearingMathError {}

/// A report in HALF atomic units. The old maintenance base is margin / 2,
/// which can land between atomic units; never truncate that fraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LossCoverage {
    pub maintenance_twice_atoms: u128,
    pub coverage_twice_atoms: i128,
}

impl LossCoverage {
    /// The requested loss is an exact, nonnegative whole-atom amount.
    pub fn covers(self, loss_atoms: u64) -> bool {
        self.coverage_twice_atoms >= i128::from(loss_atoms) * 2
    }
}

/// From clearing.js::computeLossCoverage:
/// available + margin/2 + option premium MTM - naked option maintenance.
/// Inputs are caller-provided offline observations, not checked balances.
pub fn compute_loss_coverage(
    available_atoms: i64,
    margin_atoms: u64,
    premium_mtm_atoms: i64,
    naked_maintenance_atoms: u64,
) -> Result<LossCoverage, ClearingMathError> {
    let coverage = i128::from(available_atoms)
        .checked_mul(2)
        .and_then(|v| v.checked_add(i128::from(margin_atoms)))
        .and_then(|v| v.checked_add(i128::from(premium_mtm_atoms) * 2))
        .and_then(|v| v.checked_sub(i128::from(naked_maintenance_atoms) * 2))
        .ok_or(ClearingMathError::ArithmeticOverflow)?;
    Ok(LossCoverage {
        maintenance_twice_atoms: u128::from(margin_atoms),
        coverage_twice_atoms: coverage,
    })
}

/// The integer-bps subset of clearing.js::clampFundingRate:
/// suppress abs(rate) < 5 and reduce larger magnitudes by 5.
/// The JS method accepts fractional basis points, which this API excludes.
pub const fn clamp_funding_bps(bps: i64) -> i64 {
    if bps > 5 {
        bps - 5
    } else if bps < -5 {
        bps + 5
    } else {
        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PnlRounding {
    /// clearing.js::calculateMarkToMarkPNL with the DEFAULT BigNumber.js
    /// DECIMAL_PLACES = 20 for each separate division 1/from and 1/to.
    /// The final decimalPlaces(8) uses ROUND_HALF_UP.
    Legacy20DecimalPlaces,
    /// Rational arithmetic until the final rounding to 8 decimal places.
    /// This is a candidate precise model, not a historical JS claim.
    ExactRational,
}

fn multiply_checked(values: &[u128]) -> Result<u128, ClearingMathError> {
    values
        .iter()
        .try_fold(1u128, |acc, n| acc.checked_mul(*n))
        .ok_or(ClearingMathError::ArithmeticOverflow)
}

fn ratio_round_half_up(numerator: u128, denominator: u128) -> Result<u128, ClearingMathError> {
    if denominator == 0 {
        return Err(ClearingMathError::ArithmeticOverflow);
    }
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder >= denominator - remainder {
        quotient
            .checked_add(1)
            .ok_or(ClearingMathError::ArithmeticOverflow)
    } else {
        Ok(quotient)
    }
}

fn signed_amount(magnitude: u128, negative: bool) -> Result<i128, ClearingMathError> {
    let number = i128::try_from(magnitude).map_err(|_| ClearingMathError::ArithmeticOverflow)?;
    Ok(if negative { -number } else { number })
}

/// Offline mark-to-market PnL from clearing.js::calculateMarkToMarkPNL.
///
/// `from_price_e8`, `to_price_e8`, and `notional_e8` represent prices and
/// notional decimal quantities scaled by 1e8. Contract size is signed integer.
/// Output is in 1e-8 collateral units. A ZERO price yields ZERO, as in JS.
/// A ZERO notional defaults to ONE unit, as `notional || 1` does in JS.
///
/// The exact-rational and 20dp models intentionally have separate modes:
/// intermediate reciprocal rounding can change final atoms. In both modes,
/// the last operation rounds halfway away from zero.
pub fn mark_to_mark_pnl_atoms(
    signed_contracts: i64,
    from_price_e8: u64,
    to_price_e8: u64,
    notional_e8: u64,
    inverse: bool,
    rounding: PnlRounding,
) -> Result<i128, ClearingMathError> {
    if from_price_e8 == 0 || to_price_e8 == 0 || signed_contracts == 0 {
        return Ok(0);
    }
    let notional = u128::from(if notional_e8 == 0 {
        ATOMS_PER_UNIT as u64
    } else {
        notional_e8
    });
    let contracts = u128::from(signed_contracts.unsigned_abs());
    let delta = i128::from(to_price_e8) - i128::from(from_price_e8);
    let negative = (delta < 0) != (signed_contracts < 0);

    let magnitude = if !inverse {
        let numerator = multiply_checked(&[delta.unsigned_abs(), contracts, notional])?;
        ratio_round_half_up(numerator, ATOMS_PER_UNIT)?
    } else {
        match rounding {
            PnlRounding::ExactRational => {
                let numerator =
                    multiply_checked(&[delta.unsigned_abs(), contracts, notional, ATOMS_PER_UNIT])?;
                let denominator =
                    multiply_checked(&[u128::from(from_price_e8), u128::from(to_price_e8)])?;
                ratio_round_half_up(numerator, denominator)?
            }
            PnlRounding::Legacy20DecimalPlaces => {
                // BigNumber performs two separate 1 / price calculations,
                // rounded to 20 decimal places, before it subtracts them.
                let from_reciprocal = ratio_round_half_up(
                    RECIPROCAL_NUMERATOR,
                    u128::from(from_price_e8),
                )?;
                let to_reciprocal =
                    ratio_round_half_up(RECIPROCAL_NUMERATOR, u128::from(to_price_e8))?;
                let numerator = multiply_checked(&[
                    from_reciprocal.abs_diff(to_reciprocal),
                    contracts,
                    notional,
                ])?;
                ratio_round_half_up(numerator, BIGNUMBER_DIVISION_SCALE)?
            }
        }
    };
    signed_amount(magnitude, negative)
}
