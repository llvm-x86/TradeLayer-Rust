//! Non-network ledger tests. All account identities and transaction IDs are
//! explicitly supplied test data, never proof of blockchain authorization.

use tradelayer_core::ledger::{
    AccountId, Ledger, LedgerError, Operation, PropertyId, PropertyKind, TxId,
};
use tradelayer_core::wire_v1::{encode_send, parse_send, Send};

fn p(number: u32) -> PropertyId {
    PropertyId::try_from(number).unwrap()
}

fn a(address: &str) -> AccountId {
    AccountId::try_from(address).unwrap()
}

fn tx(number: u64) -> TxId {
    let mut bytes = [0_u8; 32];
    bytes[24..].copy_from_slice(&number.to_be_bytes());
    TxId::from_bytes(bytes)
}

fn create(id: u32, kind: PropertyKind, supply: u128) -> Operation {
    Operation::CreateProperty {
        id: p(id),
        issuer: a("alice"),
        kind,
        initial_atoms: supply,
    }
}

fn send(id: u32, from: &str, to: &str, amount: u128) -> Operation {
    Operation::Transfer {
        id: p(id),
        from: a(from),
        to: a(to),
        amount_atoms: amount,
    }
}

#[test]
fn transfers_preserve_fixed_supply() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(7, PropertyKind::Fixed, 1000))
        .unwrap();
    ledger.apply(tx(2), send(7, "alice", "bob", 475)).unwrap();
    ledger.apply(tx(3), send(7, "bob", "carol", 75)).unwrap();
    assert_eq!(ledger.available_atoms(p(7), &a("alice")), 525);
    assert_eq!(ledger.available_atoms(p(7), &a("bob")), 400);
    assert_eq!(ledger.available_atoms(p(7), &a("carol")), 75);
    assert_eq!(ledger.property(p(7)).unwrap().total_supply_atoms, 1000);
    assert_eq!(ledger.verify_conservation(), Ok(()));
}

#[test]
fn managed_mint_and_self_burn_change_supply_exactly() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(11, PropertyKind::Managed, 100))
        .unwrap();
    ledger
        .apply(
            tx(2),
            Operation::Mint {
                id: p(11),
                issuer: a("alice"),
                to: a("bob"),
                amount_atoms: 75,
            },
        )
        .unwrap();
    ledger
        .apply(
            tx(3),
            Operation::Burn {
                id: p(11),
                issuer: a("alice"),
                amount_atoms: 23,
            },
        )
        .unwrap();
    assert_eq!(ledger.property(p(11)).unwrap().total_supply_atoms, 152);
    assert_eq!(ledger.available_atoms(p(11), &a("alice")), 77);
    assert_eq!(ledger.available_atoms(p(11), &a("bob")), 75);
    assert_eq!(ledger.verify_conservation(), Ok(()));
}

#[test]
fn fixed_supply_cannot_be_minted_or_burned() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(7, PropertyKind::Fixed, 100))
        .unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.apply(
            tx(2),
            Operation::Mint {
                id: p(7),
                issuer: a("alice"),
                to: a("bob"),
                amount_atoms: 1,
            }
        ),
        Err(LedgerError::FixedSupply(p(7)))
    );
    assert_eq!(
        ledger.apply(
            tx(3),
            Operation::Burn {
                id: p(7),
                issuer: a("alice"),
                amount_atoms: 1,
            }
        ),
        Err(LedgerError::FixedSupply(p(7)))
    );
    assert_eq!(ledger, before);
}

#[test]
fn nonissuer_cannot_mint_or_burn() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(8, PropertyKind::Managed, 100))
        .unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.apply(
            tx(2),
            Operation::Mint {
                id: p(8),
                issuer: a("mallory"),
                to: a("mallory"),
                amount_atoms: 10,
            }
        ),
        Err(LedgerError::UnauthorizedIssuer(p(8)))
    );
    assert_eq!(
        ledger.apply(
            tx(3),
            Operation::Burn {
                id: p(8),
                issuer: a("mallory"),
                amount_atoms: 10,
            }
        ),
        Err(LedgerError::UnauthorizedIssuer(p(8)))
    );
    assert_eq!(ledger, before);
}

