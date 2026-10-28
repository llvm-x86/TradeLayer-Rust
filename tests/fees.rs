//! Deterministic fee-engine tests: exact vectors, a seeded property sweep,
//! validation and tier rejections, and a self-trade (wash-trade) bound.
//! Standalone research arithmetic; not wired to the spot matcher or consensus.

use tradelayer_core::fees::{
    settle_fill, FeeAccrual, FeeError, FeeSchedule, FeeTier, FeeTierTable, FillFees, MAX_FEE_TIERS,
};
use tradelayer_core::ledger::PropertyId;

const SEED: u64 = 0x5EED_FEE5_2026_0001;
const PROPERTY_PAIRS: u32 = 250_000;

fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}

fn schedule(taker: u32, maker: i32) -> FeeSchedule {
    FeeSchedule::new(taker, maker).unwrap()
}

/// Deterministic LCG with a SplitMix-style output mix. Fixed seed, no floats.
struct Lcg(u64);

impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Independent specification of schedule validity.
fn reference_valid(taker: u32, maker: i32) -> bool {
    taker <= 1000 && maker.unsigned_abs() <= 1000 && (maker >= 0 || maker.unsigned_abs() <= taker)
}

/// Spec: `fee` is the smallest value with `fee * 10_000 >= num`, i.e. ceiling.
fn assert_is_ceil(fee: u128, num: u128) {
    assert!(fee * 10_000 >= num, "ceil too small: fee={fee} num={num}");
    assert!(
        fee == 0 || (fee - 1) * 10_000 < num,
        "ceil too large: fee={fee} num={num}"
    );
}

/// Spec: `rebate` is the largest value with `rebate * 10_000 <= num`, i.e. floor.
fn assert_is_floor(rebate: u128, num: u128) {
    assert!(
        rebate * 10_000 <= num,
        "floor too large: rebate={rebate} num={num}"
    );
    assert!(
        (rebate + 1) * 10_000 > num,
        "floor too small: rebate={rebate} num={num}"
    );
}

fn check_fill(schedule: &FeeSchedule, amount: u64, fees: &FillFees) {
    let q = u128::from(amount);
    let taker_num = q * u128::from(schedule.taker_fee_bps);
    assert_is_ceil(fees.taker_fee_atoms, taker_num);

    let maker_num = q * u128::from(schedule.maker_fee_bps.unsigned_abs());
    if schedule.maker_fee_bps >= 0 {
        let charged =
            u128::try_from(fees.maker_fee_atoms).expect("charged maker fee is non-negative");
        assert_is_ceil(charged, maker_num);
    } else {
        let rebate = u128::try_from(-fees.maker_fee_atoms).expect("rebate is paid out");
        assert_is_floor(rebate, maker_num);
    }

    // Accounting identity: protocol net = taker fee - rebate paid
    // (equivalently taker fee + signed maker fee).
    let net = i128::try_from(fees.taker_fee_atoms).unwrap() + fees.maker_fee_atoms;
    assert!(net >= 0, "protocol net negative: {net}");
    assert_eq!(fees.protocol_net_atoms, u128::try_from(net).unwrap());
}

#[test]
fn exact_vector_one_million_atoms_10_bps_taker_minus_2_bps_maker() {
    let fees = settle_fill(&schedule(10, -2), 1_000_000).unwrap();
    assert_eq!(fees.taker_fee_atoms, 1_000);
    assert_eq!(fees.maker_fee_atoms, -200);
    assert_eq!(fees.protocol_net_atoms, 800);
}

#[test]
fn exact_vector_one_atom_rounds_taker_up_and_rebate_down() {
    // 1 * 10 / 10_000 = 0.001 -> taker rounds UP to 1.
    // 1 * 2 / 10_000 = 0.0002 -> rebate rounds DOWN to 0.
    let fees = settle_fill(&schedule(10, -2), 1).unwrap();
    assert_eq!(fees.taker_fee_atoms, 1);
    assert_eq!(fees.maker_fee_atoms, 0);
    assert_eq!(fees.protocol_net_atoms, 1);
}

#[test]
fn exact_vector_one_atom_positive_maker_fee_rounds_up() {
    let fees = settle_fill(&schedule(0, 1), 1).unwrap();
    assert_eq!(fees.taker_fee_atoms, 0);
    assert_eq!(fees.maker_fee_atoms, 1);
    assert_eq!(fees.protocol_net_atoms, 1);
}

