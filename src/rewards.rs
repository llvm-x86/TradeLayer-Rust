//! Deterministic maker-reward accounting for one epoch. Accounting only.
//!
//! THIS IS NOT CHAIN CONSENSUS. It is not wired to the matcher, mints or moves
//! no token, and produces no payout transaction. Inputs are declared by a
//! trusted caller and are not verified here. See docs/REWARDS.md for the rules,
//! the anti-sybil analysis, and the residual risks.
//!
//! Rules implemented:
//! - Only makers appear in the input; takers are never paid.
//! - A maker's score is its resting notional-seconds, but zero unless it also
//!   filled some notional and, when `require_attestation` is set, is attested.
//! - The fee-funded and emission-funded pools are allocated separately. Each is
//!   split pro-rata to score with floor division, capped per account at
//!   `max_account_share_bps` of that pool, and never redistributed.
//! - Each account's fee-funded payout is also capped at
//!   `fees_paid_atoms * fee_rebate_cap_bps / 10_000`.
//! - Floor remainders and capped excess stay in the pool as `retained_atoms`.
//!
//! Everything is integer arithmetic. Wide products use a 256-bit intermediate
//! built from std `u128` operations, so no input size can panic or silently wrap.

use crate::ledger::AccountId;

/// Basis-point denominator. Every `*_bps` parameter is an integer in `0..=10_000`.
pub const BPS_DENOMINATOR: u32 = 10_000;

/// Parameters that govern one epoch. Validated before any allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardParams {
    /// Portion of `fee_pool_atoms` allocated to rewards. Must be `<= 10_000`.
    pub fee_share_bps: u32,
    /// Per-account cap for each funded pool, in bps of that pool. Must be `1..=10_000`.
    pub max_account_share_bps: u32,
    /// Fee-funded payout per account is capped at `fees_paid * this / 10_000`.
    /// Must be `1..=10_000`, so the fee-funded payout never exceeds the fees paid.
    pub fee_rebate_cap_bps: u32,
    /// When true, unattested makers receive a zero score.
    pub require_attestation: bool,
}

impl RewardParams {
    pub fn validate(&self) -> Result<(), RewardError> {
        if self.fee_share_bps > BPS_DENOMINATOR {
            return Err(RewardError::InvalidFeeShareBps(self.fee_share_bps));
        }
        if self.max_account_share_bps == 0 || self.max_account_share_bps > BPS_DENOMINATOR {
            return Err(RewardError::InvalidAccountCapBps(
                self.max_account_share_bps,
            ));
        }
        if self.fee_rebate_cap_bps == 0 || self.fee_rebate_cap_bps > BPS_DENOMINATOR {
            return Err(RewardError::InvalidRebateCapBps(self.fee_rebate_cap_bps));
        }
        Ok(())
    }
}

/// Declared per-epoch statistics for one maker account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakerEpochStats {
    pub account: AccountId,
    /// Sum over time of the quote value of resting orders within `max_spread_bps`
    /// of the reference mid. Computed by the caller.
    pub resting_notional_seconds: u128,
    /// Notional this account's resting orders actually filled during the epoch.
    pub filled_notional_atoms: u128,
    /// Fees this account paid during the epoch.
    pub fees_paid_atoms: u64,
    /// Whether the account is attested. Derived by the caller from the registry.
    pub attested: bool,
}

/// Result of one epoch. `sum(payouts) + retained_atoms` equals the total budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardPlan {
    /// Nonzero payouts only, sorted by `AccountId`.
    pub payouts: Vec<(AccountId, u64)>,
    pub retained_atoms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewardError {
    InvalidFeeShareBps(u32),
    InvalidAccountCapBps(u32),
    InvalidRebateCapBps(u32),
    /// The same account appeared more than once. Duplicates would bypass the cap.
    DuplicateAccount(AccountId),
    BudgetOverflow,
    ArithmeticOverflow,
    /// Internal invariant failed. Returned as an error, never a panic.
    ConservationViolation,
}

impl core::fmt::Display for RewardError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for RewardError {}

