//! Stage 25F: strict synthetic bilateral partial reductions, never forced liquidation.
//! Only signed existing participants, frozen MarkPolicy math, real escrow funds.
use std::collections::BTreeSet;
use tradelayer_core::chain::{BlockHeader, BlockId, Tip};
use tradelayer_core::clearing_chain::{
    ClearingChainError, ClearingChainState, UnverifiedClearingBlock, UnverifiedClearingEvent,
};
use tradelayer_core::clearing_math::PnlRounding;
use tradelayer_core::clearing_sandbox::{
    ClearingError, ClearingSandbox, CollateralPolicy, InsurancePolicy, MarkPolicy, MarkTerms,
    MatchedContractTrade, SyntheticBilateralReduction,
};
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::spot::{SpotExchange, SpotIntent, SpotPair, UnverifiedSpotPlacement};

const ONE: u64 = 100_000_000;
fn a(text: &str) -> AccountId {
    AccountId::try_from(text).unwrap()
}
fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn tx(n: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn policy() -> CollateralPolicy {
    CollateralPolicy {
        initial_atoms_per_contract: 10_000_000,
        maintenance_atoms_per_contract: 5_000_000,
    }
}
fn mark() -> MarkPolicy {
    MarkPolicy {
        height: 100,
        price_e8: 100 * ONE,
        notional_e8: ONE,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn insurer() -> InsurancePolicy {
    InsurancePolicy {
        sponsor: a("alice"),
    }
}
fn terms() -> MarkTerms {
    MarkTerms {
        notional_e8: ONE,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn permitted() -> BTreeSet<PropertyId> {
    BTreeSet::from([p(7), p(8)])
}
fn initial() -> ClearingSandbox {
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
                    issuer: a("carol"),
                    kind: PropertyKind::Fixed,
                    initial_atoms: 10_000_000_000,
                },
            ),
            (
                tx(3),
                Operation::Transfer {
                    id: p(7),
                    from: a("alice"),
                    to: a("bob"),
                    amount_atoms: 5_000_000_000,
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
                    id: p(7),
                    from: a("alice"),
                    to: a("dave"),
                    amount_atoms: 3_000_000_000,
                },
            ),
        ])
        .unwrap();
    let spot = SpotExchange::from_ledger_unverified(ledger, permitted()).unwrap();
    ClearingSandbox::from_spot_unverified(TradeLayerMainnet::Litecoin, 19, p(7), spot)
        .unwrap()
        .with_collateral_policy_unverified(policy())
        .unwrap()
        .with_mark_policy_unverified(mark())
        .unwrap()
        .with_insurance_policy_unverified(insurer())
        .unwrap()
}
fn opened() -> ClearingSandbox {
    let mut state = initial();
    state
        .deposit_insurance_unverified(tx(10), a("alice"), 2 * u128::from(ONE))
        .unwrap();
    for (n, owner, atoms) in [
        (11, "alice", 5 * u128::from(ONE)),
        (12, "bob", 5 * u128::from(ONE)),
        (13, "carol", 4 * u128::from(ONE)),
        (14, "dave", 4 * u128::from(ONE)),
    ] {
        state
            .deposit_margin_unverified(tx(n), a(owner), atoms)
            .unwrap();
    }
    state
        .apply_matched_trades_unverified(
            tx(15),
            &[
                MatchedContractTrade {
                    long: a("alice"),
                    short: a("bob"),
                    contracts: 3,
                },
                MatchedContractTrade {
                    long: a("carol"),
                    short: a("dave"),
                    contracts: 2,
                },
            ],
        )
        .unwrap();
    state
}
fn bilateral(q: u64, price: u64) -> SyntheticBilateralReduction {
    SyntheticBilateralReduction {
        long: a("alice"),
        short: a("bob"),
        contracts: q,
        execution_price_e8: price,
    }
}
fn restore(bytes: &[u8]) -> ClearingSandbox {
    ClearingSandbox::from_snapshot_bytes_with_insurance(
        bytes,
        TradeLayerMainnet::Litecoin,
        19,
        p(7),
        &permitted(),
        Some(policy()),
        Some(terms()),
        Some(&insurer()),
    )
    .unwrap()
}
fn anchor() -> Tip {
    Tip {
        height: 100,
        id: BlockId([1; 32]),
    }
}
fn block(id: u8, events: Vec<UnverifiedClearingEvent>) -> UnverifiedClearingBlock {
    UnverifiedClearingBlock {
        network: TradeLayerMainnet::Litecoin,
        header: BlockHeader {
            height: 101,
            parent: anchor().id,
            id: BlockId([id; 32]),
        },
        events,
    }
}
fn restore_chain(bytes: &[u8]) -> ClearingChainState {
    ClearingChainState::from_checkpoint_bytes_with_insurance(
        bytes,
        TradeLayerMainnet::Litecoin,
        anchor(),
        19,
        p(7),
        &permitted(),
        Some(policy()),
        Some(terms()),
        Some(&insurer()),
        3,
    )
    .unwrap()
}
fn unrelated_spot_order() -> UnverifiedSpotPlacement {
    UnverifiedSpotPlacement {
        txid: tx(40),
        actor: a("carol"),
        height: 101,
        intent: SpotIntent {
            offered: p(8),
            desired: p(7),
            offered_atoms: ONE / 4,
            expected_atoms: ONE / 4,
            post: false,
            stop: false,
        },
    }
}
#[test]
fn exactly_one_real_margin_transfer_reduces_equal_opposite_lots() {
    let mut s = opened();
    let before = s.clone();
    let result = s
        .reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 101 * ONE))
        .unwrap();
    assert_eq!(result.reference_mark_price_e8, 100 * ONE);
    assert_eq!(result.execution_price_e8, 101 * ONE);
    assert_eq!(result.contracts_reduced, 1);
    assert_eq!(result.payer, Some(a("bob")));
    assert_eq!(result.receiver, Some(a("alice")));
    assert_eq!(result.realized_transfer_atoms, u128::from(ONE));
    assert_eq!(result.transfer_legs, 1);
    assert_eq!(s.positions()[&a("alice")], 2);
    assert_eq!(s.positions()[&a("bob")], -2);
    assert_eq!(s.positions()[&a("carol")], 2);
    assert_eq!(s.positions()[&a("dave")], -2);
    assert_eq!(s.margin_atoms(&a("alice")), 6 * u128::from(ONE));
    assert_eq!(s.margin_atoms(&a("bob")), 4 * u128::from(ONE));
    assert_eq!(
        s.margin_atoms(&a("carol")),
        before.margin_atoms(&a("carol"))
    );
    assert_eq!(s.margin_atoms(&a("dave")), before.margin_atoms(&a("dave")));
    assert_eq!(
        s.insurance_reserve_atoms(),
        before.insurance_reserve_atoms()
    );
    assert_eq!(s.mark_policy(), before.mark_policy());
    assert_eq!(s.spot().fills(), before.spot().fills());
    assert_eq!(s.ledger().verify_conservation(), Ok(()));
    assert_eq!(s.verify(), Ok(()));
    assert_eq!(restore(&s.snapshot_bytes().unwrap()), s);
}

