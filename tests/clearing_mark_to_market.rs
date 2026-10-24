//! Stage 25C: zero-crate single-ledger mark-to-market, never live settlement.
use std::collections::{BTreeMap, BTreeSet};
use tradelayer_core::clearing_math::PnlRounding;
use tradelayer_core::clearing_pnl::{project_pnl_unverified, PnlError};
use tradelayer_core::clearing_sandbox::{
    ClearingError, ClearingSandbox, CollateralPolicy, MarkPolicy, MatchedContractTrade,
};
use tradelayer_core::ledger::{
    AccountId, Ledger, LedgerError, Operation, PropertyId, PropertyKind, TxId,
};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::spot::{SpotExchange, SpotIntent, UnverifiedSpotPlacement};

const ONE: u64 = 100_000_000;

fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn a(text: &str) -> AccountId {
    AccountId::try_from(text).unwrap()
}
fn tx(n: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn base() -> ClearingSandbox {
    let mut ledger = Ledger::new();
    ledger
        .apply_batch([
            (
                tx(1),
                Operation::CreateProperty {
                    id: p(7),
                    issuer: a("alice"),
                    kind: PropertyKind::Fixed,
                    initial_atoms: 20_000_000_000,
                },
            ),
            (
                tx(2),
                Operation::CreateProperty {
                    id: p(8),
                    issuer: a("bob"),
                    kind: PropertyKind::Fixed,
                    initial_atoms: 15_000_000_000,
                },
            ),
            (
                tx(3),
                Operation::Transfer {
                    id: p(7),
                    from: a("alice"),
                    to: a("bob"),
                    amount_atoms: 8_000_000_000,
                },
            ),
            (
                tx(4),
                Operation::Transfer {
                    id: p(7),
                    from: a("alice"),
                    to: a("carol"),
                    amount_atoms: 4_000_000_000,
                },
            ),
            (
                tx(5),
                Operation::Transfer {
                    id: p(8),
                    from: a("bob"),
                    to: a("alice"),
                    amount_atoms: 5_000_000_000,
                },
            ),
        ])
        .unwrap();
    let spot = SpotExchange::from_ledger_unverified(ledger, BTreeSet::from([p(7), p(8)])).unwrap();
    ClearingSandbox::from_spot_unverified(TradeLayerMainnet::Litecoin, 19, p(7), spot).unwrap()
}
fn default_policy() -> MarkPolicy {
    MarkPolicy {
        height: 100,
        price_e8: 100 * ONE,
        notional_e8: ONE,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn opened() -> ClearingSandbox {
    let mut state = base()
        .with_mark_policy_unverified(default_policy())
        .unwrap();
    state
        .deposit_margin_unverified(tx(10), a("alice"), 2_000_000_000)
        .unwrap();
    state
        .deposit_margin_unverified(tx(11), a("bob"), 2_000_000_000)
        .unwrap();
    state
        .apply_matched_trades_unverified(
            tx(12),
            &[MatchedContractTrade {
                long: a("alice"),
                short: a("bob"),
                contracts: 1,
            }],
        )
        .unwrap();
    state
}
fn spot_order(id: u64, actor: &str, offer: u32, desire: u32) -> UnverifiedSpotPlacement {
    UnverifiedSpotPlacement {
        txid: tx(id),
        actor: a(actor),
        height: 101,
        intent: SpotIntent {
            offered: p(offer),
            desired: p(desire),
            offered_atoms: ONE,
            expected_atoms: ONE,
            post: false,
            stop: false,
        },
    }
}

#[test]
fn independent_bigint_vectors_cover_both_rounding_policies_and_residuals() {
    const ROWS: &str = include_str!("../fixtures/clearing_pnl_vectors.tsv");
    let mut count = 0;
    let mut nonzero_residuals = 0;
    for row in ROWS.lines() {
        if row.is_empty() || row.starts_with('#') {
            continue;
        }
        let cols: Vec<_> = row.split('\t').collect();
        assert_eq!(cols.len(), 11, "bad TSV fixture: {row}");
        let positions: BTreeMap<_, _> = if cols[6].is_empty() {
            BTreeMap::new()
        } else {
            cols[6]
                .split(',')
                .map(|element| {
                    let (name, count) = element.split_once(':').unwrap();
                    (a(name), count.parse::<i64>().unwrap())
                })
                .collect()
        };
        let rounding = match cols[5] {
            "legacy" => PnlRounding::Legacy20DecimalPlaces,
            "exact" => PnlRounding::ExactRational,
            _ => panic!("bad rounding in vector"),
        };
        let projected = project_pnl_unverified(
            &positions,
            cols[1].parse().unwrap(),
            cols[2].parse().unwrap(),
            cols[3].parse().unwrap(),
            cols[4] == "1",
            rounding,
        )
        .unwrap();
        let values = projected
            .deltas
            .iter()
            .map(|entry| format!("{}:{}", entry.account.as_str(), entry.delta_atoms))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(values, cols[7], "vector: {}", cols[0]);
        assert_eq!(
            projected.net_atoms,
            cols[8].parse::<i128>().unwrap(),
            "{}",
            cols[0]
        );
        assert_eq!(
            projected.payer_atoms,
            cols[9].parse::<u128>().unwrap(),
            "{}",
            cols[0]
        );
        assert_eq!(
            projected.receiver_atoms,
            cols[10].parse::<u128>().unwrap(),
            "{}",
            cols[0]
        );
        if projected.net_atoms != 0 {
            nonzero_residuals += 1;
            assert_eq!(cols[0], "linear_round_residual");
        }
        count += 1;
    }
    assert_eq!(count, 11);
    assert_eq!(nonzero_residuals, 1);
}

#[test]
fn a_linear_mark_moves_only_existing_collateral_between_owned_escrows() {
    let mut state = opened();
    let original = state.clone();
    let receipt = state
        .settle_mark_to_market_unverified(tx(13), 101, 101 * ONE)
        .unwrap();
    assert_eq!(receipt.gross_transfer_atoms, u128::from(ONE));
    assert_eq!(receipt.ledger_transfer_legs, 1);
    assert_eq!(receipt.active_positions, 2);
    assert_eq!(receipt.from_price_e8, 100 * ONE);
    assert_eq!(state.mark_policy().unwrap().price_e8, 101 * ONE);
    assert_eq!(state.mark_policy().unwrap().height, 101);
    assert_eq!(state.margin_atoms(&a("alice")), 2_100_000_000);
    assert_eq!(state.margin_atoms(&a("bob")), 1_900_000_000);
    assert_eq!(
        state.ledger().available_atoms(p(7), &a("alice")),
        original.ledger().available_atoms(p(7), &a("alice"))
    );
    assert_eq!(
        state.ledger().property(p(7)).unwrap().total_supply_atoms,
        original.ledger().property(p(7)).unwrap().total_supply_atoms
    );
    assert_eq!(state.spot().fills(), original.spot().fills());
    assert!(std::ptr::eq(state.ledger(), state.spot().ledger()));
    assert_eq!(state.verify(), Ok(()));
}

#[test]
fn late_pnl_transfer_failure_preserves_every_balance_mark_and_replay_id() {
    let snapshot = opened();
    for failure_leg in 0..=1 {
        let mut state = snapshot.clone();
        assert_eq!(
            state.settle_mark_fault_injected_unverified(tx(13), 101, 101 * ONE, failure_leg,),
            Err(ClearingError::InjectedFailure {
                before_leg: failure_leg
            })
        );
        assert_eq!(state, snapshot);
        assert_eq!(state.processed_count(), snapshot.processed_count());
        let receipt = state
            .settle_mark_to_market_unverified(tx(13), 101, 101 * ONE)
            .unwrap();
        assert_eq!(receipt.ledger_transfer_legs, 1);
        assert_eq!(state.processed_count(), snapshot.processed_count() + 1);
    }
}

#[test]
fn mark_height_and_external_id_cannot_replay_or_regress() {
    let mut state = opened();
    let snapshot = state.clone();
    assert_eq!(
        state.settle_mark_to_market_unverified(tx(13), 100, 101 * ONE),
        Err(ClearingError::StaleMark)
    );
    assert_eq!(
        state.settle_mark_to_market_unverified(tx(13), 101, 0),
        Err(ClearingError::InvalidMarkPolicy)
    );
    assert_eq!(state, snapshot);
    state
        .settle_mark_to_market_unverified(tx(13), 101, 101 * ONE)
        .unwrap();
    let committed = state.clone();
    assert_eq!(
        state.settle_mark_to_market_unverified(tx(13), 102, 102 * ONE),
        Err(ClearingError::Replay(tx(13)))
    );
    assert_eq!(
        state.settle_mark_to_market_unverified(tx(14), 101, 102 * ONE),
        Err(ClearingError::StaleMark)
    );
    assert_eq!(state, committed);
}

#[test]
fn two_mark_steps_can_reverse_price_without_double_paying() {
    let mut state = opened();
    state
        .settle_mark_to_market_unverified(tx(13), 101, 101 * ONE)
        .unwrap();
    state
        .settle_mark_to_market_unverified(tx(14), 102, 99 * ONE)
        .unwrap();
    assert_eq!(state.margin_atoms(&a("alice")), 1_900_000_000);
    assert_eq!(state.margin_atoms(&a("bob")), 2_100_000_000);
    assert_eq!(state.verify(), Ok(()));
}

#[test]
fn inverse_marking_and_linear_marking_share_no_unreviewed_formula_defaults() {
    let mut policy = default_policy();
    policy.inverse = true;
    policy.notional_e8 = 100 * ONE;
    policy.rounding = PnlRounding::Legacy20DecimalPlaces;
    let mut state = base().with_mark_policy_unverified(policy).unwrap();
    for (id, owner) in [(10, "alice"), (11, "bob")] {
        state
            .deposit_margin_unverified(tx(id), a(owner), 2_000_000_000)
            .unwrap();
    }
    state
        .apply_matched_trades_unverified(
            tx(12),
            &[MatchedContractTrade {
                long: a("alice"),
                short: a("bob"),
                contracts: 1,
            }],
        )
        .unwrap();
    let receipt = state
        .settle_mark_to_market_unverified(tx(13), 101, 120 * ONE)
        .unwrap();
    assert_eq!(receipt.gross_transfer_atoms, 16_666_667);
    assert_eq!(state.margin_atoms(&a("alice")), 2_016_666_667);
    assert_eq!(state.margin_atoms(&a("bob")), 1_983_333_333);
}

#[test]
fn rounded_multi_party_imbalance_is_observable_but_cannot_mutate_state() {
    let mut policy = default_policy();
    policy.notional_e8 = ONE / 2;
    let mut state = base().with_mark_policy_unverified(policy).unwrap();
    for (id, name) in [(10, "alice"), (11, "bob"), (12, "carol")] {
        state
            .deposit_margin_unverified(tx(id), a(name), u128::from(ONE))
            .unwrap();
    }
    state
        .apply_matched_trades_unverified(
            tx(13),
            &[
                MatchedContractTrade {
                    long: a("alice"),
                    short: a("bob"),
                    contracts: 1,
                },
                MatchedContractTrade {
                    long: a("carol"),
                    short: a("bob"),
                    contracts: 1,
                },
            ],
        )
        .unwrap();
    let preview = state
        .preview_mark_to_market_unverified(101, 100 * ONE + 1)
        .unwrap();
    assert_eq!(preview.net_atoms, 1);
    assert_eq!(preview.payer_atoms, 1);
    assert_eq!(preview.receiver_atoms, 2);
    let before = state.clone();
    assert_eq!(
        state.settle_mark_to_market_unverified(tx(14), 101, 100 * ONE + 1),
        Err(ClearingError::PnlRoundingImbalance(1))
    );
    assert_eq!(state, before);
    // A price difference of two atoms produces exact 1+1 vs -2,
    // without unratified supply inflation or deflation.
    let receipt = state
        .settle_mark_to_market_unverified(tx(14), 101, 100 * ONE + 2)
        .unwrap();
    assert_eq!(receipt.gross_transfer_atoms, 2);
    assert_eq!(state.verify(), Ok(()));
}

#[test]
fn unavailable_margin_and_spot_order_reserves_cannot_pay_mark_losses() {
    let mut state = base()
        .with_mark_policy_unverified(default_policy())
        .unwrap();
    state
        .deposit_margin_unverified(tx(10), a("alice"), 100_000_000)
        .unwrap();
    state
        .deposit_margin_unverified(tx(11), a("bob"), 100_000_000)
        .unwrap();
    state
        .apply_matched_trades_unverified(
            tx(12),
            &[MatchedContractTrade {
                long: a("alice"),
                short: a("bob"),
                contracts: 1,
            }],
        )
        .unwrap();
    state
        .enqueue_spot_order_unverified(spot_order(13, "bob", 7, 8))
        .unwrap();
    state.process_spot_block_unverified(101).unwrap();
    assert_eq!(state.spot().open_count(), 1);
    let before = state.clone();
    let failing = state.settle_mark_to_market_unverified(tx(14), 102, 103 * ONE);
    assert!(matches!(
        failing,
        Err(ClearingError::Ledger(
            LedgerError::InsufficientAvailable { .. }
        ))
    ));
    assert_eq!(state, before);
    assert_eq!(state.spot().open_count(), 1);
    assert_eq!(state.verify(), Ok(()));
}

#[test]
fn maintenance_failure_after_transfers_reverts_everything() {
    let policy = CollateralPolicy {
        initial_atoms_per_contract: 450_000_000,
        maintenance_atoms_per_contract: 450_000_000,
    };
    let mut state = base()
        .with_collateral_policy_unverified(policy)
        .unwrap()
        .with_mark_policy_unverified(default_policy())
        .unwrap();
    state
        .deposit_margin_unverified(tx(10), a("alice"), 500_000_000)
        .unwrap();
    state
        .deposit_margin_unverified(tx(11), a("bob"), 500_000_000)
        .unwrap();
    state
        .apply_matched_trades_unverified(
            tx(12),
            &[MatchedContractTrade {
                long: a("alice"),
                short: a("bob"),
                contracts: 1,
            }],
        )
        .unwrap();
    let before = state.clone();
    assert_eq!(
        state.settle_mark_to_market_unverified(tx(13), 101, 101 * ONE),
        Err(ClearingError::InsufficientMargin {
            account: a("bob"),
            required_atoms: 450_000_000,
            available_atoms: 400_000_000,
        })
    );
    assert_eq!(state, before);
    state
        .deposit_margin_unverified(tx(14), a("bob"), 100_000_000)
        .unwrap();
    state
        .settle_mark_to_market_unverified(tx(13), 101, 101 * ONE)
        .unwrap();
    assert_eq!(state.margin_atoms(&a("bob")), 500_000_000);
    assert_eq!(state.verify(), Ok(()));
}

#[test]
fn empty_mark_still_fences_spot_checkpoint_and_joint_migration() {
    let mut state = base()
        .with_mark_policy_unverified(default_policy())
        .unwrap();
    assert!(state.spot().snapshot_bytes().is_ok());
    let receipt = state
        .settle_mark_to_market_unverified(tx(10), 101, 101 * ONE)
        .unwrap();
    assert_eq!(receipt.ledger_transfer_legs, 0);
    assert_eq!(state.ledger().verify_conservation(), Ok(()));
    assert!(state.spot().snapshot_bytes().is_err());
    assert_eq!(state.processed_count(), 1);
    assert!(ClearingSandbox::from_spot_unverified(
        TradeLayerMainnet::Litecoin,
        19,
        p(7),
        state.spot().clone(),
    )
    .is_err());
}

#[test]
fn empty_funding_also_fences_unrepresented_replay_history() {
    let mut state = base();
    assert!(state.spot().snapshot_bytes().is_ok());
    let receipt = state.settle_funding_unverified(tx(10), ONE, 0).unwrap();
    assert_eq!(receipt.ledger_transfer_legs, 0);
    assert!(state.spot().snapshot_bytes().is_err());
    assert_eq!(state.processed_count(), 1);
}

#[test]
fn invalid_mark_policy_frozen_after_clearing_activity() {
    let mut bad = default_policy();
    bad.height = 0;
    assert_eq!(
        base().with_mark_policy_unverified(bad),
        Err(ClearingError::InvalidMarkPolicy)
    );
    let state = opened();
    let snapshot = state.clone();
    assert_eq!(
        state.clone().with_mark_policy_unverified(default_policy()),
        Err(ClearingError::MarkPolicyFrozen)
    );
    assert_eq!(state, snapshot);
    assert_eq!(
        state
            .preview_mark_to_market_unverified(101, 101 * ONE)
            .unwrap()
            .net_atoms,
        0
    );
    let no_mark = base();
    assert_eq!(
        no_mark.preview_mark_to_market_unverified(101, 101 * ONE),
        Err(ClearingError::MarkPolicyMissing)
    );
    assert_eq!(
        project_pnl_unverified(
            &BTreeMap::new(),
            0,
            ONE,
            ONE,
            false,
            PnlRounding::ExactRational
        ),
        Err(PnlError::InvalidMark)
    );
}

#[test]
fn one_ledger_forbids_backdated_marks_and_spot_orders() {
    let mut state = base()
        .with_mark_policy_unverified(default_policy())
        .unwrap();
    state.process_spot_block_unverified(110).unwrap();
    let before = state.clone();
    assert_eq!(
        state.settle_mark_to_market_unverified(tx(10), 109, 101 * ONE),
        Err(ClearingError::StaleMark)
    );
    assert_eq!(state, before);
    // A mark at the already completed spot height is permitted as an
    // end-of-block settlement, with no source-authority inference.
    state.settle_mark_to_market_unverified(tx(10), 110, 101 * ONE).unwrap();
    let after_mark = state.clone();
    assert_eq!(
        state.enqueue_spot_order_unverified(spot_order(11, "alice", 7, 8)),
        Err(ClearingError::StaleMark)
    );
    assert_eq!(
        state.process_spot_block_unverified(110),
        Err(ClearingError::StaleMark)
    );
    assert_eq!(state, after_mark);
    assert!(state.spot().snapshot_bytes().is_err());
}

#[test]
fn mark_policy_cannot_retroactively_precede_spot_finalization() {
    let mut state = base();
    state.process_spot_block_unverified(120).unwrap();
    assert_eq!(
        state.with_mark_policy_unverified(default_policy()),
        Err(ClearingError::StaleMark)
    );
}

#[test]
fn deterministic_mark_stress_preserves_both_property_supplies_and_oi() {
    let mut state = opened();
    let p7_supply = state.ledger().property(p(7)).unwrap().total_supply_atoms;
    let p8_supply = state.ledger().property(p(8)).unwrap().total_supply_atoms;
    for step in 0..100u64 {
        let price = 100 * ONE + (step + 1);
        let prior = state.clone();
        let receipt = state
            .settle_mark_to_market_unverified(tx(1000 + step), 101 + step, price)
            .unwrap();
        assert_eq!(receipt.gross_transfer_atoms, 1);
        assert_eq!(receipt.ledger_transfer_legs, 1);
        assert_eq!(
            state.ledger().property(p(7)).unwrap().total_supply_atoms,
            p7_supply
        );
        assert_eq!(
            state.ledger().property(p(8)).unwrap().total_supply_atoms,
            p8_supply
        );
        assert_eq!(state.positions(), prior.positions());
        assert_eq!(state.ledger().verify_conservation(), Ok(()));
        assert_eq!(state.verify(), Ok(()));
    }
    assert_eq!(state.margin_atoms(&a("alice")), 2_000_000_100);
    assert_eq!(state.margin_atoms(&a("bob")), 1_999_999_900);
}