#[test]
fn exact_vector_u64_max_full_rebate_leaves_one_atom() {
    // u64::MAX * 1000 / 10_000 = 1_844_674_407_370_955_161.5
    // taker rounds up to ...162, rebate rounds down to ...161.
    let fees = settle_fill(&schedule(1000, -1000), u64::MAX).unwrap();
    assert_eq!(fees.taker_fee_atoms, 1_844_674_407_370_955_162);
    assert_eq!(fees.maker_fee_atoms, -1_844_674_407_370_955_161);
    assert_eq!(fees.protocol_net_atoms, 1);
}

#[test]
fn exact_vector_u64_max_positive_maker_fee() {
    let fees = settle_fill(&schedule(0, 1000), u64::MAX).unwrap();
    assert_eq!(fees.taker_fee_atoms, 0);
    assert_eq!(fees.maker_fee_atoms, 1_844_674_407_370_955_162);
    assert_eq!(fees.protocol_net_atoms, 1_844_674_407_370_955_162);
}

#[test]
fn exact_vector_u64_max_maximum_taker_no_maker() {
    let fees = settle_fill(&schedule(1000, 0), u64::MAX).unwrap();
    assert_eq!(fees.taker_fee_atoms, 1_844_674_407_370_955_162);
    assert_eq!(fees.maker_fee_atoms, 0);
    assert_eq!(fees.protocol_net_atoms, 1_844_674_407_370_955_162);
}

#[test]
fn rounding_boundary_taker_one_bps() {
    assert_eq!(
        settle_fill(&schedule(1, 0), 9_999).unwrap().taker_fee_atoms,
        1
    );
    assert_eq!(
        settle_fill(&schedule(1, 0), 10_000)
            .unwrap()
            .taker_fee_atoms,
        1
    );
    assert_eq!(
        settle_fill(&schedule(1, 0), 10_001)
            .unwrap()
            .taker_fee_atoms,
        2
    );
}

#[test]
fn rounding_boundary_rebate_two_bps_rounds_down() {
    // 4_999 * 2 = 9_998 < 10_000 -> rebate 0.
    // 5_000 * 2 = 10_000 -> rebate exactly 1, and the taker fee is also
    // exactly 1, so protocol net is 0 (break-even).
    let below = settle_fill(&schedule(2, -2), 4_999).unwrap();
    assert_eq!(below.taker_fee_atoms, 1);
    assert_eq!(below.maker_fee_atoms, 0);
    assert_eq!(below.protocol_net_atoms, 1);
    let at = settle_fill(&schedule(2, -2), 5_000).unwrap();
    assert_eq!(at.taker_fee_atoms, 1);
    assert_eq!(at.maker_fee_atoms, -1);
    assert_eq!(at.protocol_net_atoms, 0);
}

#[test]
fn zero_quote_amount_is_refused() {
    assert_eq!(
        settle_fill(&schedule(10, -2), 0),
        Err(FeeError::ZeroQuoteAmount)
    );
}

#[test]
fn schedule_validation_accepts_bounds() {
    for (t, m) in [
        (0, 0),
        (0, 1000),
        (1000, 0),
        (5, -5),
        (1000, -1000),
        (10, -2),
    ] {
        assert!(FeeSchedule::new(t, m).is_ok(), "rejected ({t}, {m})");
    }
}

#[test]
fn schedule_validation_rejections() {
    assert_eq!(
        FeeSchedule::new(1001, 0),
        Err(FeeError::TakerFeeOutOfRange { bps: 1001 })
    );
    assert_eq!(
        FeeSchedule::new(1000, 1001),
        Err(FeeError::MakerFeeOutOfRange { bps: 1001 })
    );
    assert_eq!(
        FeeSchedule::new(1000, -1001),
        Err(FeeError::MakerFeeOutOfRange { bps: -1001 })
    );
    assert_eq!(
        FeeSchedule::new(5, -6),
        Err(FeeError::RebateExceedsTakerFee {
            rebate_bps: 6,
            taker_fee_bps: 5
        })
    );
    assert_eq!(
        FeeSchedule::new(0, -1),
        Err(FeeError::RebateExceedsTakerFee {
            rebate_bps: 1,
            taker_fee_bps: 0
        })
    );
}

#[test]
fn settle_refuses_unvalidated_struct_literal() {
    // Public fields allow a literal that bypasses `new`. Settlement must still
    // refuse it.
    let raw = FeeSchedule {
        taker_fee_bps: 0,
        maker_fee_bps: -1,
    };
    assert_eq!(
        settle_fill(&raw, 10_000),
        Err(FeeError::RebateExceedsTakerFee {
            rebate_bps: 1,
            taker_fee_bps: 0
        })
    );
    let raw_taker = FeeSchedule {
        taker_fee_bps: 1001,
        maker_fee_bps: 0,
    };
    assert_eq!(
        settle_fill(&raw_taker, 1),
        Err(FeeError::TakerFeeOutOfRange { bps: 1001 })
    );
}