#[test]
fn cannot_burn_someone_elses_balance() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(8, PropertyKind::Managed, 100))
        .unwrap();
    ledger.apply(tx(2), send(8, "alice", "bob", 91)).unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.apply(
            tx(3),
            Operation::Burn {
                id: p(8),
                issuer: a("alice"),
                amount_atoms: 10,
            }
        ),
        Err(LedgerError::InsufficientAvailable {
            property: p(8),
            available: 9,
            requested: 10,
        })
    );
    assert_eq!(ledger, before);
}

#[test]
fn atomic_batch_rolls_back_property_creation_and_replay_markers() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(2, PropertyKind::Fixed, 50))
        .unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.apply_batch([
            (tx(2), create(3, PropertyKind::Managed, 75)),
            (tx(3), send(2, "alice", "bob", 20)),
            (tx(4), send(2, "bob", "carol", 21)),
        ]),
        Err(LedgerError::InsufficientAvailable {
            property: p(2),
            available: 20,
            requested: 21,
        })
    );
    assert_eq!(ledger, before);
    for n in [2, 3, 4] {
        assert!(!ledger.is_processed(tx(n)));
    }
    assert!(ledger.property(p(3)).is_none());
}

#[test]
fn atomic_batch_commits_in_sequence() {
    let mut ledger = Ledger::new();
    ledger
        .apply_batch([
            (tx(1), create(2, PropertyKind::Fixed, 50)),
            (tx(2), send(2, "alice", "bob", 20)),
            (tx(3), send(2, "bob", "carol", 15)),
        ])
        .unwrap();
    assert_eq!(ledger.available_atoms(p(2), &a("alice")), 30);
    assert_eq!(ledger.available_atoms(p(2), &a("bob")), 5);
    assert_eq!(ledger.available_atoms(p(2), &a("carol")), 15);
    assert_eq!(ledger.processed_count(), 3);
    assert_eq!(ledger.verify_conservation(), Ok(()));
}

#[test]
fn replay_rejection_is_global_and_atomic() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(1, PropertyKind::Fixed, 50))
        .unwrap();
    ledger.apply(tx(2), send(1, "alice", "bob", 20)).unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.apply(tx(2), send(1, "alice", "bob", 20)),
        Err(LedgerError::AlreadyProcessed(tx(2)))
    );
    assert_eq!(
        ledger.apply_batch([
            (tx(3), send(1, "alice", "carol", 10)),
            (tx(3), send(1, "carol", "bob", 5)),
        ]),
        Err(LedgerError::AlreadyProcessed(tx(3)))
    );
    assert_eq!(ledger, before);
}

#[test]
fn rejected_txid_may_be_retried_after_fixing_operation() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(1, PropertyKind::Fixed, 50))
        .unwrap();
    assert!(ledger.apply(tx(2), send(1, "alice", "bob", 51)).is_err());
    assert!(!ledger.is_processed(tx(2)));
    ledger.apply(tx(2), send(1, "alice", "bob", 50)).unwrap();
    assert!(ledger.is_processed(tx(2)));
}

#[test]
fn self_send_checks_balance_without_inflation() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(4, PropertyKind::Fixed, u128::MAX))
        .unwrap();
    ledger
        .apply(tx(2), send(4, "alice", "alice", u128::MAX))
        .unwrap();
    assert_eq!(ledger.property(p(4)).unwrap().total_supply_atoms, u128::MAX);
    assert_eq!(ledger.available_atoms(p(4), &a("alice")), u128::MAX);
    assert_eq!(ledger.verify_conservation(), Ok(()));
    assert!(ledger.is_processed(tx(2)));
}

#[test]
fn mint_overflow_preserves_every_field() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(4, PropertyKind::Managed, u128::MAX))
        .unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.apply(
            tx(2),
            Operation::Mint {
                id: p(4),
                issuer: a("alice"),
                to: a("bob"),
                amount_atoms: 1,
            }
        ),
        Err(LedgerError::ArithmeticOverflow)
    );
    assert_eq!(ledger, before);
}