#[test]
fn inverse_price_move_debits_exact_opposite_owner_and_never_insurer() {
    let mut s = opened();
    let before = s.clone();
    let receipt = s
        .reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 99 * ONE))
        .unwrap();
    assert_eq!(receipt.payer, Some(a("alice")));
    assert_eq!(receipt.receiver, Some(a("bob")));
    assert_eq!(receipt.realized_transfer_atoms, u128::from(ONE));
    assert_eq!(s.margin_atoms(&a("alice")), 4 * u128::from(ONE));
    assert_eq!(s.margin_atoms(&a("bob")), 6 * u128::from(ONE));
    assert_eq!(
        s.insurance_reserve_atoms(),
        before.insurance_reserve_atoms()
    );
}

#[test]
fn unrelated_existing_spot_order_and_margin_owners_remain_identical() {
    let mut s = opened();
    s.enqueue_spot_order_unverified(unrelated_spot_order())
        .unwrap();
    s.process_spot_block_unverified(101).unwrap();
    assert_eq!(s.spot().open_count(), 1);
    let pair = SpotPair::new(p(7), p(8)).unwrap();
    let orders = s.spot().open_orders(pair).to_vec();
    let before = s.clone();
    s.reduce_bilateral_positions_unverified(tx(50), 102, &bilateral(1, 101 * ONE))
        .unwrap();
    assert_eq!(s.spot().open_orders(pair), orders);
    assert_eq!(s.spot().open_count(), 1);
    assert_eq!(s.spot().fills(), before.spot().fills());
    assert_eq!(s.positions()[&a("carol")], 2);
    assert_eq!(s.positions()[&a("dave")], -2);
    assert_eq!(
        s.margin_atoms(&a("carol")),
        before.margin_atoms(&a("carol"))
    );
    assert_eq!(s.ledger().property(p(8)), before.ledger().property(p(8)));
    assert_eq!(s.verify(), Ok(()));
}