#[test]
fn seeded_property_sweep_over_random_schedule_amount_pairs() {
    let mut rng = Lcg(SEED);
    let mut accrual = FeeAccrual::new();
    let mut expected_by_property = [0u128; 3];
    let mut settled = 0u32;
    let mut refused = 0u32;

    for i in 0..PROPERTY_PAIRS {
        let taker = (rng.next_u64() % 1001) as u32;
        let maker: i32 = match rng.next_u64() % 4 {
            0 => (rng.next_u64() % 2001) as i32 - 1000,
            1 => -((rng.next_u64() % (u64::from(taker) + 1)) as i32),
            2 => (rng.next_u64() % 1001) as i32,
            _ => -((rng.next_u64() % 1001) as i32),
        };
        let bits = 1 + (rng.next_u64() % 64) as u32;
        let mut amount = rng.next_u64() >> (64 - bits);
        if amount == 0 {
            amount = 1;
        }
        if rng.next_u64().is_multiple_of(512) {
            amount = u64::MAX;
        }

        let constructed = FeeSchedule::new(taker, maker);
        assert_eq!(constructed.is_ok(), reference_valid(taker, maker));
        // The unvalidated literal path must agree with the constructor.
        let raw = FeeSchedule {
            taker_fee_bps: taker,
            maker_fee_bps: maker,
        };
        match constructed {
            Err(_) => {
                assert!(settle_fill(&raw, amount).is_err());
                refused += 1;
            }
            Ok(valid) => {
                let fees = settle_fill(&valid, amount).expect("valid schedule settles");
                check_fill(&valid, amount, &fees);
                let property = p(7 + (i % 3));
                expected_by_property[(i % 3) as usize] += fees.protocol_net_atoms;
                accrual.accrue(property, &fees).unwrap();
                settled += 1;
            }
        }
    }

    assert_eq!(settled + refused, PROPERTY_PAIRS);
    assert!(settled > 50_000, "too few valid pairs settled: {settled}");
    assert!(refused > 0, "no invalid schedules exercised");
    for (offset, expected) in expected_by_property.iter().enumerate() {
        assert_eq!(accrual.total(p(7 + offset as u32)), *expected);
    }
}

#[test]
fn self_trade_is_never_profitable() {
    // Owner is both taker and maker. Net cash flow to the owner is
    // -(taker_fee + maker_fee). That cost must be >= 0 for every valid schedule
    // and amount.
    let takers = [0u32, 1, 2, 5, 10, 33, 100, 999, 1000];
    let amounts = [1u64, 2, 3, 999, 1_000, 9_999, 10_000, 10_001, u64::MAX];
    let mut checked = 0u32;
    for &taker in &takers {
        for maker in -1000i32..=1000 {
            let Ok(valid) = FeeSchedule::new(taker, maker) else {
                continue;
            };
            for &amount in &amounts {
                let fees = settle_fill(&valid, amount).unwrap();
                let owner_cost = fees.taker_fee_atoms as i128 + fees.maker_fee_atoms;
                assert!(
                    owner_cost >= 0,
                    "profitable self-trade: {valid:?} amount={amount}"
                );
                assert_eq!(owner_cost as u128, fees.protocol_net_atoms);
                checked += 1;
            }
        }
    }
    assert!(checked > 100_000);
}

#[test]
fn self_trade_at_equal_taker_and_rebate_breaks_even_at_most() {
    // 10 bps taker and -10 bps rebate: cost is ceil(q*10/1e4) - floor(q*10/1e4),
    // which is 0 or 1 and never negative.
    let s = schedule(10, -10);
    for amount in [1u64, 999, 1_000, 1_001, 10_000, 10_001, 123_456_789] {
        let fees = settle_fill(&s, amount).unwrap();
        let cost = fees.taker_fee_atoms as i128 + fees.maker_fee_atoms;
        assert!(cost == 0 || cost == 1, "amount={amount} cost={cost}");
    }
    assert_eq!(settle_fill(&s, 1_000).unwrap().protocol_net_atoms, 0);
}

#[test]
fn unvalidated_rebate_above_taker_would_be_a_profitable_self_trade() {
    // Illustration only: the refused literal would pay the owner 1 atom
    // net at 10_000. Settlement refuses it, so the wash trade does not
    // execute.
    let raw = FeeSchedule {
        taker_fee_bps: 0,
        maker_fee_bps: -1,
    };
    assert!(settle_fill(&raw, 10_000).is_err());
}