#[test]
fn zero_amounts_and_unknown_properties_are_rejected() {
    let mut ledger = Ledger::new();
    assert_eq!(
        ledger.apply(tx(1), create(1, PropertyKind::Fixed, 0)),
        Err(LedgerError::ZeroAmount)
    );
    assert_eq!(
        ledger.apply(tx(2), send(9, "alice", "bob", 1)),
        Err(LedgerError::UnknownProperty(p(9)))
    );
    ledger
        .apply(tx(3), create(1, PropertyKind::Fixed, 10))
        .unwrap();
    let before = ledger.clone();
    assert_eq!(
        ledger.apply(tx(4), send(1, "alice", "bob", 0)),
        Err(LedgerError::ZeroAmount)
    );
    assert_eq!(
        ledger.apply(tx(5), create(1, PropertyKind::Fixed, 10)),
        Err(LedgerError::PropertyAlreadyExists(p(1)))
    );
    assert_eq!(ledger, before);
}

#[test]
fn independent_valid_transactions_commute_and_are_deterministic() {
    let mut left = Ledger::new();
    left.apply(tx(1), create(1, PropertyKind::Fixed, 100))
        .unwrap();
    let mut right = left.clone();
    left.apply(tx(2), send(1, "alice", "bob", 20)).unwrap();
    left.apply(tx(3), send(1, "alice", "carol", 30)).unwrap();
    right.apply(tx(3), send(1, "alice", "carol", 30)).unwrap();
    right.apply(tx(2), send(1, "alice", "bob", 20)).unwrap();
    assert_eq!(left, right);
    assert_eq!(left.verify_conservation(), Ok(()));
}

#[test]
fn forked_state_can_be_replayed_from_an_unchanged_checkpoint() {
    let mut checkpoint = Ledger::new();
    checkpoint
        .apply(tx(1), create(1, PropertyKind::Fixed, 100))
        .unwrap();
    let mut fork_a = checkpoint.clone();
    let mut fork_b = checkpoint.clone();
    fork_a.apply(tx(2), send(1, "alice", "bob", 40)).unwrap();
    fork_b.apply(tx(3), send(1, "alice", "carol", 30)).unwrap();
    assert_eq!(checkpoint.available_atoms(p(1), &a("alice")), 100);
    assert_eq!(fork_a.available_atoms(p(1), &a("bob")), 40);
    assert_eq!(fork_b.available_atoms(p(1), &a("carol")), 30);
    assert_ne!(fork_a, fork_b);
    assert_eq!(fork_a.verify_conservation(), Ok(()));
    assert_eq!(fork_b.verify_conservation(), Ok(()));
    assert!(!checkpoint.is_processed(tx(2)));
}

#[test]
fn v1_binary_send_can_be_mapped_to_an_explicit_ledger_operation() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(5, PropertyKind::Fixed, 100))
        .unwrap();
    let bytes = encode_send(Send::Direct {
        property_id: 5,
        amount_atoms: 1,
        recipient: "bob",
    })
    .unwrap();
    let decoded = parse_send(&bytes).unwrap();
    match decoded {
        Send::Direct {
            property_id,
            amount_atoms,
            recipient,
        } => {
            ledger
                .apply(
                    tx(2),
                    send(property_id, "alice", recipient, u128::from(amount_atoms)),
                )
                .unwrap();
        }
        Send::All { .. } => panic!("unexpected send-all"),
    }
    assert_eq!(ledger.available_atoms(p(5), &a("bob")), 1);
    assert_eq!(ledger.available_atoms(p(5), &a("alice")), 99);
    assert_eq!(ledger.verify_conservation(), Ok(()));
}

#[test]
fn randomized_legal_and_illegal_transfers_preserve_every_invariant() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(2, PropertyKind::Fixed, 1_000_000))
        .unwrap();
    let names = ["alice", "bob", "carol", "dave"];
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for i in 2..=2_002_u64 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let from = names[(state as usize) % names.len()];
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let to = names[(state as usize) % names.len()];
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let amount = u128::from(state % 25_000 + 1);
        let before = ledger.clone();
        let outcome = ledger.apply(tx(i), send(2, from, to, amount));
        if outcome.is_err() {
            assert_eq!(ledger, before, "failed mutation at {i}");
        }
        assert_eq!(ledger.property(p(2)).unwrap().total_supply_atoms, 1_000_000);
        assert_eq!(ledger.verify_conservation(), Ok(()), "after {i}");
    }
}

