//! Offline Stage 24A synthetic numeric parity, no chain authentication.
use tradelayer_core::clearing_math::{
    clamp_funding_bps, compute_loss_coverage, mark_to_mark_pnl_atoms, ClearingMathError,
    PnlRounding, ATOMS_PER_UNIT,
};

const VECTORS: &str = include_str!("../fixtures/clearing_math_vectors.tsv");

fn parse<T: std::str::FromStr>(s: &str) -> T
where
    T::Err: core::fmt::Debug,
{
    s.parse::<T>().unwrap()
}

#[test]
fn independent_node_bigint_vectors_match_in_both_rounding_modes() {
    let mut examined = 0usize;
    let mut precision_differences = 0usize;
    for line in VECTORS.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 15, "malformed row: {line}");
        let label = columns[0];
        let contracts = parse::<i64>(columns[1]);
        let from = parse::<u64>(columns[2]);
        let to = parse::<u64>(columns[3]);
        let notional = parse::<u64>(columns[4]);
        let inverse = match columns[5] {
            "0" => false,
            "1" => true,
            _ => panic!("invalid inverse flag in {label}"),
        };
        let legacy = mark_to_mark_pnl_atoms(
            contracts, from, to, notional, inverse, PnlRounding::Legacy20DecimalPlaces,
        )
        .unwrap();
        let rational = mark_to_mark_pnl_atoms(
            contracts, from, to, notional, inverse, PnlRounding::ExactRational,
        )
        .unwrap();
        assert_eq!(legacy, parse::<i128>(columns[6]), "legacy {label}");
        assert_eq!(rational, parse::<i128>(columns[7]), "exact {label}");
        assert_eq!(
            clamp_funding_bps(parse::<i64>(columns[8])),
            parse::<i64>(columns[9]),
            "funding {label}"
        );
        let coverage = compute_loss_coverage(
            parse::<i64>(columns[10]),
            parse::<u64>(columns[11]),
            parse::<i64>(columns[12]),
            parse::<u64>(columns[13]),
        )
        .unwrap();
        assert_eq!(
            coverage.coverage_twice_atoms,
            parse::<i128>(columns[14]),
            "loss coverage {label}"
        );
        assert_eq!(
            coverage.maintenance_twice_atoms,
            parse::<u128>(columns[11]),
            "maintenance half-unit {label}"
        );
        if legacy != rational {
            assert_eq!(label, "20dp_inverse_divergence");
            assert_eq!((legacy, rational), (36_633_685_829, 36_633_685_828));
            precision_differences += 1;
        }
        examined += 1;
    }
    assert_eq!(examined, 15);
    assert_eq!(precision_differences, 1);
}

#[test]
fn half_atomic_margin_is_preserved_and_never_spendable_as_full_atom() {
    let c = compute_loss_coverage(0, 1, 0, 0).unwrap();
    assert_eq!(c.maintenance_twice_atoms, 1);
    assert_eq!(c.coverage_twice_atoms, 1);
    assert!(c.covers(0));
    assert!(!c.covers(1));
    assert!(compute_loss_coverage(0, 0, -3, 1)
        .unwrap()
        .coverage_twice_atoms
        < 0);
}

#[test]
fn inverse_and_linear_signed_positions_cancel_symmetrically() {
    for i in 1u64..100 {
        let from = (100 + i) * 100_000_000 + 1;
        let to = (140 + i) * 100_000_000 + 13;
        let notional = (i + 1) * 100_000_000;
        for rounding in [
            PnlRounding::Legacy20DecimalPlaces,
            PnlRounding::ExactRational,
        ] {
            for inverse in [false, true] {
                let gain = mark_to_mark_pnl_atoms(
                    3,
                    from,
                    to,
                    notional,
                    inverse,
                    rounding,
                )
                .unwrap();
                let loss = mark_to_mark_pnl_atoms(
                    -3,
                    from,
                    to,
                    notional,
                    inverse,
                    rounding,
                )
                .unwrap();
                assert_eq!(gain + loss, 0);
            }
        }
    }
}

#[test]
fn funding_deadband_is_monotone_and_sign_symmetric() {
    for bps in -500..=500 {
        let effective = clamp_funding_bps(bps);
        assert_eq!(effective, -clamp_funding_bps(-bps));
        assert!(effective.abs() <= bps.abs());
    }
    assert_eq!(clamp_funding_bps(i64::MAX), i64::MAX - 5);
    assert_eq!(clamp_funding_bps(i64::MIN), i64::MIN + 5);
}

#[test]
fn zero_price_and_zero_notional_follow_pinned_legacy_semantics() {
    for mode in [
        PnlRounding::Legacy20DecimalPlaces,
        PnlRounding::ExactRational,
    ] {
        assert_eq!(mark_to_mark_pnl_atoms(20, 0, 1, 1, false, mode), Ok(0));
        assert_eq!(mark_to_mark_pnl_atoms(20, 1, 0, 1, true, mode), Ok(0));
        assert_eq!(
            mark_to_mark_pnl_atoms(1, 100 * ATOMS_PER_UNIT as u64, 110 * ATOMS_PER_UNIT as u64, 0, false, mode),
            Ok(10 * ATOMS_PER_UNIT as i128)
        );
    }
}

#[test]
fn oversized_rational_intermediates_fail_closed() {
    let err = mark_to_mark_pnl_atoms(
        i64::MAX,
        u64::MAX,
        1,
        u64::MAX,
        true,
        PnlRounding::ExactRational,
    )
    .unwrap_err();
    assert_eq!(err, ClearingMathError::ArithmeticOverflow);
    let err = mark_to_mark_pnl_atoms(
        i64::MAX,
        1,
        u64::MAX,
        u64::MAX,
        false,
        PnlRounding::Legacy20DecimalPlaces,
    )
    .unwrap_err();
    assert_eq!(err, ClearingMathError::ArithmeticOverflow);
}