#[test]
fn a_queued_spot_order_due_at_fill_height_blocks_ambiguous_ordering() {
    let mut s = opened();
    s.enqueue_spot_order_unverified(unrelated_spot_order())
        .unwrap();
    let before = s.clone();
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 101 * ONE)),
        Err(ClearingError::PendingSpotOrders)
    );
    assert_eq!(s, before);
}

#[test]
fn full_close_and_position_flip_or_synthesized_counterparty_reject() {
    let mut s = opened();
    let before = s.clone();
    for q in [0, 3, 4, u64::MAX] {
        assert!(s
            .reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(q, 101 * ONE),)
            .is_err());
        assert_eq!(s, before);
    }
    let mut invented = bilateral(1, 101 * ONE);
    invented.short = a("mallory");
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &invented),
        Err(ClearingError::MissingOppositePosition)
    );
    invented.short = invented.long.clone();
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &invented),
        Err(ClearingError::SelfTrade)
    );
    invented.short = a("carol");
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &invented),
        Err(ClearingError::MissingOppositePosition)
    );
    assert_eq!(s, before);
}

#[test]
fn no_leg_close_consumes_replay_but_keeps_escrow_and_mark_unchanged() {
    let mut s = opened();
    let before = s.clone();
    let result = s
        .reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 100 * ONE))
        .unwrap();
    assert_eq!(result.transfer_legs, 0);
    assert_eq!(result.realized_transfer_atoms, 0);
    assert_eq!(result.payer, None);
    assert_eq!(result.receiver, None);
    assert_eq!(
        s.margin_atoms(&a("alice")),
        before.margin_atoms(&a("alice"))
    );
    assert_eq!(
        s.insurance_reserve_atoms(),
        before.insurance_reserve_atoms()
    );
    assert_eq!(s.processed_count(), before.processed_count() + 1);
    assert_eq!(restore(&s.snapshot_bytes().unwrap()), s);
    let after = s.clone();
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 100 * ONE)),
        Err(ClearingError::Replay(tx(50)))
    );
    assert_eq!(s, after);
}

#[test]
fn losing_margin_shortfall_must_reject_without_using_real_insurance() {
    let mut s = opened();
    let before = s.clone();
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 106 * ONE)),
        Err(ClearingError::InsufficientMargin {
            account: a("bob"),
            required_atoms: 6 * u128::from(ONE),
            available_atoms: 5 * u128::from(ONE),
        })
    );
    assert_eq!(s, before);
    assert_eq!(s.insurance_reserve_atoms(), 2 * u128::from(ONE));
}

#[test]
fn a_funded_payout_that_breaks_remaining_maintenance_is_atomic_rejection() {
    let mut s = opened();
    let before = s.clone();
    // Bob has 500M posted, paying 496M leaves 4M. After reducing one
    // of three shorts he still needs 2 * 5M maintenance, so must reject.
    let price = 100 * ONE + 496_000_000;
    assert!(matches!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, price)),
        Err(ClearingError::InsufficientMargin {
            account, required_atoms: 10_000_000, available_atoms: 4_000_000
        }) if account == a("bob")
    ));
    assert_eq!(s, before);
}

#[test]
fn failure_before_and_after_the_only_transfer_restores_all_tlcl_bytes() {
    let original = opened();
    for leg in [0, 1] {
        let mut s = original.clone();
        let before = s.snapshot_bytes().unwrap();
        assert_eq!(
            s.reduce_bilateral_fault_injected_unverified(
                tx(50),
                101,
                &bilateral(1, 101 * ONE),
                leg
            ),
            Err(ClearingError::InjectedFailure { before_leg: leg })
        );
        assert_eq!(s.snapshot_bytes().unwrap(), before);
        assert_eq!(s, original);
        s.reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 101 * ONE))
            .unwrap();
        assert_eq!(s.processed_count(), original.processed_count() + 1);
    }
    let mut s = original.clone();
    assert_eq!(
        s.reduce_bilateral_fault_injected_unverified(tx(50), 101, &bilateral(1, 100 * ONE), 0,),
        Err(ClearingError::InjectedFailure { before_leg: 0 })
    );
    assert_eq!(s, original);
}