#[test]
fn duplicate_late_in_batch_rolls_back_every_collection() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(20, PropertyKind::Fixed, 100))
        .unwrap();
    let before = ledger.clone();
    let result = ledger.apply_batch([
        (tx(2), send(20, "alice", "bob", 20)),
        (tx(3), create(21, PropertyKind::Managed, 50)),
        (tx(2), send(20, "bob", "carol", 1)),
    ]);
    assert_eq!(result, Err(LedgerError::AlreadyProcessed(tx(2))));
    assert_eq!(ledger, before);
    assert!(!ledger.is_processed(tx(2)));
    assert!(!ledger.is_processed(tx(3)));
    assert!(ledger.property(p(21)).is_none());
    assert_eq!(ledger.verify_conservation(), Ok(()));
}

#[test]
fn late_overflow_rolls_back_earlier_mint_and_replay_markers() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(22, PropertyKind::Managed, u128::MAX - 1))
        .unwrap();
    let before = ledger.clone();
    let result = ledger.apply_batch([
        (
            tx(2),
            Operation::Mint {
                id: p(22),
                issuer: a("alice"),
                to: a("bob"),
                amount_atoms: 1,
            },
        ),
        (
            tx(3),
            Operation::Mint {
                id: p(22),
                issuer: a("alice"),
                to: a("bob"),
                amount_atoms: 1,
            },
        ),
    ]);
    assert_eq!(result, Err(LedgerError::ArithmeticOverflow));
    assert_eq!(ledger, before);
    assert!(!ledger.is_processed(tx(2)));
    assert!(!ledger.is_processed(tx(3)));
    assert_eq!(ledger.available_atoms(p(22), &a("bob")), 0);
    assert_eq!(ledger.verify_conservation(), Ok(()));
}

#[test]
fn iterator_panic_during_staging_preserves_public_ledger() {
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(23, PropertyKind::Fixed, 100))
        .unwrap();
    let before = ledger.clone();
    let partial =
        std::iter::once((tx(2), send(23, "alice", "bob", 20))).chain(std::iter::once_with(
            || -> (TxId, Operation) { panic!("deliberate panic after a valid staged transaction") },
        ));
    let caught =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ledger.apply_batch(partial)));
    assert!(caught.is_err());
    assert_eq!(ledger, before);
    assert!(!ledger.is_processed(tx(2)));
    assert_eq!(ledger.verify_conservation(), Ok(()));
}

