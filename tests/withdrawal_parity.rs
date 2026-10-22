//! Stage 11: source-derived queue divergence and finalized-block replay tests.
//! These are offline models. No signed chain data or live JS service is used.
use tradelayer_core::chain::ChainId;
use tradelayer_core::execution::{ExecutionError, ProtocolOperation, ProtocolState, UnverifiedTx};
use tradelayer_core::ledger::{
    AccountId, ChannelSide, LedgerError, PropertyId, PropertyKind, TxId,
};
use tradelayer_core::registry::RegistryAction;
use tradelayer_core::withdrawal_parity::{
    audit_legacy_type21_amount, LegacyAmountError, LegacyWithdrawalShadow, ShadowError,
    ShadowEvent, ShadowResolution, ShadowVerdict, MAX_EXACT_LEGACY_JS_ATOMS,
};

fn a(name: &str) -> AccountId {
    AccountId::try_from(name).unwrap()
}
fn p() -> PropertyId {
    PropertyId::try_from(7).unwrap()
}
fn txid(value: u8) -> TxId {
    TxId::from_bytes([value; 32])
}
fn chain() -> ChainId {
    ChainId([77; 32])
}
fn tx(value: u8, actor: &str, height: u64, operation: ProtocolOperation) -> UnverifiedTx {
    UnverifiedTx {
        chain: chain(),
        txid: txid(value),
        claimed_actor: a(actor),
        claimed_height: height,
        operation,
    }
}
fn bootstrap(atoms: u128) -> ProtocolState {
    let mut state = ProtocolState::new_unverified(chain(), a("admin"));
    state
        .apply_unverified(tx(
            1,
            "admin",
            100,
            ProtocolOperation::Registry(RegistryAction::Activate {
                types: vec![0, 1, 2, 4, 21, 22],
                code_hash: [9; 32],
            }),
        ))
        .unwrap();
    state
        .apply_unverified(tx(
            2,
            "alice",
            101,
            ProtocolOperation::Issue {
                id: p(),
                kind: PropertyKind::Fixed,
                initial_atoms: atoms,
            },
        ))
        .unwrap();
    state
}
fn commit(atoms: u128) -> ProtocolOperation {
    ProtocolOperation::ChannelCommit {
        id: p(),
        channel: a("chan"),
        side: ChannelSide::A,
        amount_atoms: atoms,
    }
}
fn request(atoms: u128) -> ProtocolOperation {
    ProtocolOperation::ChannelWithdraw {
        id: p(),
        channel: a("chan"),
        side: ChannelSide::A,
        amount_atoms: atoms,
    }
}
fn transfer_out(atoms: u128) -> ProtocolOperation {
    ProtocolOperation::ChannelTransfer {
        id: p(),
        from_side: ChannelSide::A,
        to_channel: a("other"),
        to_side: ChannelSide::B,
        amount_atoms: atoms,
    }
}
fn advance_to(state: &mut ProtocolState, start: u64, end: u64) {
    for height in start..=end {
        state.finalize_block_unverified(height).unwrap();
    }
}

#[test]
fn overlapping_legacy_requests_are_accepted_but_second_request_drops_at_maturity() {
    let mut js = LegacyWithdrawalShadow::new(100);
    js.apply_block(102, &[ShadowEvent::Commit(100)]).unwrap();
    js.apply_block(
        103,
        &[
            ShadowEvent::Request {
                txid: txid(4),
                amount_atoms: 60,
            },
            ShadowEvent::Request {
                txid: txid(5),
                amount_atoms: 60,
            },
        ],
    )
    .unwrap();
    assert_eq!(js.pending_count(), 2);
    assert_eq!(js.channel_atoms(), 100);
    for height in 104..110 {
        assert!(js.apply_block(height, &[]).unwrap().is_empty());
    }
    assert_eq!(
        js.apply_block(110, &[]).unwrap(),
        vec![
            ShadowResolution {
                txid: txid(4),
                amount_atoms: 60,
                verdict: ShadowVerdict::Paid
            },
            ShadowResolution {
                txid: txid(5),
                amount_atoms: 60,
                verdict: ShadowVerdict::DiscardedInsufficient,
            },
        ]
    );
    assert_eq!(js.available_atoms(), 60);
    assert_eq!(js.channel_atoms(), 40);
    js.verify_conservation().unwrap();

    // Stage 10's reserving policy refuses the second request immediately.
    let mut stage10 = bootstrap(100);
    stage10
        .apply_unverified(tx(3, "alice", 102, commit(100)))
        .unwrap();
    stage10
        .apply_unverified(tx(4, "alice", 103, request(60)))
        .unwrap();
    let before = stage10.clone();
    assert!(matches!(
        stage10.apply_unverified(tx(5, "alice", 103, request(60))),
        Err(ExecutionError::Ledger(
            LedgerError::InsufficientAvailable { .. }
        ))
    ));
    assert_eq!(before, stage10);
    advance_to(&mut stage10, 103, 110);
    assert_eq!(stage10.ledger().available_atoms(p(), &a("alice")), 60);
    assert_eq!(
        stage10
            .ledger()
            .channel_atoms(p(), &a("chan"), ChannelSide::A),
        40
    );
}