/// Splits one epoch's reward budget among makers.
///
/// The budget is `floor(fee_pool_atoms * fee_share_bps / 10_000) + emission_budget_atoms`.
/// The caller is responsible for capping `emission_budget_atoms`.
pub fn plan_epoch_rewards(
    params: &RewardParams,
    fee_pool_atoms: u64,
    emission_budget_atoms: u64,
    makers: &[MakerEpochStats],
) -> Result<RewardPlan, RewardError> {
    params.validate()?;

    let fee_budget = to_u64(mul_div_floor(
        u128::from(fee_pool_atoms),
        u128::from(params.fee_share_bps),
        u128::from(BPS_DENOMINATOR),
    ))?;
    let budget = fee_budget
        .checked_add(emission_budget_atoms)
        .ok_or(RewardError::BudgetOverflow)?;

    // Sorting makes the result independent of input order and lets duplicates
    // be found by adjacency. No HashMap iteration is involved.
    let mut rows: Vec<&MakerEpochStats> = makers.iter().collect();
    rows.sort_by(|a, b| a.account.cmp(&b.account));
    if let Some(pair) = rows.windows(2).find(|w| w[0].account == w[1].account) {
        return Err(RewardError::DuplicateAccount(pair[0].account.clone()));
    }

    let mut scores = Vec::with_capacity(rows.len());
    let mut fee_limits = Vec::with_capacity(rows.len());
    let mut total_score: u128 = 0;
    for row in &rows {
        let eligible =
            row.filled_notional_atoms > 0 && (row.attested || !params.require_attestation);
        let score = if eligible {
            row.resting_notional_seconds
        } else {
            0
        };
        total_score = total_score
            .checked_add(score)
            .ok_or(RewardError::ArithmeticOverflow)?;
        scores.push(score);
        fee_limits.push(fee_rebate_limit(
            row.fees_paid_atoms,
            params.fee_rebate_cap_bps,
        )?);
    }

    let no_fee_limit = vec![u64::MAX; rows.len()];
    let fee_cap = pool_account_cap(fee_budget, params.max_account_share_bps)?;
    let emission_cap = pool_account_cap(emission_budget_atoms, params.max_account_share_bps)?;
    let (fee_payouts, fee_retained) =
        split_pool(fee_budget, fee_cap, &scores, total_score, &fee_limits)?;
    let (emission_payouts, emission_retained) = split_pool(
        emission_budget_atoms,
        emission_cap,
        &scores,
        total_score,
        &no_fee_limit,
    )?;

    let mut payouts = Vec::new();
    let mut distributed: u64 = 0;
    for (idx, row) in rows.iter().enumerate() {
        let total = fee_payouts[idx]
            .checked_add(emission_payouts[idx])
            .ok_or(RewardError::ArithmeticOverflow)?;
        distributed = distributed
            .checked_add(total)
            .ok_or(RewardError::ArithmeticOverflow)?;
        if total > 0 {
            payouts.push((row.account.clone(), total));
        }
    }
    let retained = fee_retained
        .checked_add(emission_retained)
        .ok_or(RewardError::ArithmeticOverflow)?;
    let accounted = distributed
        .checked_add(retained)
        .ok_or(RewardError::ArithmeticOverflow)?;
    if accounted != budget {
        return Err(RewardError::ConservationViolation);
    }
    Ok(RewardPlan {
        payouts,
        retained_atoms: retained,
    })
}

/// Allocates one pool. Each account gets `min(floor(pool * score / total), account_cap, limit)`.
/// Excess is not redistributed, so the returned retained amount is `pool - sum(payouts)`.
fn split_pool(
    pool_atoms: u64,
    account_cap_atoms: u64,
    scores: &[u128],
    total_score: u128,
    limits: &[u64],
) -> Result<(Vec<u64>, u64), RewardError> {
    let mut payouts = Vec::with_capacity(scores.len());
    let mut paid: u64 = 0;
    for (score, limit) in scores.iter().zip(limits) {
        let raw = if total_score == 0 || *score == 0 {
            0
        } else {
            to_u64(mul_div_floor(u128::from(pool_atoms), *score, total_score))?
        };
        let payout = raw.min(account_cap_atoms).min(*limit);
        paid = paid
            .checked_add(payout)
            .ok_or(RewardError::ArithmeticOverflow)?;
        payouts.push(payout);
    }
    let retained = pool_atoms
        .checked_sub(paid)
        .ok_or(RewardError::ConservationViolation)?;
    Ok((payouts, retained))
}

