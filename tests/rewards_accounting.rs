//! Accounting-only tests for src/rewards.rs: anti-sybil, wash-trade, conservation,
//! order independence, and overflow behavior. Not consensus. See docs/REWARDS.md.

use tradelayer_core::ledger::AccountId;
use tradelayer_core::rewards::{
    plan_epoch_rewards, MakerEpochStats, RewardError, RewardParams, RewardPlan, BPS_DENOMINATOR,
};

const FULL: u32 = BPS_DENOMINATOR;

fn id(name: &str) -> AccountId {
    AccountId::try_from(name).expect("valid test account id")
}

fn maker(name: &str, resting: u128, filled: u128, fees: u64) -> MakerEpochStats {
    MakerEpochStats {
        account: id(name),
        resting_notional_seconds: resting,
        filled_notional_atoms: filled,
        fees_paid_atoms: fees,
        attested: true,
    }
}

fn params(fee_share_bps: u32, max_account_share_bps: u32) -> RewardParams {
    RewardParams {
        fee_share_bps,
        max_account_share_bps,
        fee_rebate_cap_bps: FULL,
        require_attestation: false,
    }
}

fn payout(plan: &RewardPlan, name: &str) -> u64 {
    plan.payouts
        .iter()
        .find(|(a, _)| a.as_str() == name)
        .map_or(0, |(_, p)| *p)
}

fn sum_payouts(plan: &RewardPlan) -> u64 {
    plan.payouts.iter().map(|(_, p)| *p).sum()
}

/// Deterministic LCG (Knuth MMIX constants). Used only for seeded test inputs.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 16
    }
    /// Uniform-ish value in `0..n`. `n` must be nonzero.
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
    fn coin(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }
}

/// Splits `total` into `k` nonnegative parts that sum exactly to `total`.
fn partition(rng: &mut Lcg, total: u64, k: usize) -> Vec<u64> {
    let mut cuts: Vec<u64> = (0..k - 1)
        .map(|_| if total == 0 { 0 } else { rng.below(total + 1) })
        .collect();
    cuts.sort_unstable();
    let mut parts = Vec::with_capacity(k);
    let mut prev = 0;
    for c in cuts {
        parts.push(c - prev);
        prev = c;
    }
    parts.push(total - prev);
    parts
}

fn expected_budget(fee_pool: u64, fee_share_bps: u32, emission: u64) -> u64 {
    let fee = u128::from(fee_pool) * u128::from(fee_share_bps) / u128::from(FULL);
    u64::try_from(fee).unwrap() + emission
}

// ---------------------------------------------------------------------------
// Worked examples with exact expected values.
// ---------------------------------------------------------------------------

#[test]
fn empty_stats_retain_entire_budget() {
    let plan = plan_epoch_rewards(&params(5_000, FULL), 1_000, 200, &[]).unwrap();
    assert!(plan.payouts.is_empty());
    assert_eq!(plan.retained_atoms, 700);
}

#[test]
fn zero_budget_with_makers_is_ok_and_pays_nothing() {
    let stats = [maker("a", 5, 1, 9)];
    let plan = plan_epoch_rewards(&params(FULL, FULL), 0, 0, &stats).unwrap();
    assert!(plan.payouts.is_empty());
    assert_eq!(plan.retained_atoms, 0);
}

#[test]
fn floor_remainder_is_retained_exactly() {
    // 10 atoms over three equal scores: floor(10/3) = 3 each, 1 atom retained.
    let stats = [
        maker("a", 1, 1, 0),
        maker("b", 1, 1, 0),
        maker("c", 1, 1, 0),
    ];
    let plan = plan_epoch_rewards(&params(0, FULL), 0, 10, &stats).unwrap();
    assert_eq!(payout(&plan, "a"), 3);
    assert_eq!(payout(&plan, "b"), 3);
    assert_eq!(payout(&plan, "c"), 3);
    assert_eq!(plan.retained_atoms, 1);
}

#[test]
fn resting_without_fills_earns_nothing() {
    // Kills mutation M1 (filled-notional zeroing removed).
    let stats = [maker("idle", 1_000_000, 0, 0), maker("active", 10, 5, 0)];
    let plan = plan_epoch_rewards(&params(0, FULL), 0, 100, &stats).unwrap();
    assert_eq!(payout(&plan, "idle"), 0);
    assert_eq!(payout(&plan, "active"), 100);
    assert_eq!(plan.retained_atoms, 0);
}