#[test]
fn a_transfer_after_legacy_request_can_defund_its_eventual_payout() {
    let mut js = LegacyWithdrawalShadow::new(100);
    js.apply_block(102, &[ShadowEvent::Commit(100)]).unwrap();
    js.apply_block(
        103,
        &[ShadowEvent::Request {
            txid: txid(4),
            amount_atoms: 80,
        }],
    )
    .unwrap();
    js.apply_block(104, &[ShadowEvent::TransferOut(80)])
        .unwrap();
    for height in 105..110 {
        js.apply_block(height, &[]).unwrap();
    }
    assert_eq!(
        js.apply_block(110, &[]).unwrap(),
        vec![ShadowResolution {
            txid: txid(4),
            amount_atoms: 80,
            verdict: ShadowVerdict::DiscardedInsufficient,
        }]
    );
    assert_eq!(
        (js.available_atoms(), js.channel_atoms(), js.outside_atoms()),
        (0, 20, 80)
    );

    let mut safe = bootstrap(100);
    safe.apply_unverified(tx(3, "alice", 102, commit(100)))
        .unwrap();
    safe.apply_unverified(tx(4, "alice", 103, request(80)))
        .unwrap();
    let before = safe.clone();
    assert!(matches!(
        safe.apply_unverified(tx(5, "chan", 104, transfer_out(80))),
        Err(ExecutionError::Ledger(
            LedgerError::InsufficientAvailable { .. }
        ))
    ));
    assert_eq!(safe, before);
    advance_to(&mut safe, 103, 110);
    assert_eq!(safe.ledger().available_atoms(p(), &a("alice")), 80);
}

#[test]
fn deferred_queue_maturity_uses_post_block_balance_and_request_order() {
    let mut js = LegacyWithdrawalShadow::new(100);
    js.apply_block(102, &[ShadowEvent::Commit(100)]).unwrap();
    js.apply_block(
        103,
        &[ShadowEvent::Request {
            txid: txid(4),
            amount_atoms: 80,
        }],
    )
    .unwrap();
    js.apply_block(104, &[ShadowEvent::TransferOut(80)])
        .unwrap();
    for height in 105..109 {
        js.apply_block(height, &[]).unwrap();
    }
    // Funds arrive before the 110 end-of-block queue processing.
    js.apply_block(109, &[ShadowEvent::TransferBack(80)])
        .unwrap();
    assert_eq!(
        js.apply_block(110, &[]).unwrap(),
        vec![ShadowResolution {
            txid: txid(4),
            amount_atoms: 80,
            verdict: ShadowVerdict::Paid
        }]
    );
    assert_eq!(
        (js.available_atoms(), js.channel_atoms(), js.outside_atoms()),
        (80, 20, 0)
    );
    js.verify_conservation().unwrap();
}