#[test]
fn journal_sweep_preserves_other_property_and_rejects_wrong_receipt_atomically() {
    use tradelayer_core::ledger::ChannelSide;

    let chan = a("chan");
    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(7, PropertyKind::Fixed, 200))
        .unwrap();
    ledger.apply(tx(2), send(7, "alice", "bob", 100)).unwrap();
    ledger
        .apply(
            tx(3),
            Operation::ChannelCommit {
                id: p(7),
                owner: a("alice"),
                channel: chan.clone(),
                side: ChannelSide::A,
                amount_atoms: 100,
            },
        )
        .unwrap();
    ledger
        .apply(
            tx(4),
            Operation::ChannelCommit {
                id: p(7),
                owner: a("bob"),
                channel: chan.clone(),
                side: ChannelSide::B,
                amount_atoms: 100,
            },
        )
        .unwrap();
    ledger
        .apply(tx(5), create(8, PropertyKind::Managed, 50))
        .unwrap();
    ledger
        .apply(
            tx(6),
            Operation::Mint {
                id: p(8),
                issuer: a("alice"),
                to: a("bob"),
                amount_atoms: 20,
            },
        )
        .unwrap();
    let property8_alice = ledger.available_atoms(p(8), &a("alice"));
    let property8_bob = ledger.available_atoms(p(8), &a("bob"));

    for (txid, side, atoms, reference, height) in [
        (7, ChannelSide::A, 30, 50, 104),
        (8, ChannelSide::B, 10, 51, 105),
    ] {
        ledger
            .apply(
                tx(txid),
                Operation::ChannelNetClaim {
                    id: p(7),
                    channel: chan.clone(),
                    payer_side: side,
                    amount_atoms: atoms,
                    trade_reference: tx(reference),
                    height,
                    expiry_height: 200,
                },
            )
            .unwrap();
    }
    let preview = ledger
        .preview_king_settlement_unverified(&chan, p(7), 104, 105, 106)
        .unwrap();
    assert_eq!(preview.net_atoms, 20);
    let make_sweep = |net_atoms| Operation::ChannelKingSettle {
        id: p(7),
        channel: chan.clone(),
        start_height: 104,
        end_height: 105,
        settled_at: 106,
        expected_root: preview.root,
        expected_count: preview.count,
        payer_side: preview.payer_side,
        net_atoms,
    };

    let prior = ledger.clone();
    assert_eq!(
        ledger.apply(tx(9), make_sweep(21)),
        Err(LedgerError::SettlementCommitmentMismatch)
    );
    assert_eq!(ledger, prior);
    assert!(!ledger.is_processed(tx(9)));

    ledger.apply(tx(9), make_sweep(20)).unwrap();
    assert_eq!(ledger.pending_net_settlement_count(), 0);
    assert_eq!(ledger.swept_net_settlement_count(), 2);
    assert_eq!(ledger.king_settlement_count(), 1);
    assert_eq!(ledger.channel_atoms(p(7), &chan, ChannelSide::A), 80);
    assert_eq!(ledger.channel_atoms(p(7), &chan, ChannelSide::B), 120);
    assert_eq!(ledger.available_atoms(p(8), &a("alice")), property8_alice);
    assert_eq!(ledger.available_atoms(p(8), &a("bob")), property8_bob);
    assert_eq!(ledger.property(p(8)).unwrap().total_supply_atoms, 70);
    assert_eq!(ledger.verify_conservation(), Ok(()));
    let checkpoint = ledger.snapshot_bytes().unwrap();
    assert_eq!(Ledger::from_snapshot_bytes(&checkpoint).unwrap(), ledger);
}