fn pool_account_cap(pool_atoms: u64, max_share_bps: u32) -> Result<u64, RewardError> {
    to_u64(mul_div_floor(
        u128::from(pool_atoms),
        u128::from(max_share_bps),
        u128::from(BPS_DENOMINATOR),
    ))
}

fn fee_rebate_limit(fees_paid_atoms: u64, rebate_cap_bps: u32) -> Result<u64, RewardError> {
    to_u64(mul_div_floor(
        u128::from(fees_paid_atoms),
        u128::from(rebate_cap_bps),
        u128::from(BPS_DENOMINATOR),
    ))
}

fn to_u64(value: Option<u128>) -> Result<u64, RewardError> {
    value
        .and_then(|v| u64::try_from(v).ok())
        .ok_or(RewardError::ArithmeticOverflow)
}

const LOW_64_BITS: u128 = u64::MAX as u128;

/// Full 256-bit product of two `u128`s, returned as `(high, low)`.
fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    let (a0, a1) = (a & LOW_64_BITS, a >> 64);
    let (b0, b1) = (b & LOW_64_BITS, b >> 64);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & LOW_64_BITS) + (p10 & LOW_64_BITS);
    let low = (p00 & LOW_64_BITS) | ((mid & LOW_64_BITS) << 64);
    let high = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    (high, low)
}

