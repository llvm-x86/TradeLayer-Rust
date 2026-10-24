//! Stage 25A: all funds stay in ONE owned Ledger, even when margin is locked.
//! No chain actors, signed trades, settlement proofs or external dependencies.
use tradelayer_core::clearing_sandbox::{
    ClearingError, ClearingSandbox, MatchedContractTrade, MAX_CLEARING_OWNERS,
    MAX_CLEARING_TRADES_PER_INPUT,
};
use tradelayer_core::funding_audit::FundingAuditError;
use tradelayer_core::ledger::{
    AccountId, Ledger, LedgerError, Operation, PropertyId, PropertyKind, TxId,
};
use tradelayer_core::network_profile::TradeLayerMainnet;

fn p() -> PropertyId {
    PropertyId::try_from(7).unwrap()
}
fn a(value: &str) -> AccountId {
    AccountId::try_from(value).unwrap()
}
fn tx(value: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&value.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn prefunded() -> Ledger {
    let mut ledger = Ledger::new();
    ledger
        .apply(
            tx(1),
            Operation::CreateProperty {
                id: p(),
                issuer: a("alice"),
                kind: PropertyKind::Fixed,
                initial_atoms: 1_000_000_000,
            },
        )
        .unwrap();
    ledger
        .apply_batch([
            (
                tx(2),
                Operation::Transfer {
                    id: p(),
                    from: a("alice"),
                    to: a("bob"),
                    amount_atoms: 350_000_000,
                },
            ),
            (
                tx(3),
                Operation::Transfer {
                    id: p(),
                    from: a("alice"),
                    to: a("carol"),
                    amount_atoms: 250_000_000,
                },
            ),
        ])
        .unwrap();
    ledger
}
fn sandbox() -> ClearingSandbox {
    ClearingSandbox::from_ledger_unverified(TradeLayerMainnet::Litecoin, 19, p(), prefunded())
        .unwrap()
}
fn opening() -> ClearingSandbox {
    let mut book = sandbox();
    book.deposit_margin_unverified(tx(4), a("alice"), 50_000_000)
        .unwrap();
    book.deposit_margin_unverified(tx(5), a("bob"), 40_000_000)
        .unwrap();
    book.deposit_margin_unverified(tx(6), a("carol"), 25_000_000)
        .unwrap();
    book.apply_matched_trades_unverified(
        tx(7),
        &[
            MatchedContractTrade {
                long: a("alice"),
                short: a("bob"),
                contracts: 2,
            },
            MatchedContractTrade {
                long: a("alice"),
                short: a("carol"),
                contracts: 1,
            },
        ],
    )
    .unwrap();
    book
}

#[test]
fn opening_funding_and_escrow_moves_use_exactly_one_token_ledger() {
    let mut book = opening();
    assert_eq!(book.processed_count(), 4);
    assert_eq!(book.positions()[&a("alice")], 3);
    assert_eq!(book.positions()[&a("bob")], -2);
    assert_eq!(book.positions()[&a("carol")], -1);
    assert_eq!(book.margin_atoms(&a("alice")), 50_000_000);
    assert_eq!(book.margin_atoms(&a("bob")), 40_000_000);
    assert_eq!(book.margin_atoms(&a("carol")), 25_000_000);
    assert_eq!(book.ledger().available_atoms(p(), &a("alice")), 350_000_000);
    assert_eq!(book.ledger().available_atoms(p(), &a("bob")), 310_000_000);
    assert_eq!(book.ledger().available_atoms(p(), &a("carol")), 225_000_000);
    let escrows = [
        book.margin_escrow_for_audit(&a("alice")).unwrap().clone(),
        book.margin_escrow_for_audit(&a("bob")).unwrap().clone(),
        book.margin_escrow_for_audit(&a("carol")).unwrap().clone(),
    ];
    assert_ne!(escrows[0], escrows[1]);
    assert_ne!(escrows[0], escrows[2]);
    assert!(escrows.iter().all(|account| account.as_str().starts_with("~clrm:")));

    let receipt = book.settle_funding_unverified(tx(8), 100_000_000, 100_000).unwrap();
    assert_eq!(receipt.nominal_atoms, 300_000);
    assert_eq!(receipt.ledger_transfer_legs, 2);
    assert_eq!(receipt.active_positions, 3);
    assert_eq!(book.margin_atoms(&a("alice")), 49_700_000);
    assert_eq!(book.margin_atoms(&a("bob")), 40_200_000);
    assert_eq!(book.margin_atoms(&a("carol")), 25_100_000);
    assert_eq!(book.ledger().property(p()).unwrap().total_supply_atoms, 1_000_000_000);
    assert_eq!(book.ledger().verify_conservation(), Ok(()));
    assert_eq!(book.verify(), Ok(()));
}

#[test]
fn negative_funding_charges_shorts_not_longs() {
    let mut book = opening();
    let receipt = book.settle_funding_unverified(tx(8), 100_000_000, -100_000).unwrap();
    assert_eq!(receipt.nominal_atoms, 300_000);
    assert_eq!(book.margin_atoms(&a("alice")), 50_300_000);
    assert_eq!(book.margin_atoms(&a("bob")), 39_800_000);
    assert_eq!(book.margin_atoms(&a("carol")), 24_900_000);
    assert_eq!(book.verify(), Ok(()));
}

#[test]
fn fault_before_every_funding_leg_or_commit_reverts_state_and_markers() {
    let original = opening();
    for leg in 0..=2 {
        let mut book = original.clone();
        let result =
            book.settle_funding_fault_injected_unverified(tx(8), 100_000_000, 100_000, leg);
        assert_eq!(result, Err(ClearingError::InjectedFailure { before_leg: leg }));
        assert_eq!(book, original, "injected failure at leg {leg}");
        assert_eq!(book.verify(), Ok(()));
        // The same external ID must remain usable after an aborted attempt.
        let receipt = book.settle_funding_unverified(tx(8), 100_000_000, 100_000).unwrap();
        assert_eq!(receipt.ledger_transfer_legs, 2);
        assert_eq!(book.processed_count(), original.processed_count() + 1);
    }
}

#[test]
fn duplicate_external_replay_and_baseline_ledger_ids_are_rejected() {
    let mut book = opening();
    let before = book.clone();
    assert_eq!(
        book.settle_funding_unverified(tx(7), 100_000_000, 100_000),
        Err(ClearingError::Replay(tx(7)))
    );
    assert_eq!(
        book.settle_funding_unverified(tx(2), 100_000_000, 100_000),
        Err(ClearingError::Replay(tx(2)))
    );
    assert_eq!(book, before);
    book.settle_funding_unverified(tx(8), 100_000_000, 100_000).unwrap();
    let after = book.clone();
    assert_eq!(
        book.settle_funding_unverified(tx(8), 100_000_000, 100_000),
        Err(ClearingError::Replay(tx(8)))
    );
    assert_eq!(book, after);
}

#[test]
fn depositing_without_funds_and_withdrawing_open_margin_are_atomic() {
    let mut book = opening();
    let before = book.clone();
    let failure = book.deposit_margin_unverified(tx(8), a("bob"), 400_000_000);
    assert!(matches!(
        failure,
        Err(ClearingError::Ledger(LedgerError::InsufficientAvailable { .. }))
    ));
    assert_eq!(book, before);
    assert_eq!(
        book.withdraw_margin_unverified(tx(8), a("alice"), 10),
        Err(ClearingError::NotFlat)
    );
    assert_eq!(book, before);
}

#[test]
fn closing_then_withdrawing_releases_actual_ledger_atoms() {
    let mut book = opening();
    book.apply_matched_trades_unverified(
        tx(8),
        &[
            MatchedContractTrade {
                long: a("bob"),
                short: a("alice"),
                contracts: 2,
            },
            MatchedContractTrade {
                long: a("carol"),
                short: a("alice"),
                contracts: 1,
            },
        ],
    )
    .unwrap();
    assert!(book.positions().is_empty());
    book.withdraw_margin_unverified(tx(9), a("alice"), 50_000_000)
        .unwrap();
    assert_eq!(book.margin_atoms(&a("alice")), 0);
    assert!(book.margin_escrow_for_audit(&a("alice")).is_none());
    assert_eq!(book.ledger().available_atoms(p(), &a("alice")), 400_000_000);
    assert_eq!(book.ledger().verify_conservation(), Ok(()));
}

#[test]
fn invalid_second_match_rolls_back_first_match_and_replay_marker() {
    let mut book = opening();
    let before = book.clone();
    let bad = book.apply_matched_trades_unverified(
        tx(8),
        &[
            MatchedContractTrade {
                long: a("bob"),
                short: a("alice"),
                contracts: 2,
            },
            MatchedContractTrade {
                long: a("mallory"),
                short: a("carol"),
                contracts: 1,
            },
        ],
    );
    assert_eq!(bad, Err(ClearingError::UnknownMarginOwner));
    assert_eq!(book, before);
    assert_eq!(book.verify(), Ok(()));
    book.settle_funding_unverified(tx(8), 100_000_000, 100_000).unwrap();
}

#[test]
fn insufficient_locked_collateral_denies_full_transfer_batch_atomically() {
    let mut book = sandbox();
    book.deposit_margin_unverified(tx(4), a("alice"), 1_000_000).unwrap();
    book.deposit_margin_unverified(tx(5), a("bob"), 1).unwrap();
    book.apply_matched_trades_unverified(
        tx(6),
        &[MatchedContractTrade {
            long: a("alice"),
            short: a("bob"),
            contracts: 1,
        }],
    )
    .unwrap();
    let before = book.clone();
    assert_eq!(
        book.settle_funding_unverified(tx(7), 100_000_000, -100_000),
        Err(ClearingError::Funding(
            FundingAuditError::InsufficientAvailable("bob".to_string())
        ))
    );
    assert_eq!(book, before);
}

#[test]
fn foreign_reserved_balances_and_channels_cannot_join_a_margin_sandbox() {
    let mut foreign = prefunded();
    foreign
        .apply(
            tx(4),
            Operation::Transfer {
                id: p(),
                from: a("alice"),
                to: a("~spot:forged"),
                amount_atoms: 100,
            },
        )
        .unwrap();
    assert_eq!(
        ClearingSandbox::from_ledger_unverified(TradeLayerMainnet::Litecoin, 19, p(), foreign),
        Err(ClearingError::EscrowCollision)
    );
    let mut book = sandbox();
    assert_eq!(
        book.deposit_margin_unverified(tx(4), a("~spot:forged"), 1),
        Err(ClearingError::ReservedOwner)
    );
}

#[test]
fn namespacing_changes_between_mainnets_and_contracts() {
    let network_one = sandbox();
    let network_two = ClearingSandbox::from_ledger_unverified(
        TradeLayerMainnet::Dogecoin, 19, p(), prefunded(),
    ).unwrap();
    let another_contract = ClearingSandbox::from_ledger_unverified(
        TradeLayerMainnet::Litecoin, 20, p(), prefunded(),
    ).unwrap();
    let mut books = [network_one, network_two, another_contract];
    for book in &mut books {
        book.deposit_margin_unverified(tx(4), a("alice"), 1).unwrap();
    }
    let names: Vec<_> = books
        .iter()
        .map(|book| book.margin_escrow_for_audit(&a("alice")).unwrap().clone())
        .collect();
    assert_ne!(names[0], names[1]);
    assert_ne!(names[0], names[2]);
    assert_ne!(names[1], names[2]);
}

#[test]
fn oversized_and_self_matched_trades_reject_before_changing_any_state() {
    let mut book = opening();
    let snapshot = book.clone();
    assert_eq!(
        book.apply_matched_trades_unverified(
            tx(8),
            &[MatchedContractTrade {
                long: a("alice"),
                short: a("alice"),
                contracts: 1,
            }]
        ),
        Err(ClearingError::SelfTrade)
    );
    assert_eq!(
        book.apply_matched_trades_unverified(
            tx(8),
            &[MatchedContractTrade {
                long: a("alice"),
                short: a("bob"),
                contracts: u64::MAX,
            }]
        ),
        Err(ClearingError::PositionOverflow)
    );
    assert_eq!(book, snapshot);
    assert_eq!(
        book.apply_matched_trades_unverified(tx(8), &[]),
        Err(ClearingError::EmptyTradeBatch)
    );
    let trade = MatchedContractTrade {
        long: a("alice"),
        short: a("bob"),
        contracts: 1,
    };
    let excessive = vec![trade; MAX_CLEARING_TRADES_PER_INPUT + 1];
    assert_eq!(
        book.apply_matched_trades_unverified(tx(8), &excessive),
        Err(ClearingError::TooManyTrades)
    );
    assert_eq!(book, snapshot);
}

#[test]
fn bounded_owner_count_rejects_another_registration_without_side_effects() {
    assert_eq!(MAX_CLEARING_OWNERS, 128);
    let mut book = sandbox();
    let before = book.clone();
    assert_eq!(
        book.deposit_margin_unverified(tx(4), a("alice"), 0),
        Err(ClearingError::ZeroAmount)
    );
    assert_eq!(book, before);
}

#[test]
fn deterministic_seeded_funding_cycles_preserve_supply_and_net_open_interest() {
    let mut book = opening();
    let mut seed = 0x7e57_1234_5678_1234u64;
    let total_supply = book.ledger().property(p()).unwrap().total_supply_atoms;
    for i in 0..96u64 {
        // A fixed-seed xorshift generator avoids rand/proc-macro dependencies.
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let rate = if seed & 1 == 0 { 100_000 } else { -100_000 };
        let notional = 1_000_000 + (seed % 1_000_000);
        let event = tx(50 + i);
        let before = book.clone();
        let receipt = book
            .settle_funding_unverified(event, notional, rate)
            .unwrap();
        assert!(receipt.ledger_transfer_legs <= 2);
        assert_eq!(book.ledger().property(p()).unwrap().total_supply_atoms, total_supply);
        assert_eq!(book.ledger().verify_conservation(), Ok(()));
        assert_eq!(book.verify(), Ok(()));
        assert_eq!(book.positions(), before.positions());
        assert_eq!(book.processed_count(), before.processed_count() + 1);
    }
}