#[test]
fn per_account_cap_limits_single_identity() {
    // Kills mutation M2 (cap removed). Pool 1000, cap 50% = 500.
    // A has score 300 (raw 750, capped to 500). B has 100 (raw 250).
    let stats = [maker("a", 300, 1, 0), maker("b", 100, 1, 0)];
    let plan = plan_epoch_rewards(&params(0, 5_000), 0, 1_000, &stats).unwrap();
    assert_eq!(payout(&plan, "a"), 500);
    assert_eq!(payout(&plan, "b"), 250);
}

#[test]
fn cap_excess_is_retained_not_redistributed() {
    // Kills mutation M3b (remainder or excess handed to other accounts).
    // A's 250 excess above the cap stays in the pool, so B is not increased.
    let stats = [maker("a", 300, 1, 0), maker("b", 100, 1, 0)];
    let plan = plan_epoch_rewards(&params(0, 5_000), 0, 1_000, &stats).unwrap();
    assert_eq!(payout(&plan, "b"), 250);
    assert_eq!(plan.retained_atoms, 250);
}

// ---------------------------------------------------------------------------
// Anti-sybil properties.
// ---------------------------------------------------------------------------

#[test]
fn splitting_without_binding_cap_never_increases_payout_exact_example() {
    // Property (1), uncapped regime (max_account_share_bps = 10_000).
    // Combined A=3, B=3, pool 10: A gets floor(10*3/6) = 5.
    // Splitting A into 1 and 2 gives floor(10/6) + floor(20/6) = 1 + 3 = 4.
    // The result is strictly lower, so the relation is <=, not equality.
    let combined = [maker("a", 3, 1, 0), maker("b", 3, 1, 0)];
    let split = [
        maker("a1", 1, 1, 0),
        maker("a2", 2, 1, 0),
        maker("b", 3, 1, 0),
    ];
    let before = payout(
        &plan_epoch_rewards(&params(0, FULL), 0, 10, &combined).unwrap(),
        "a",
    );
    let plan = plan_epoch_rewards(&params(0, FULL), 0, 10, &split).unwrap();
    let after = payout(&plan, "a1") + payout(&plan, "a2");
    assert_eq!(before, 5);
    assert_eq!(after, 4);
    assert!(after <= before);
}

#[test]
fn per_account_cap_is_defeated_by_identity_splitting() {
    // Documents a limit of the design. The cap is keyed on AccountId, so it
    // bounds one identity, not one entity. Same stats as the cap test above,
    // with A's 300 split into 150 and 150. The entity now earns 750, not 500.
    // Splitting only increases payout when the cap binds on the combined account.
    let combined = [maker("a", 300, 1, 0), maker("b", 100, 1, 0)];
    let split = [
        maker("a1", 150, 1, 0),
        maker("a2", 150, 1, 0),
        maker("b", 100, 1, 0),
    ];
    let before = payout(
        &plan_epoch_rewards(&params(0, 5_000), 0, 1_000, &combined).unwrap(),
        "a",
    );
    let plan = plan_epoch_rewards(&params(0, 5_000), 0, 1_000, &split).unwrap();
    let after = payout(&plan, "a1") + payout(&plan, "a2");
    assert_eq!(before, 500);
    assert_eq!(after, 750);
    assert!(after > before);
}

#[test]
fn fee_funded_aggregate_is_bounded_by_fees_across_identities() {
    // Property (2) generalized. Fee-funded payouts summed over any set of
    // identities controlled by one entity cannot exceed rebate_bps of the
    // fees those identities paid. Splitting cannot raise this bound.
    let pool = 1_000_000u64;
    let p = RewardParams {
        fee_share_bps: FULL,
        max_account_share_bps: FULL,
        fee_rebate_cap_bps: 5_000,
        require_attestation: false,
    };
    // One entity with 100 fees, split across three identities that all trade.
    let stats = [
        maker("e1", 1_000, 1, 40),
        maker("e2", 1_000, 1, 30),
        maker("e3", 1_000, 1, 30),
        maker("honest", 1_000, 1, 1_000_000),
    ];
    let plan = plan_epoch_rewards(&p, pool, 0, &stats).unwrap();
    let entity_fee_payout = payout(&plan, "e1") + payout(&plan, "e2") + payout(&plan, "e3");
    assert!(entity_fee_payout <= 100 * 5_000 / u64::from(FULL));
    assert_eq!(entity_fee_payout, 20 + 15 + 15);
}