#[test]
fn canonical_supply_audit_tracks_arbitrary_wallet_channel_and_pending_columns() {
    use tradelayer_core::ledger::{
        ChannelSide, SupplyAuditRow, SupplyColumnRow, SupplyColumnSource,
    };

    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(7, PropertyKind::Fixed, 1000))
        .unwrap();
    ledger.apply(tx(2), send(7, "alice", "bob", 400)).unwrap();
    ledger.apply(tx(3), send(7, "bob", "carol", 100)).unwrap();
    ledger
        .apply(
            tx(4),
            Operation::ChannelCommit {
                id: p(7),
                owner: a("bob"),
                channel: a("chan"),
                side: ChannelSide::A,
                amount_atoms: 150,
            },
        )
        .unwrap();
    ledger
        .apply(
            tx(5),
            Operation::ChannelCommit {
                id: p(7),
                owner: a("carol"),
                channel: a("chan"),
                side: ChannelSide::B,
                amount_atoms: 60,
            },
        )
        .unwrap();
    ledger
        .apply(
            tx(6),
            Operation::ChannelWithdraw {
                id: p(7),
                owner: a("bob"),
                channel: a("chan"),
                side: ChannelSide::A,
                amount_atoms: 35,
                unlock_height: 150,
            },
        )
        .unwrap();

    ledger
        .apply(tx(7), create(8, PropertyKind::Managed, 500))
        .unwrap();
    ledger
        .apply(
            tx(8),
            Operation::Mint {
                id: p(8),
                issuer: a("alice"),
                to: a("bob"),
                amount_atoms: 70,
            },
        )
        .unwrap();
    ledger.apply(tx(9), send(8, "alice", "carol", 30)).unwrap();

    let expected = vec![
        SupplyAuditRow {
            property: p(7),
            total_supply_atoms: 1000,
            available_atoms: 790,
            escrow_atoms: 175,
            pending_withdrawal_atoms: 35,
        },
        SupplyAuditRow {
            property: p(8),
            total_supply_atoms: 570,
            available_atoms: 570,
            escrow_atoms: 0,
            pending_withdrawal_atoms: 0,
        },
    ];
    assert_eq!(ledger.supply_audit_rows_unverified().unwrap(), expected);
    let columns = ledger.supply_column_rows_unverified().unwrap();
    let expected_columns = vec![
        SupplyColumnRow {
            property: p(7),
            source: SupplyColumnSource::Available(a("alice")),
            atoms: 600,
        },
        SupplyColumnRow {
            property: p(7),
            source: SupplyColumnSource::Available(a("bob")),
            atoms: 150,
        },
        SupplyColumnRow {
            property: p(7),
            source: SupplyColumnSource::Available(a("carol")),
            atoms: 40,
        },
        SupplyColumnRow {
            property: p(7),
            source: SupplyColumnSource::ChannelEscrow {
                channel: a("chan"),
                side: ChannelSide::A,
            },
            atoms: 115,
        },
        SupplyColumnRow {
            property: p(7),
            source: SupplyColumnSource::ChannelEscrow {
                channel: a("chan"),
                side: ChannelSide::B,
            },
            atoms: 60,
        },
        SupplyColumnRow {
            property: p(7),
            source: SupplyColumnSource::ReservedWithdrawal(tx(6)),
            atoms: 35,
        },
        SupplyColumnRow {
            property: p(8),
            source: SupplyColumnSource::Available(a("alice")),
            atoms: 470,
        },
        SupplyColumnRow {
            property: p(8),
            source: SupplyColumnSource::Available(a("bob")),
            atoms: 70,
        },
        SupplyColumnRow {
            property: p(8),
            source: SupplyColumnSource::Available(a("carol")),
            atoms: 30,
        },
    ];
    assert_eq!(columns, expected_columns);
    assert!(columns.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(columns.iter().all(|row| row.atoms > 0));
    let mut grouped = std::collections::BTreeMap::new();
    for row in &columns {
        let amounts = grouped.entry(row.property).or_insert([0u128; 3]);
        let group = match &row.source {
            SupplyColumnSource::Available(_) => 0,
            SupplyColumnSource::ChannelEscrow { .. } => 1,
            SupplyColumnSource::ReservedWithdrawal(_) => 2,
        };
        amounts[group] = amounts[group].checked_add(row.atoms).unwrap();
    }
    assert_eq!(grouped.get(&p(7)), Some(&[790, 175, 35]));
    assert_eq!(grouped.get(&p(8)), Some(&[570, 0, 0]));
    assert_eq!(ledger.verify_conservation(), Ok(()));
    let snapshot = ledger.snapshot_bytes().unwrap();
    let reloaded = Ledger::from_snapshot_bytes(&snapshot).unwrap();
    assert_eq!(reloaded, ledger);
    assert_eq!(reloaded.supply_audit_rows_unverified().unwrap(), expected);
    assert_eq!(
        reloaded.supply_column_rows_unverified().unwrap(),
        expected_columns
    );

    let immutable = ledger.clone();
    assert_eq!(ledger.settle_due_withdrawals_unverified(149), Ok(0));
    assert_eq!(ledger, immutable);
    assert_eq!(ledger.settle_due_withdrawals_unverified(150), Ok(1));
    let mut matured = expected;
    matured[0].available_atoms += 35;
    matured[0].pending_withdrawal_atoms = 0;
    assert_eq!(ledger.supply_audit_rows_unverified().unwrap(), matured);
    assert_eq!(ledger.pending_withdrawal_count(), 0);
    assert_eq!(ledger.verify_conservation(), Ok(()));
}