#[test]
fn source_derived_queue_rejects_bad_replays_and_does_not_mutate_on_failure() {
    let mut js = LegacyWithdrawalShadow::new(100);
    js.apply_block(100, &[ShadowEvent::Commit(70)]).unwrap();
    let before = js.clone();
    assert_eq!(
        js.apply_block(
            101,
            &[ShadowEvent::Request {
                txid: txid(1),
                amount_atoms: 80
            }]
        ),
        Err(ShadowError::InsufficientEscrow)
    );
    assert_eq!(js, before);
    js.apply_block(
        101,
        &[ShadowEvent::Request {
            txid: txid(1),
            amount_atoms: 60,
        }],
    )
    .unwrap();
    let before = js.clone();
    assert_eq!(
        js.apply_block(
            102,
            &[ShadowEvent::Request {
                txid: txid(1),
                amount_atoms: 10
            }]
        ),
        Err(ShadowError::DuplicateRequest)
    );
    assert_eq!(js, before);
    assert_eq!(
        js.apply_block(103, &[]),
        Err(ShadowError::NonSequentialBlock)
    );
    assert_eq!(js, before);
    let before = js.clone();
    assert_eq!(
        js.apply_block(
            102,
            &[ShadowEvent::TransferOut(1), ShadowEvent::TransferBack(999)]
        ),
        Err(ShadowError::InsufficientOutside)
    );
    assert_eq!(js, before);
    js.verify_conservation().unwrap();
}

#[test]
fn finalized_height_is_versioned_and_prevents_time_travel() {
    let mut s = bootstrap(100);
    let legacy_bytes = s.snapshot_bytes().unwrap();
    assert!(legacy_bytes.starts_with(b"TLPS\x01"));
    assert_eq!(
        ProtocolState::from_snapshot_bytes(&legacy_bytes, chain(), &a("admin")).unwrap(),
        s
    );
    assert_eq!(s.finalized_height(), None);

    s.finalize_block_unverified(101).unwrap();
    assert_eq!(s.finalized_height(), Some(101));
    let current_bytes = s.snapshot_bytes().unwrap();
    assert!(current_bytes.starts_with(b"TLPS\x02"));
    let restored =
        ProtocolState::from_snapshot_bytes(&current_bytes, chain(), &a("admin")).unwrap();
    assert_eq!(restored, s);
    assert_eq!(restored.snapshot_bytes().unwrap(), current_bytes);

    let before = s.clone();
    assert_eq!(
        s.apply_unverified(tx(3, "alice", 101, commit(10))),
        Err(ExecutionError::FinalizedHeight)
    );
    assert_eq!(
        s.finalize_block_unverified(101),
        Err(ExecutionError::FinalizationOutOfSequence)
    );
    assert_eq!(
        s.finalize_block_unverified(103),
        Err(ExecutionError::FinalizationOutOfSequence)
    );
    assert_eq!(s, before);
    s.apply_unverified(tx(3, "alice", 102, commit(10))).unwrap();
    s.finalize_block_unverified(102).unwrap();
    assert_eq!(s.finalized_height(), Some(102));
    assert_eq!(
        s.apply_unverified(tx(4, "alice", 102, request(5))),
        Err(ExecutionError::FinalizedHeight)
    );

    let mut damaged = s.snapshot_bytes().unwrap();
    damaged[10] ^= 1;
    assert!(ProtocolState::from_snapshot_bytes(&damaged, chain(), &a("admin")).is_err());
}

#[test]
fn oracle_rejects_unbounded_request_maturity_atomically() {
    let mut js = LegacyWithdrawalShadow::new(30);
    let near_overflow = u64::MAX - 4;
    let before = js.clone();
    assert_eq!(
        js.apply_block(
            near_overflow,
            &[
                ShadowEvent::Commit(30),
                ShadowEvent::Request {
                    txid: txid(9),
                    amount_atoms: 10
                },
            ]
        ),
        Err(ShadowError::ArithmeticOverflow)
    );
    assert_eq!(js, before);
}

#[test]
fn legacy_type21_wire_base_mismatch_agrees_with_independent_node_vectors() {
    let fixtures = include_str!("../fixtures/withdrawal_codec_vectors.tsv");
    let mut count = 0;
    for line in fixtures
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
    {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 3);
        let atoms: u128 = fields[0].parse().unwrap();
        let expected_decoded: u128 = fields[2].parse().unwrap();
        let finding = audit_legacy_type21_amount(atoms).unwrap();
        assert_eq!(finding.decimal_encoded, fields[1]);
        assert_eq!(finding.base36_decoded_atoms, expected_decoded);
        count += 1;
    }
    assert!(count >= 6);
    assert_eq!(
        audit_legacy_type21_amount(MAX_EXACT_LEGACY_JS_ATOMS + 1),
        Err(LegacyAmountError::UnsafeJsIntegerRange)
    );
}