/// `floor(a * b / d)`, exact for any `u128` inputs. Returns `None` when `d == 0`
/// or when the quotient does not fit in `u128`.
fn mul_div_floor(a: u128, b: u128, d: u128) -> Option<u128> {
    if d == 0 {
        return None;
    }
    let (high, low) = mul_wide(a, b);
    if high == 0 {
        return Some(low / d);
    }
    if high >= d {
        return None;
    }
    // Restoring long division of the 256-bit numerator (high, low) by d.
    // Invariant: rem < d. The quotient fits in u128 because high < d.
    let mut rem = high;
    let mut quo: u128 = 0;
    for shift in (0..128).rev() {
        let bit = (low >> shift) & 1;
        let carry = rem >> 127;
        rem = (rem << 1) | bit;
        quo <<= 1;
        if carry == 1 || rem >= d {
            // When carry is set, the true value is 2^128 + rem and wrapping_sub
            // yields the correct result because the true difference is below d.
            rem = rem.wrapping_sub(d);
            quo |= 1;
        }
    }
    Some(quo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> AccountId {
        AccountId::try_from(name).unwrap()
    }

    fn maker(
        name: &str,
        resting: u128,
        filled: u128,
        fees: u64,
        attested: bool,
    ) -> MakerEpochStats {
        MakerEpochStats {
            account: id(name),
            resting_notional_seconds: resting,
            filled_notional_atoms: filled,
            fees_paid_atoms: fees,
            attested,
        }
    }

    fn params() -> RewardParams {
        RewardParams {
            fee_share_bps: 0,
            max_account_share_bps: BPS_DENOMINATOR,
            fee_rebate_cap_bps: BPS_DENOMINATOR,
            require_attestation: false,
        }
    }

    #[test]
    fn mul_div_matches_native_arithmetic_when_product_fits() {
        let cases: [(u128, u128, u128); 6] = [
            (0, 5, 3),
            (7, 0, 3),
            (10, 3, 6),
            (1, 1, 1),
            (u64::MAX as u128, 1 << 40, 1 << 50),
            ((1 << 63) + 9, 1 << 30, 12_345),
        ];
        for (a, b, d) in cases {
            assert_eq!(mul_div_floor(a, b, d), Some(a * b / d), "{a} {b} {d}");
        }
    }

    #[test]
    fn mul_div_wide_path_is_exact() {
        // (2^128 - 1) * (2^64) / (2^128 - 1) must equal 2^64 even though the
        // product needs 192 bits.
        assert_eq!(
            mul_div_floor(u128::MAX, 1u128 << 64, u128::MAX),
            Some(1u128 << 64)
        );
        // Quotient is still exact when the numerator overflows u128.
        let a = u128::MAX / 3;
        let b = 1u128 << 70;
        let d = 1u128 << 100;
        // a * 2^70 / 2^100 == a / 2^30 (floor).
        assert_eq!(mul_div_floor(a, b, d), Some(a >> 30));
    }

    #[test]
    fn mul_div_rejects_zero_divisor_and_unrepresentable_quotient() {
        assert_eq!(mul_div_floor(1, 1, 0), None);
        assert_eq!(mul_div_floor(u128::MAX, u128::MAX, 1), None);
    }

    #[test]
    fn mul_div_reconstructs_dividend_for_pseudo_random_inputs() {
        let mut state: u128 = 0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C834;
        let mut next = || {
            state = state
                .wrapping_mul(0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645)
                .wrapping_add(0x5851_F42D_4C95_7F2D_1405_7B7E_F767_814F);
            state
        };
        for _ in 0..2_000 {
            let a = next();
            let b = next() >> (next() % 127);
            let d = (next() >> (next() % 127)).max(1);
            let Some(q) = mul_div_floor(a, b, d) else {
                continue;
            };
            // q * d <= a * b < (q + 1) * d, checked on the 256-bit products.
            let (qh, ql) = mul_wide(q, d);
            let (ah, al) = mul_wide(a, b);
            assert!((qh, ql) <= (ah, al));
            // The true remainder a*b - q*d lies in [0, d), so it fits in the low
            // 128 bits and the wrapping difference of the low halves equals it.
            let r = al.wrapping_sub(ql);
            assert!(r < d, "remainder must be below divisor");
        }
    }

    #[test]
    fn validation_rejects_degenerate_parameters() {
        let mut p = params();
        p.fee_share_bps = BPS_DENOMINATOR + 1;
        assert_eq!(p.validate(), Err(RewardError::InvalidFeeShareBps(10_001)));
        let mut p = params();
        p.max_account_share_bps = 0;
        assert_eq!(p.validate(), Err(RewardError::InvalidAccountCapBps(0)));
        let mut p = params();
        p.max_account_share_bps = BPS_DENOMINATOR + 1;
        assert!(matches!(
            p.validate(),
            Err(RewardError::InvalidAccountCapBps(_))
        ));
        let mut p = params();
        p.fee_rebate_cap_bps = 0;
        assert_eq!(p.validate(), Err(RewardError::InvalidRebateCapBps(0)));
        let mut p = params();
        p.fee_rebate_cap_bps = BPS_DENOMINATOR + 1;
        assert!(matches!(
            p.validate(),
            Err(RewardError::InvalidRebateCapBps(_))
        ));
        assert_eq!(params().validate(), Ok(()));
    }

    #[test]
    fn unattested_maker_scores_zero_only_when_required() {
        let stats = [maker("a", 10, 1, 0, false), maker("b", 10, 1, 0, true)];
        let mut p = params();
        p.require_attestation = true;
        let plan = plan_epoch_rewards(&p, 0, 100, &stats).unwrap();
        assert_eq!(plan.payouts, vec![(id("b"), 100)]);
        p.require_attestation = false;
        let plan = plan_epoch_rewards(&p, 0, 100, &stats).unwrap();
        assert_eq!(plan.payouts, vec![(id("a"), 50), (id("b"), 50)]);
    }

    #[test]
    fn duplicate_account_is_rejected() {
        let stats = [maker("a", 1, 1, 0, true), maker("a", 1, 1, 0, true)];
        assert_eq!(
            plan_epoch_rewards(&params(), 0, 10, &stats),
            Err(RewardError::DuplicateAccount(id("a")))
        );
    }
}