#[test]
fn newly_seen_recipient_and_zero_pruning_change_only_column_support() {
    use tradelayer_core::ledger::{SupplyColumnRow, SupplyColumnSource};

    let mut ledger = Ledger::new();
    ledger
        .apply(tx(1), create(31, PropertyKind::Fixed, 100))
        .unwrap();
    let initial = ledger.supply_column_rows_unverified().unwrap();
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].source, SupplyColumnSource::Available(a("alice")));

    ledger
        .apply(tx(2), send(31, "alice", "new-recipient", 100))
        .unwrap();
    let after = ledger.supply_column_rows_unverified().unwrap();
    assert_eq!(
        after,
        vec![SupplyColumnRow {
            property: p(31),
            source: SupplyColumnSource::Available(a("new-recipient")),
            atoms: 100,
        }]
    );
    let first = ledger.supply_audit_rows_unverified().unwrap();
    assert_eq!(first[0].available_atoms, 100);
    assert_eq!(first[0].total_supply_atoms, 100);

    // Identity sends consume replay IDs after checking funds without adding
    // another balance column.
    ledger
        .apply(tx(3), send(31, "new-recipient", "new-recipient", 100))
        .unwrap();
    assert_eq!(ledger.supply_column_rows_unverified().unwrap(), after);
    let snapshot = ledger.snapshot_bytes().unwrap();
    let restored = Ledger::from_snapshot_bytes(&snapshot).unwrap();
    assert_eq!(restored.supply_column_rows_unverified().unwrap(), after);
    assert_eq!(restored.verify_conservation(), Ok(()));
}

#[test]
fn independent_transfer_permutations_produce_identical_canonical_support() {
    use tradelayer_core::ledger::{SupplyColumnRow, SupplyColumnSource};

    fn init() -> Ledger {
        let mut l = Ledger::new();
        l.apply(tx(1), create(41, PropertyKind::Fixed, 1000)).unwrap();
        l.apply(tx(2), create(42, PropertyKind::Managed, 500)).unwrap();
        l
    }
    let sends = [
        (3u64, "zulu", 200u128),
        (4, "alpha", 150),
        (5, "mike", 100),
        (6, "bob", 50),
    ];
    let mut forward = init();
    let mut reverse = init();
    for &(t, recipient, atoms) in &sends {
        forward.apply(tx(t), send(41, "alice", recipient, atoms)).unwrap();
    }
    for &(t, recipient, atoms) in sends.iter().rev() {
        reverse.apply(tx(t), send(41, "alice", recipient, atoms)).unwrap();
    }
    assert_eq!(forward, reverse);
    assert_eq!(forward.supply_column_rows_unverified().unwrap(),
        vec![
            SupplyColumnRow { property: p(41),
                source: SupplyColumnSource::Available(a("alice")), atoms: 500 },
            SupplyColumnRow { property: p(41),
                source: SupplyColumnSource::Available(a("alpha")), atoms: 150 },
            SupplyColumnRow { property: p(41),
                source: SupplyColumnSource::Available(a("bob")), atoms: 50 },
            SupplyColumnRow { property: p(41),
                source: SupplyColumnSource::Available(a("mike")), atoms: 100 },
            SupplyColumnRow { property: p(41),
                source: SupplyColumnSource::Available(a("zulu")), atoms: 200 },
            SupplyColumnRow { property: p(42),
                source: SupplyColumnSource::Available(a("alice")), atoms: 500 },
        ]);
    // Deleting a middle BTreeMap entry and introducing a new lower-sorting
    // account changes support at those keys only.
    let mut pruned = forward.clone();
    pruned.apply(tx(7), send(41, "bob", "aardvark", 50)).unwrap();
    let records = pruned.supply_column_rows_unverified().unwrap();
    assert!(records.iter().all(|r| r.atoms != 0));
    assert!(!records.iter().any(|r| r.source == SupplyColumnSource::Available(a("bob"))));
    assert!(records.iter().any(|r| r.source == SupplyColumnSource::Available(a("aardvark"))
        && r.property == p(41) && r.atoms == 50));
    assert_eq!(pruned.available_atoms(p(42), &a("alice")), 500);
    assert_eq!(pruned.verify_conservation(), Ok(()));
    let roundtrip = Ledger::from_snapshot_bytes(&pruned.snapshot_bytes().unwrap()).unwrap();
    assert_eq!(roundtrip.supply_column_rows_unverified().unwrap(), records);
}