// ---------------------------------------------------------------------------
// Wash trading.
// ---------------------------------------------------------------------------

#[test]
fn fee_rebate_cap_limits_wash_trader_to_its_own_fees() {
    // Kills mutation M4 (fee rebate cap removed).
    // The wash trader W pays 100 fees and would take almost the whole fee pool
    // on raw score. Its fee-funded payout must be capped at 100 * 5000 / 10000 = 50.
    let p = RewardParams {
        fee_share_bps: FULL,
        max_account_share_bps: FULL,
        fee_rebate_cap_bps: 5_000,
        require_attestation: false,
    };
    let stats = [
        maker("wash", 1_000_000_000, 1_000_000, 100),
        maker("honest", 1_000_000_000, 1, 1_000_000),
    ];
    let plan = plan_epoch_rewards(&p, 1_000_000, 0, &stats).unwrap();
    let wash = payout(&plan, "wash");
    assert_eq!(wash, 50);
    assert!(
        wash < 100,
        "wash trader must net a loss on fee-funded payout"
    );
    assert_eq!(payout(&plan, "honest"), 500_000);
    assert_eq!(plan.retained_atoms, 1_000_000 - 50 - 500_000);
}

#[test]
fn wash_trader_with_zero_fees_gets_no_fee_funded_payout() {
    let p = RewardParams {
        fee_share_bps: FULL,
        max_account_share_bps: FULL,
        fee_rebate_cap_bps: FULL,
        require_attestation: false,
    };
    let stats = [maker("wash", 1_000_000, 1_000, 0)];
    let plan = plan_epoch_rewards(&p, 1_000_000, 0, &stats).unwrap();
    assert_eq!(payout(&plan, "wash"), 0);
    assert_eq!(plan.retained_atoms, 1_000_000);
}

#[test]
fn residual_emission_pool_is_not_fee_capped() {
    // Documents a residual risk, not a guarantee. Emission-funded payouts are
    // not limited by fees, so a zero-fee wash account still earns from emissions.
    // Only the fee-funded part is protected by fee_rebate_cap_bps.
    let stats = [maker("wash", 1_000, 1_000, 0)];
    let plan = plan_epoch_rewards(&params(0, FULL), 0, 1_000, &stats).unwrap();
    assert_eq!(payout(&plan, "wash"), 1_000);
}

// ---------------------------------------------------------------------------
// Rejections, overflow, and no-panic behavior.
// ---------------------------------------------------------------------------

#[test]
fn duplicate_account_cannot_bypass_cap() {
    let stats = [maker("a", 1, 1, 0), maker("a", 1, 1, 0)];
    assert_eq!(
        plan_epoch_rewards(&params(0, 5_000), 0, 10, &stats),
        Err(RewardError::DuplicateAccount(id("a")))
    );
}

#[test]
fn degenerate_parameters_are_rejected() {
    let stats = [maker("a", 1, 1, 0)];
    let bad = [
        RewardParams {
            fee_share_bps: FULL + 1,
            ..params(0, FULL)
        },
        RewardParams {
            max_account_share_bps: 0,
            ..params(0, FULL)
        },
        RewardParams {
            max_account_share_bps: FULL + 1,
            ..params(0, FULL)
        },
        RewardParams {
            fee_rebate_cap_bps: 0,
            ..params(0, FULL)
        },
        RewardParams {
            fee_rebate_cap_bps: FULL + 1,
            ..params(0, FULL)
        },
    ];
    for p in bad {
        assert!(plan_epoch_rewards(&p, 1, 1, &stats).is_err(), "{p:?}");
    }
}

#[test]
fn budget_overflow_is_an_error_not_a_panic() {
    assert_eq!(
        plan_epoch_rewards(&params(FULL, FULL), u64::MAX, 1, &[]),
        Err(RewardError::BudgetOverflow)
    );
}

