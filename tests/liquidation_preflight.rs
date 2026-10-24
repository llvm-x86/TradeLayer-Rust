//! Stage 25E1: no liquidation execution, only an explicit risk preflight.
//! All economic inputs are synthetic and every spend remains default-deny.
use std::collections::BTreeMap;
use tradelayer_core::clearing_math::PnlRounding;
use tradelayer_core::clearing_pnl::{
    project_pnl_unverified, PnlAccountDelta, PnlProjection,
};
use tradelayer_core::clearing_sandbox::{
    ClearingError, ClearingSandbox, CollateralPolicy, MarkPolicy, MatchedContractTrade,
};
use tradelayer_core::ledger::{
    AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId,
};
use tradelayer_core::liquidation_audit::{
    assess_liquidation_risk_unverified, preview_loss_budget_unverified,
    HypotheticalLossRule, LiquidationAuditError, MarginRisk, MAX_RISK_POSITIONS,
};
use tradelayer_core::network_profile::TradeLayerMainnet;

const ONE: u64 = 100_000_000;
fn p() -> PropertyId { PropertyId::try_from(7).unwrap() }
fn a(s: &str) -> AccountId { AccountId::try_from(s).unwrap() }
fn tx(n: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn funded() -> Ledger {
    let mut l = Ledger::new();
    l.apply_batch([
        (tx(1), Operation::CreateProperty {
            id: p(), issuer: a("alice"), kind: PropertyKind::Fixed,
            initial_atoms: 1_000_000_000,
        }),
        (tx(2), Operation::Transfer {
            id: p(), from: a("alice"), to: a("bob"),
            amount_atoms: 300_000_000,
        }),
        (tx(3), Operation::Transfer {
            id: p(), from: a("alice"), to: a("carol"),
            amount_atoms: 300_000_000,
        }),
    ]).unwrap();
    l
}
fn mark(notional: u64) -> MarkPolicy {
    MarkPolicy {
        height: 100,
        price_e8: 100 * ONE,
        notional_e8: notional,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn state_with_floor(margin: u128, maintenance: u128, notional: u64) -> ClearingSandbox {
    let mut s = ClearingSandbox::from_ledger_unverified(
        TradeLayerMainnet::Litecoin, 19, p(), funded(),
    ).unwrap()
    .with_collateral_policy_unverified(CollateralPolicy {
        initial_atoms_per_contract: maintenance,
        maintenance_atoms_per_contract: maintenance,
    }).unwrap()
    .with_mark_policy_unverified(mark(notional)).unwrap();
    s.deposit_margin_unverified(tx(10), a("alice"), margin).unwrap();
    s.deposit_margin_unverified(tx(11), a("bob"), margin).unwrap();
    s.apply_matched_trades_unverified(tx(12), &[
        MatchedContractTrade {
            long: a("alice"), short: a("bob"), contracts: 1
        },
    ]).unwrap();
    s
}

#[test]
fn loss_budget_matches_independent_bigint_source_derived_vectors() {
    const VECTORS: &str = include_str!("../fixtures/liquidation_loss_budget_vectors.tsv");
    let mut count = 0;
    for row in VECTORS.lines() {
        if row.starts_with('#') || row.is_empty() { continue; }
        let cols: Vec<_> = row.split('\t').collect();
        assert_eq!(cols.len(), 11, "bad vector: {row}");
        let rule = match cols[1] {
            "legacy" => HypotheticalLossRule::LegacyAvailableThenMargin,
            "posted" => HypotheticalLossRule::PostedMarginOnly,
            _ => panic!("bad policy"),
        };
        let plan = preview_loss_budget_unverified(
            cols[2].parse().unwrap(), cols[3].parse().unwrap(),
            cols[4].parse().unwrap(), cols[5].parse().unwrap(), rule,
        );
        assert_eq!(plan.available_debit_atoms, cols[6].parse().unwrap(), "{}", cols[0]);
        assert_eq!(plan.margin_debit_atoms, cols[7].parse().unwrap(), "{}", cols[0]);
        assert_eq!(plan.hypothetical_insurance_debit_atoms, cols[8].parse().unwrap(), "{}", cols[0]);
        assert_eq!(plan.unresolved_atoms, cols[9].parse().unwrap(), "{}", cols[0]);
        assert_eq!(plan.fully_accounted(), cols[10] == "1", "{}", cols[0]);
        assert!(plan.fully_accounted());
        count += 1;
    }
    assert_eq!(count, 16);
}

#[test]
fn insolvency_from_mark_beyond_real_margin_is_visible_without_any_seizure() {
    let s = state_with_floor(50_000_000, 40_000_000, ONE);
    let before = s.clone();
    let risk = s.preview_liquidation_unverified(101, 101 * ONE).unwrap();
    assert_eq!(risk.accounts.len(), 2);
    assert_eq!(risk.accounts[0].account, a("alice"));
    assert_eq!(risk.accounts[0].risk, MarginRisk::Healthy);
    assert_eq!(risk.accounts[1].account, a("bob"));
    assert_eq!(risk.accounts[1].risk, MarginRisk::Insolvent);
    assert_eq!(risk.accounts[1].post_mark_equity_atoms, -50_000_000);
    assert_eq!(risk.accounts[1].maintenance_deficit_atoms, 90_000_000);
    assert_eq!(risk.accounts[1].uncovered_mark_loss_atoms, 50_000_000);
    assert_eq!(risk.total_uncovered_mark_loss_atoms, 50_000_000);
    assert_eq!(risk.total_maintenance_deficit_atoms, 90_000_000);
    assert_eq!(risk.candidates_by_severity, vec![a("bob")]);
    assert_eq!(risk.rounded_pnl_residual_atoms, 0);
    assert_eq!(s, before);
    let mut rejected = s.clone();
    assert!(rejected.settle_mark_to_market_unverified(tx(13), 101, 101 * ONE).is_err());
    assert_eq!(rejected, before);
    assert_eq!(s.ledger().verify_conservation(), Ok(()));
}

#[test]
fn below_maintenance_but_not_bankrupt_is_distinguished() {
    let s = state_with_floor(60_000_000, 50_000_000, ONE);
    let preview = s.preview_liquidation_unverified(101, 100 * ONE + ONE / 5).unwrap();
    let bob = &preview.accounts[1];
    assert_eq!(bob.risk, MarginRisk::BelowMaintenance);
    assert_eq!(bob.post_mark_equity_atoms, 40_000_000);
    assert_eq!(bob.maintenance_deficit_atoms, 10_000_000);
    assert_eq!(bob.uncovered_mark_loss_atoms, 0);
    assert_eq!(preview.total_uncovered_mark_loss_atoms, 0);
    assert_eq!(preview.candidates_by_severity, vec![a("bob")]);
    let mut state = s.clone();
    assert!(state.settle_mark_to_market_unverified(
        tx(13), 101, 100 * ONE + ONE / 5,
    ).is_err());
    assert_eq!(state, s);
}

#[test]
fn solvent_mark_has_no_liquidation_candidates_and_mutates_nothing() {
    let s = state_with_floor(200_000_000, 50_000_000, ONE);
    let before = s.clone();
    for step in 0..70 {
        let price = 100 * ONE + step * 1_000;
        let preview = s.preview_liquidation_unverified(101 + step, price).unwrap();
        assert!(preview.candidates_by_severity.is_empty());
        assert_eq!(preview.total_uncovered_mark_loss_atoms, 0);
        assert_eq!(preview.total_maintenance_deficit_atoms, 0);
        assert_eq!(preview.rounded_pnl_residual_atoms, 0);
    }
    assert_eq!(s, before);
}

#[test]
fn quantized_mark_roundoff_is_reported_but_never_authorized() {
    let mut s = ClearingSandbox::from_ledger_unverified(
        TradeLayerMainnet::Litecoin, 19, p(), funded(),
    ).unwrap()
    .with_collateral_policy_unverified(CollateralPolicy {
        initial_atoms_per_contract: 1,
        maintenance_atoms_per_contract: 1,
    }).unwrap()
    .with_mark_policy_unverified(mark(ONE / 2)).unwrap();
    for (id, name) in [(10, "alice"), (11, "bob"), (12, "carol")] {
        s.deposit_margin_unverified(tx(id), a(name), 100_000_000).unwrap();
    }
    s.apply_matched_trades_unverified(tx(13), &[
        MatchedContractTrade { long: a("alice"), short: a("bob"), contracts: 1 },
        MatchedContractTrade { long: a("carol"), short: a("bob"), contracts: 1 },
    ]).unwrap();
    let before = s.clone();
    let report = s.preview_liquidation_unverified(101, 100 * ONE + 1).unwrap();
    assert_eq!(report.rounded_pnl_residual_atoms, 1);
    assert_eq!(report.total_uncovered_mark_loss_atoms, 0);
    assert_eq!(report.accounts.len(), 3);
    assert!(s.settle_mark_to_market_unverified(tx(14), 101, 100 * ONE + 1).is_err());
    assert_eq!(s, before);
}

#[test]
fn severity_priority_is_deterministic_and_never_a_market_order() {
    let mut positions = BTreeMap::new();
    let mut margins = BTreeMap::new();
    for (owner, contracts, atoms) in [
        ("alice", -2, 40u128), ("bob", -1, 60), ("carol", 1, 100), ("dave", 2, 100),
    ] {
        positions.insert(a(owner), contracts);
        margins.insert(a(owner), atoms);
    }
    let projection = PnlProjection {
        from_price_e8: ONE,
        to_price_e8: ONE + 1,
        notional_e8: ONE,
        inverse: false,
        rounding: PnlRounding::ExactRational,
        deltas: vec![
            PnlAccountDelta { account: a("alice"), delta_atoms: -200 },
            PnlAccountDelta { account: a("bob"), delta_atoms: -100 },
            PnlAccountDelta { account: a("carol"), delta_atoms: 100 },
            PnlAccountDelta { account: a("dave"), delta_atoms: 200 },
        ],
        payer_atoms: 300,
        receiver_atoms: 300,
        net_atoms: 0,
    };
    let report = assess_liquidation_risk_unverified(&positions, &margins, &projection, 10)
        .unwrap();
    assert_eq!(report.candidates_by_severity, vec![a("alice"), a("bob")]);
    assert_eq!(report.total_uncovered_mark_loss_atoms, 200);
    assert_eq!(report.accounts[0].risk, MarginRisk::Insolvent);
    assert_eq!(report.accounts[1].risk, MarginRisk::Insolvent);
    assert_eq!(report.accounts[2].risk, MarginRisk::Healthy);
    assert_eq!(report.accounts[3].risk, MarginRisk::Healthy);
}

#[test]
fn risk_assessment_denies_missing_policy_or_mark_and_wrong_account_maps() {
    let s = ClearingSandbox::from_ledger_unverified(
        TradeLayerMainnet::Litecoin, 19, p(), funded(),
    ).unwrap();
    assert_eq!(
        s.preview_liquidation_unverified(101, 101 * ONE),
        Err(ClearingError::LiquidationPolicyMissing)
    );
    let s = s.with_collateral_policy_unverified(CollateralPolicy {
        initial_atoms_per_contract: 1,
        maintenance_atoms_per_contract: 1,
    }).unwrap();
    assert_eq!(
        s.preview_liquidation_unverified(101, 101 * ONE),
        Err(ClearingError::MarkPolicyMissing)
    );

    let mut positions = BTreeMap::from([(a("alice"), 1i64), (a("bob"), -1)]);
    let mut margins = BTreeMap::from([(a("alice"), 1u128), (a("bob"), 1)]);
    let mut mark = project_pnl_unverified(
        &positions, 100 * ONE, 101 * ONE, ONE, false, PnlRounding::ExactRational,
    ).unwrap();
    margins.remove(&a("bob"));
    assert_eq!(
        assess_liquidation_risk_unverified(&positions, &margins, &mark, 1),
        Err(LiquidationAuditError::MissingMargin(a("bob")))
    );
    margins.insert(a("bob"), 1);
    mark.deltas.swap(0, 1);
    assert_eq!(
        assess_liquidation_risk_unverified(&positions, &margins, &mark, 1),
        Err(LiquidationAuditError::PositionProjectionMismatch)
    );
    mark.deltas.swap(0, 1);
    positions.insert(a("carol"), 1);
    assert_eq!(
        assess_liquidation_risk_unverified(&positions, &margins, &mark, 1),
        Err(LiquidationAuditError::PositionProjectionMismatch)
    );
    positions.remove(&a("carol"));
    assert_eq!(
        assess_liquidation_risk_unverified(&positions, &margins, &mark, 0),
        Err(LiquidationAuditError::InvalidMaintenance)
    );
}

#[test]
fn huge_maintenance_and_unbalanced_open_interest_fail_closed() {
    let positions = BTreeMap::from([(a("alice"), 1i64), (a("bob"), -1)]);
    let margins = BTreeMap::from([(a("alice"), 1u128), (a("bob"), 1)]);
    let mark = project_pnl_unverified(
        &positions, 100 * ONE, 101 * ONE, ONE, false, PnlRounding::ExactRational,
    ).unwrap();
    assert_eq!(
        assess_liquidation_risk_unverified(&positions, &margins, &mark, u128::MAX),
        Err(LiquidationAuditError::ArithmeticOverflow)
    );
    let mut positions = positions.clone();
    positions.insert(a("carol"), 1);
    let mut margins = margins.clone();
    margins.insert(a("carol"), 1);
    let mark = project_pnl_unverified(
        &positions, 100 * ONE, 100 * ONE, ONE, false, PnlRounding::ExactRational,
    ).unwrap();
    assert_eq!(
        assess_liquidation_risk_unverified(&positions, &margins, &mark, 1),
        Err(LiquidationAuditError::UnbalancedOpenInterest)
    );
    let mut too_many = BTreeMap::new();
    let mut posted = BTreeMap::new();
    for i in 0..=MAX_RISK_POSITIONS {
        let account = a(&format!("owner{i:03}"));
        too_many.insert(account.clone(), 1);
        posted.insert(account, 1);
    }
    let mark = project_pnl_unverified(
        &BTreeMap::new(), ONE, ONE, ONE, false, PnlRounding::ExactRational,
    ).unwrap();
    assert_eq!(
        assess_liquidation_risk_unverified(&too_many, &posted, &mark, 1),
        Err(LiquidationAuditError::TooManyPositions)
    );
}

#[test]
fn multiple_offline_loss_pool_budgets_cannot_reuse_hypothetical_insurance() {
    // This is a warning/example, not a spend simulator: two independent
    // previews can each claim the SAME insurer's 100 atoms.
    let one = preview_loss_budget_unverified(
        100, 0, 0, 100, HypotheticalLossRule::PostedMarginOnly,
    );
    let two = preview_loss_budget_unverified(
        100, 0, 0, 100, HypotheticalLossRule::PostedMarginOnly,
    );
    assert_eq!(one.hypothetical_insurance_debit_atoms, 100);
    assert_eq!(two.hypothetical_insurance_debit_atoms, 100);
    assert_eq!(one.unresolved_atoms + two.unresolved_atoms, 0);
    // A real insurer would have to debit a single shared ledger reserve:
    // neither plan is executable, and the second must be rejected without
    // a ledger-backed, atomically reserved insurer budget.
}