#[test]
fn repeated_and_competing_partials_cannot_reuse_position_or_event_identity() {
    let mut s = opened();
    s.reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(2, 101 * ONE))
        .unwrap();
    let after = s.clone();
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 101 * ONE)),
        Err(ClearingError::Replay(tx(50)))
    );
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(51), 101, &bilateral(2, 101 * ONE)),
        Err(ClearingError::NotStrictPartial)
    );
    assert_eq!(s, after);
    // A new distinct match may reduce 0? No: qty zero never admitted.
}

#[test]
fn wrong_height_zero_price_and_unratified_order_priority_fail_closed() {
    let mut s = opened();
    let before = s.clone();
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 100, &bilateral(1, 101 * ONE)),
        Err(ClearingError::StaleMark)
    );
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 0)),
        Err(ClearingError::InvalidMarkPolicy)
    );
    s.process_spot_block_unverified(101).unwrap();
    let after_spot = s.clone();
    assert_eq!(
        s.reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 101 * ONE)),
        Err(ClearingError::StaleMark)
    );
    assert_eq!(s, after_spot);
    assert_eq!(before.positions(), after_spot.positions());
}

#[test]
fn tlcc_v2_synthetic_event_can_checkpoint_and_reorg_without_touching_e2b() {
    let original = opened();
    let mut journal = ClearingChainState::new_unverified(
        TradeLayerMainnet::Litecoin,
        anchor(),
        original.clone(),
        3,
    )
    .unwrap();
    let first = block(
        2,
        vec![UnverifiedClearingEvent::SyntheticBilateralReduction {
            txid: tx(50),
            fill: bilateral(1, 101 * ONE),
        }],
    );
    journal.apply_unverified_block(&first).unwrap();
    assert_eq!(
        journal.tip(),
        Tip {
            height: 101,
            id: BlockId([2; 32])
        }
    );
    assert_eq!(journal.state().positions()[&a("alice")], 2);
    assert_eq!(
        journal.state().insurance_reserve_atoms(),
        2 * u128::from(ONE)
    );
    let checkpoint = journal.checkpoint_bytes().unwrap();
    assert_eq!(&checkpoint[..5], b"TLCC\x02");
    let restored = restore_chain(&checkpoint);
    assert_eq!(restored, journal);
    let mut alternate = restored.clone();
    alternate
        .rollback_tip(Tip {
            height: 101,
            id: BlockId([2; 32]),
        })
        .unwrap();
    assert_eq!(alternate.state(), &original);
    alternate
        .apply_unverified_block(&block(
            9,
            vec![UnverifiedClearingEvent::SyntheticBilateralReduction {
                txid: tx(50),
                fill: bilateral(1, 99 * ONE),
            }],
        ))
        .unwrap();
    assert_eq!(alternate.state().positions()[&a("alice")], 2);
    assert_eq!(
        alternate.state().margin_atoms(&a("alice")),
        4 * u128::from(ONE)
    );
    assert_eq!(
        alternate.state().insurance_reserve_atoms(),
        2 * u128::from(ONE)
    );
    assert_eq!(alternate.state().ledger().verify_conservation(), Ok(()));
}

#[test]
fn late_invalid_competing_claim_rolls_back_entire_tlcc_block() {
    let initial = opened();
    let mut journal =
        ClearingChainState::new_unverified(TradeLayerMainnet::Litecoin, anchor(), initial, 3)
            .unwrap();
    let before = journal.clone();
    let two = block(
        2,
        vec![
            UnverifiedClearingEvent::SyntheticBilateralReduction {
                txid: tx(50),
                fill: bilateral(2, 101 * ONE),
            },
            UnverifiedClearingEvent::SyntheticBilateralReduction {
                txid: tx(51),
                fill: bilateral(1, 101 * ONE),
            },
        ],
    );
    assert!(matches!(
        journal.apply_unverified_block(&two),
        Err(ClearingChainError::Clearing(
            ClearingError::NotStrictPartial
        ))
    ));
    assert_eq!(journal, before);
    assert_eq!(journal.state().ledger().verify_conservation(), Ok(()));
}

#[test]
fn no_authenticated_insurance_or_e2b_payout_is_implicitly_enabled() {
    let mut s = opened();
    let start = s.clone();
    // The counterparties are existing margin accounts. If the loss is
    // larger than the real payer escrow, this partial rule CANNOT borrow
    // the existing 200M insurer. E2B remains separate.
    assert!(s
        .reduce_bilateral_positions_unverified(tx(50), 101, &bilateral(1, 106 * ONE),)
        .is_err());
    assert_eq!(s, start);
    assert!(s.spot().snapshot_bytes().is_err());
}
