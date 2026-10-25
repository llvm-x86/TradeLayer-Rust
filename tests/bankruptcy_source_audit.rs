//! Stage25F pinned-source bankruptcy PRICE ONLY. Never a market fill.
use std::cmp::Ordering;
use tradelayer_core::bankruptcy_source_audit::{
    characterize_bankruptcy_fraction_unverified as threshold, BankruptcyAuditError as Error,
    BankruptcyInputs as Input,
};

const ONE: u64 = 100_000_000;

fn input(last: u64, equity: u128, contracts: i64, notional: u64, inverse: bool) -> Input {
    Input {
        last_mark_e8: last,
        equity_atoms: equity,
        signed_contracts: contracts,
        notional_e8: notional,
        inverse,
    }
}

#[test]
fn zero_equity_and_zero_contracts_return_last_mark_like_pinned_js() {
    for signed in [1, -1, 0] {
        let result = threshold(input(100 * ONE, 0, signed, ONE, true)).unwrap();
        assert_eq!(result.floor_e8(), 100 * u128::from(ONE));
        assert!(result.is_exact_e8());
    }
    assert_eq!(
        threshold(input(100 * ONE, u128::from(ONE), 0, 0, false))
            .unwrap()
            .floor_e8(),
        100 * u128::from(ONE)
    );
}

#[test]
fn linear_long_and_short_boundaries_use_signed_quantity_and_notional() {
    let q = u128::from(2 * ONE);
    // At L=100, equity=20, size=2, notional=2:
    // long threshold 95; short threshold 105.
    let long = threshold(input(100 * ONE, 20 * q / 2, 2, 2 * ONE, false)).unwrap();
    let short = threshold(input(100 * ONE, 20 * q / 2, -2, 2 * ONE, false)).unwrap();
    assert_eq!(long.floor_e8(), 95 * u128::from(ONE));
    assert_eq!(short.floor_e8(), 105 * u128::from(ONE));
    assert!(long.is_exact_e8());
    assert_eq!(
        long.compare_declared_quote_e8(95 * ONE),
        Ok(Ordering::Equal)
    );
    assert_eq!(
        short.compare_declared_quote_e8(105 * ONE),
        Ok(Ordering::Equal)
    );
}

#[test]
fn inverse_long_and_short_use_historical_reciprocal_formula() {
    // q=1, notional=100, mark=100. The source's inverse long at
    // equity=1 reaches bankruptcy at price=50; inverse short at
    // equity=0.5 reaches bankruptcy at price=200.
    let long = threshold(input(100 * ONE, u128::from(ONE), 1, 100 * ONE, true)).unwrap();
    let short = threshold(input(100 * ONE, u128::from(ONE / 2), -1, 100 * ONE, true)).unwrap();
    assert_eq!(long.floor_e8(), 50 * u128::from(ONE));
    assert_eq!(short.floor_e8(), 200 * u128::from(ONE));
    assert_eq!(long.compare_declared_quote_e8(49 * ONE), Ok(Ordering::Less));
    assert_eq!(
        short.compare_declared_quote_e8(201 * ONE),
        Ok(Ordering::Greater)
    );
}

#[test]
fn nonintegral_sat_boundary_remains_exact_unrounded_fraction() {
    // Long: mark=100, equity=1, three contracts of notional 1.
    // Bankruptcy = 99 + 2/3; the neighboring 1e-8 quotes straddle it.
    let boundary = threshold(input(100 * ONE, u128::from(ONE), 3, ONE, false)).unwrap();
    assert_eq!(boundary.floor_e8(), 99 * u128::from(ONE) + 66_666_666);
    assert!(!boundary.is_exact_e8());
    assert_eq!(
        boundary.compare_declared_quote_e8(99 * ONE + 66_666_666),
        Ok(Ordering::Less)
    );
    assert_eq!(
        boundary.compare_declared_quote_e8(99 * ONE + 66_666_667),
        Ok(Ordering::Greater)
    );
    // This is NOT a claim that JS BigNumber's 30dp round matches a
    // particular executable 8dp price-tick or maker boundary.
}

#[test]
fn nonpositive_or_divergent_thresholds_are_unusable() {
    assert_eq!(
        threshold(input(100 * ONE, u128::from(100 * ONE), 1, ONE, false)),
        Err(Error::NonpositiveBoundary)
    );
    assert_eq!(
        threshold(input(100 * ONE, u128::from(ONE), -1, 100 * ONE, true)),
        Err(Error::NonpositiveInverseReciprocal)
    );
    assert_eq!(
        threshold(input(0, 1, 1, ONE, false)),
        Err(Error::MissingMark)
    );
    assert_eq!(
        threshold(input(100 * ONE, 1, 1, 0, true)),
        Err(Error::MissingNotional)
    );
    let long = threshold(input(100 * ONE, 1, 1, ONE, false)).unwrap();
    assert_eq!(long.compare_declared_quote_e8(0), Err(Error::MissingMark));
}

#[test]
fn extreme_inputs_fail_closed_without_wraparound_or_network_lookup() {
    let maximum = input(u64::MAX, u128::MAX, i64::MAX, u64::MAX, false);
    assert_eq!(threshold(maximum), Err(Error::ArithmeticOverflow));
    let max_inverse = input(u64::MAX, 1, i64::MAX, u64::MAX, true);
    assert_eq!(threshold(max_inverse), Err(Error::ArithmeticOverflow));
}

#[test]
fn no_spendable_execution_or_verifier_is_exposed_by_the_fraction() {
    let price = threshold(input(100 * ONE, u128::from(ONE), 1, ONE, false)).unwrap();
    assert_eq!(price.floor_e8(), 99 * u128::from(ONE));
    // The only public result is a rational inequality. It has no
    // account, signature, token, payout, side-effect or authorization.
    assert_eq!(
        price.compare_declared_quote_e8(99 * ONE),
        Ok(Ordering::Equal)
    );
}
