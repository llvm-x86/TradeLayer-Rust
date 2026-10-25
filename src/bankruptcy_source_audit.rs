//! Stage25F read-only characterization of historical JS bankruptcy threshold.
//!
//! Source: tradelayer.js@dca0888b src/clearing.js:2232-2288
//! (computeLiquidationPriceFromLoss).
//!
//! This is a bounded EXACT-RATIONAL reconstruction of its algebra, not
//! JavaScript BigNumber 20-decimal reciprocal / final 30dp numerical parity.
//! The threshold is never a verified oracle, market fill, clearance right,
//! signed channel update, insurance claim or trustable settlement price.
//! No connection to JointChainState, ClearingSandbox, TLCL/TLCC or E2B.
//! The user-supplied inputs are untrusted synthetic observations.

use core::cmp::Ordering;

/// Units: all prices and notional in decimal 1e8, equity in collateral atoms.
const ONE: u128 = 100_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankruptcyInputs {
    pub last_mark_e8: u64,
    pub equity_atoms: u128,
    pub signed_contracts: i64,
    pub notional_e8: u64,
    pub inverse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankruptcyAuditError {
    MissingMark,
    MissingNotional,
    NonpositiveBoundary,
    NonpositiveInverseReciprocal,
    ArithmeticOverflow,
}

/// An exact positive rational bankruptcy threshold in price-e8 units.
/// NEVER round this to a tradable tick without an independently approved
/// rounding rule. The historical JS rounds its output to 30 decimal places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankruptcyFraction {
    pub numerator_price_e8: u128,
    pub denominator: u128,
}

impl BankruptcyFraction {
    /// A comparison, not a safe-price rule or matching authorization.
    /// A quote is a caller-declared synthetic value; it need not be firm.
    pub fn compare_declared_quote_e8(
        self,
        quote_e8: u64,
    ) -> Result<Ordering, BankruptcyAuditError> {
        if quote_e8 == 0 {
            return Err(BankruptcyAuditError::MissingMark);
        }
        let left = u128::from(quote_e8)
            .checked_mul(self.denominator)
            .ok_or(BankruptcyAuditError::ArithmeticOverflow)?;
        Ok(left.cmp(&self.numerator_price_e8))
    }

    pub fn floor_e8(self) -> u128 {
        self.numerator_price_e8 / self.denominator
    }
    pub fn is_exact_e8(self) -> bool {
        self.numerator_price_e8.is_multiple_of(self.denominator)
    }
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a
}

fn fraction(n: u128, d: u128) -> Result<BankruptcyFraction, BankruptcyAuditError> {
    if n == 0 || d == 0 {
        return Err(BankruptcyAuditError::NonpositiveBoundary);
    }
    let divisor = gcd(n, d);
    Ok(BankruptcyFraction {
        numerator_price_e8: n / divisor,
        denominator: d / divisor,
    })
}

/// Historical algebra without the source's intermediate decimal rounding:
///
/// linear long: last - equity / (contracts * notional)
/// linear short: last + equity / (abs(contracts) * notional)
///
/// inverse long: 1 / (1/last + equity / (contracts * notional))
/// inverse short: 1 / (1/last - equity / (abs(contracts) * notional))
///
/// The original JS returns the last price if `contracts==0 || equity<=0`.
/// This integer-input model covers equity==0 only; negative equity is outside
/// its caller type. Where historical arithmetic gives a nonpositive price
/// or a nonpositive inverse reciprocal, fail closed rather than fabricate
/// an executable threshold or price tick.
pub fn characterize_bankruptcy_fraction_unverified(
    input: BankruptcyInputs,
) -> Result<BankruptcyFraction, BankruptcyAuditError> {
    if input.last_mark_e8 == 0 {
        return Err(BankruptcyAuditError::MissingMark);
    }
    if input.signed_contracts == 0 || input.equity_atoms == 0 {
        return fraction(u128::from(input.last_mark_e8), 1);
    }
    if input.notional_e8 == 0 {
        return Err(BankruptcyAuditError::MissingNotional);
    }

    let last = u128::from(input.last_mark_e8);
    let contracts = u128::from(input.signed_contracts.unsigned_abs());
    let notional = u128::from(input.notional_e8);
    let qn = contracts
        .checked_mul(notional)
        .ok_or(BankruptcyAuditError::ArithmeticOverflow)?;
    if !input.inverse {
        let price_times_size = last
            .checked_mul(qn)
            .ok_or(BankruptcyAuditError::ArithmeticOverflow)?;
        let eq_price = input.equity_atoms
            .checked_mul(ONE)
            .ok_or(BankruptcyAuditError::ArithmeticOverflow)?;
        let numerator = if input.signed_contracts > 0 {
            price_times_size
                .checked_sub(eq_price)
                .ok_or(BankruptcyAuditError::NonpositiveBoundary)?
        } else {
            price_times_size
                .checked_add(eq_price)
                .ok_or(BankruptcyAuditError::ArithmeticOverflow)?
        };
        fraction(numerator, qn)
    } else {
        let eq_last = input.equity_atoms
            .checked_mul(last)
            .ok_or(BankruptcyAuditError::ArithmeticOverflow)?;
        let scaled = ONE
            .checked_mul(qn)
            .ok_or(BankruptcyAuditError::ArithmeticOverflow)?;
        // Currency units: price_e8 =
        // ONE * last_e8 * (q * notional_e8)
        // / (ONE * (q * notional_e8) ± equity_atoms * last_e8).
        let numerator = scaled
            .checked_mul(last)
            .ok_or(BankruptcyAuditError::ArithmeticOverflow)?;
        let denominator = if input.signed_contracts > 0 {
            scaled
                .checked_add(eq_last)
                .ok_or(BankruptcyAuditError::ArithmeticOverflow)?
        } else {
            scaled
                .checked_sub(eq_last)
                .ok_or(BankruptcyAuditError::NonpositiveInverseReciprocal)?
        };
        if denominator == 0 {
            return Err(BankruptcyAuditError::NonpositiveInverseReciprocal);
        }
        fraction(numerator, denominator)
    }
}