#[test]
fn score_sum_overflow_is_an_error_not_a_panic() {
    let stats = [maker("a", u128::MAX, 1, 0), maker("b", 1, 1, 0)];
    assert_eq!(
        plan_epoch_rewards(&params(0, FULL), 0, 10, &stats),
        Err(RewardError::ArithmeticOverflow)
    );
}

#[test]
fn maximal_single_score_uses_wide_arithmetic_exactly() {
    // pool * score = u64::MAX * u128::MAX needs 192 bits. The exact share is the
    // full pool, so nothing is retained.
    let stats = [maker("a", u128::MAX, 1, 0)];
    let plan = plan_epoch_rewards(&params(0, FULL), 0, u64::MAX, &stats).unwrap();
    assert_eq!(payout(&plan, "a"), u64::MAX);
    assert_eq!(plan.retained_atoms, 0);
}

// ---------------------------------------------------------------------------
// Seeded randomized properties.
// ---------------------------------------------------------------------------

fn random_params(rng: &mut Lcg) -> RewardParams {
    RewardParams {
        fee_share_bps: rng.below(u64::from(FULL) + 1) as u32,
        max_account_share_bps: 1 + rng.below(u64::from(FULL)) as u32,
        fee_rebate_cap_bps: 1 + rng.below(u64::from(FULL)) as u32,
        require_attestation: rng.coin(2),
    }
}

fn random_makers(rng: &mut Lcg) -> Vec<MakerEpochStats> {
    let n = rng.below(9) as usize;
    let mut pool: Vec<u32> = (0..32).collect();
    rng.shuffle(&mut pool);
    (0..n)
        .map(|k| MakerEpochStats {
            account: id(&format!("maker-{:02}", pool[k])),
            resting_notional_seconds: if rng.coin(5) {
                0
            } else {
                u128::from(rng.below(1 << 50))
            },
            filled_notional_atoms: if rng.coin(3) {
                0
            } else {
                u128::from(rng.below(1 << 40) + 1)
            },
            fees_paid_atoms: rng.below(1 << 36),
            attested: !rng.coin(4),
        })
        .collect()
}

#[test]
fn hundred_thousand_seeded_epochs_conserve_and_are_order_independent() {
    let mut rng = Lcg::new(0x5EED_0001);
    for epoch in 0..100_000u32 {
        let p = random_params(&mut rng);
        let fee_pool = rng.below(1 << 40);
        let emission = if rng.coin(8) { 0 } else { rng.below(1 << 40) };
        let makers = random_makers(&mut rng);

        let plan = plan_epoch_rewards(&p, fee_pool, emission, &makers)
            .unwrap_or_else(|e| panic!("epoch {epoch}: unexpected error {e}"));

        // Conservation: payouts plus retained equal the budget exactly.
        let budget = expected_budget(fee_pool, p.fee_share_bps, emission);
        assert_eq!(
            sum_payouts(&plan) + plan.retained_atoms,
            budget,
            "epoch {epoch}"
        );

        // Per-account cap, expressed as the sum of the per-pool caps.
        let fee_budget = expected_budget(fee_pool, p.fee_share_bps, 0);
        let cap_total = fee_budget * u64::from(p.max_account_share_bps) / u64::from(FULL)
            + emission * u64::from(p.max_account_share_bps) / u64::from(FULL);
        for (acct, pay) in &plan.payouts {
            assert!(*pay <= cap_total, "epoch {epoch} {acct:?}");
        }

        // Ineligible makers are never paid.
        for m in &makers {
            if m.filled_notional_atoms == 0 || (p.require_attestation && !m.attested) {
                assert_eq!(payout(&plan, m.account.as_str()), 0, "epoch {epoch}");
            }
        }

        // The fee-funded part is the plan with zero emission. Its aggregate
        // payout per maker must respect the rebate cap of that maker's own fees.
        let fee_only = plan_epoch_rewards(&p, fee_pool, 0, &makers).unwrap();
        let emission_only = plan_epoch_rewards(&p, 0, emission, &makers).unwrap();
        for m in &makers {
            let name = m.account.as_str();
            assert_eq!(
                payout(&plan, name),
                payout(&fee_only, name) + payout(&emission_only, name),
                "epoch {epoch}: pools must be additive"
            );
            let rebate_cap =
                u128::from(m.fees_paid_atoms) * u128::from(p.fee_rebate_cap_bps) / u128::from(FULL);
            assert!(
                u128::from(payout(&fee_only, name)) <= rebate_cap,
                "epoch {epoch}: fee-funded payout above rebate cap"
            );
        }
        assert_eq!(
            plan.retained_atoms,
            fee_only.retained_atoms + emission_only.retained_atoms,
            "epoch {epoch}"
        );

        // Order independence.
        let mut shuffled = makers.clone();
        rng.shuffle(&mut shuffled);
        let replay = plan_epoch_rewards(&p, fee_pool, emission, &shuffled).unwrap();
        assert_eq!(plan, replay, "epoch {epoch}: input order changed output");
    }
}