#[test]
fn accrual_sums_per_property_and_untouched_is_zero() {
    let mut acc = FeeAccrual::new();
    let a = settle_fill(&schedule(10, -2), 1_000_000).unwrap(); // net 800
    let b = settle_fill(&schedule(10, -2), 1).unwrap(); // net 1
    acc.accrue(p(7), &a).unwrap();
    acc.accrue(p(7), &b).unwrap();
    acc.accrue(p(8), &a).unwrap();
    assert_eq!(acc.total(p(7)), 801);
    assert_eq!(acc.total(p(8)), 800);
    assert_eq!(acc.total(p(9)), 0);
}

#[test]
fn accrual_refuses_overflow_and_changes_nothing() {
    let mut acc = FeeAccrual::new();
    let max = FillFees {
        taker_fee_atoms: u128::MAX,
        maker_fee_atoms: 0,
        protocol_net_atoms: u128::MAX,
    };
    acc.accrue(p(7), &max).unwrap();
    let one = FillFees {
        taker_fee_atoms: 1,
        maker_fee_atoms: 0,
        protocol_net_atoms: 1,
    };
    assert_eq!(acc.accrue(p(7), &one), Err(FeeError::ArithmeticOverflow));
    assert_eq!(acc.total(p(7)), u128::MAX);
}

#[test]
fn tier_table_rejects_empty_and_oversized_tables() {
    assert_eq!(FeeTierTable::new(vec![]), Err(FeeError::EmptyTierTable));
    let too_many: Vec<FeeTier> = (0..=MAX_FEE_TIERS as u128)
        .map(|i| FeeTier::new(i * 10, 10, 0).unwrap())
        .collect();
    assert_eq!(
        FeeTierTable::new(too_many),
        Err(FeeError::TooManyTiers {
            count: MAX_FEE_TIERS + 1
        })
    );
}

#[test]
fn tier_table_rejects_nonzero_first_threshold() {
    let tiers = vec![FeeTier::new(1, 10, 0).unwrap()];
    assert_eq!(
        FeeTierTable::new(tiers),
        Err(FeeError::FirstTierThresholdNonZero { threshold_atoms: 1 })
    );
}

#[test]
fn tier_table_rejects_duplicate_threshold() {
    let tiers = vec![
        FeeTier::new(0, 10, 0).unwrap(),
        FeeTier::new(100, 8, 0).unwrap(),
        FeeTier::new(100, 6, 0).unwrap(),
    ];
    assert_eq!(
        FeeTierTable::new(tiers),
        Err(FeeError::DuplicateTierThreshold { index: 2 })
    );
}

#[test]
fn tier_table_rejects_non_monotonic_thresholds() {
    let tiers = vec![
        FeeTier::new(0, 10, 0).unwrap(),
        FeeTier::new(200, 8, 0).unwrap(),
        FeeTier::new(100, 6, 0).unwrap(),
    ];
    assert_eq!(
        FeeTierTable::new(tiers),
        Err(FeeError::NonMonotonicTierThreshold { index: 2 })
    );
}

#[test]
fn tier_with_rebate_above_taker_is_rejected() {
    assert_eq!(
        FeeTier::new(0, 10, -11),
        Err(FeeError::RebateExceedsTakerFee {
            rebate_bps: 11,
            taker_fee_bps: 10
        })
    );
    // A literal tier that bypasses FeeTier::new is still rejected by the table.
    let literal = FeeTier {
        min_trailing_volume_atoms: 0,
        schedule: FeeSchedule {
            taker_fee_bps: 10,
            maker_fee_bps: -11,
        },
    };
    assert_eq!(
        FeeTierTable::new(vec![literal]),
        Err(FeeError::RebateExceedsTakerFee {
            rebate_bps: 11,
            taker_fee_bps: 10
        })
    );
}

#[test]
fn tier_with_taker_above_cap_is_rejected() {
    assert_eq!(
        FeeTier::new(0, 1001, 0),
        Err(FeeError::TakerFeeOutOfRange { bps: 1001 })
    );
}

#[test]
fn tier_lookup_selects_highest_threshold_not_exceeding_volume() {
    let table = FeeTierTable::new(vec![
        FeeTier::new(0, 10, -5).unwrap(),
        FeeTier::new(1_000, 8, -4).unwrap(),
        FeeTier::new(10_000, 5, -2).unwrap(),
    ])
    .unwrap();
    assert_eq!(*table.schedule_for_volume(0), schedule(10, -5));
    assert_eq!(*table.schedule_for_volume(999), schedule(10, -5));
    assert_eq!(*table.schedule_for_volume(1_000), schedule(8, -4));
    assert_eq!(*table.schedule_for_volume(9_999), schedule(8, -4));
    assert_eq!(*table.schedule_for_volume(10_000), schedule(5, -2));
    assert_eq!(*table.schedule_for_volume(u128::MAX), schedule(5, -2));
}