/// Property (1) in the uncapped regime. Splitting one entity into several
/// identities, with its score, fills, and fees partitioned, never increases the
/// entity's total payout. The fee-funded aggregate also respects its fee bound.
#[test]
fn uncapped_identity_splitting_never_increases_entity_payout() {
    let mut rng = Lcg::new(0x5EED_0002);
    for trial in 0..20_000u32 {
        let fee_pool = rng.below(1 << 40);
        let emission = rng.below(1 << 40);
        let p = RewardParams {
            fee_share_bps: rng.below(u64::from(FULL) + 1) as u32,
            max_account_share_bps: FULL,
            fee_rebate_cap_bps: 1 + rng.below(u64::from(FULL)) as u32,
            require_attestation: false,
        };

        let other_count = rng.below(5) as usize;
        let others: Vec<MakerEpochStats> = (0..other_count)
            .map(|i| MakerEpochStats {
                account: id(&format!("other-{i}")),
                resting_notional_seconds: u128::from(rng.below(1 << 50)),
                filled_notional_atoms: u128::from(rng.below(1 << 30) + 1),
                fees_paid_atoms: rng.below(1 << 36),
                attested: true,
            })
            .collect();

        let entity_score = 1 + rng.below(1 << 50);
        let entity_filled = 1 + rng.below(1 << 30);
        let entity_fees = rng.below(1 << 36);
        let k = 2 + rng.below(3) as usize;
        let score_parts = partition(&mut rng, entity_score, k);
        let fee_parts = partition(&mut rng, entity_fees, k);
        let fill_parts = partition(&mut rng, entity_filled, k);

        let combined_entity = MakerEpochStats {
            account: id("entity"),
            resting_notional_seconds: u128::from(entity_score),
            filled_notional_atoms: u128::from(entity_filled),
            fees_paid_atoms: entity_fees,
            attested: true,
        };
        let mut combined_set = others.clone();
        combined_set.push(combined_entity);
        let combined_plan = plan_epoch_rewards(&p, fee_pool, emission, &combined_set).unwrap();
        let combined_total = payout(&combined_plan, "entity");

        let mut split_set = others.clone();
        for j in 0..k {
            split_set.push(MakerEpochStats {
                account: id(&format!("entity-{j}")),
                resting_notional_seconds: u128::from(score_parts[j]),
                filled_notional_atoms: u128::from(fill_parts[j]),
                fees_paid_atoms: fee_parts[j],
                attested: true,
            });
        }
        let split_plan = plan_epoch_rewards(&p, fee_pool, emission, &split_set).unwrap();
        let split_total: u64 = (0..k)
            .map(|j| payout(&split_plan, &format!("entity-{j}")))
            .sum();
        assert!(
            split_total <= combined_total,
            "trial {trial}: split {split_total} > combined {combined_total}"
        );

        // Fee-funded aggregate over all identities must respect the entity's own fees.
        let fee_split = plan_epoch_rewards(&p, fee_pool, 0, &split_set).unwrap();
        let split_fee_total: u64 = (0..k)
            .map(|j| payout(&fee_split, &format!("entity-{j}")))
            .sum();
        let entity_rebate_cap =
            u128::from(entity_fees) * u128::from(p.fee_rebate_cap_bps) / u128::from(FULL);
        assert!(
            u128::from(split_fee_total) <= entity_rebate_cap,
            "trial {trial}: fee-funded aggregate above entity rebate cap"
        );
    }
}
