//! Stage 20 audit-only spot block replay, irreversible-file checkpoint and
//! exact restoration of reserve balances and replay markers.
//! All block IDs and senders here are synthetic caller-provided assertions.

use std::collections::BTreeSet;
use tradelayer_core::chain::{BlockHeader, BlockId, ChainId, Tip};
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::spot::{
    CancelSelector, SpotError, SpotExchange, SpotIntent, SpotPair, UnverifiedSpotCancellation,
    UnverifiedSpotPlacement,
};
use tradelayer_core::spot_chain::{
    SpotChainError, SpotChainState, UnverifiedSpotBlock, UnverifiedSpotOperation,
};

fn prop(i: u32) -> PropertyId {
    PropertyId::try_from(i).unwrap()
}
fn owner(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn id(n: u8) -> TxId {
    TxId::from_bytes([n; 32])
}
fn network() -> ChainId {
    ChainId([0x26; 32])
}
fn anchor() -> Tip {
    Tip {
        height: 99,
        id: BlockId([0x44; 32]),
    }
}
fn policy() -> BTreeSet<PropertyId> {
    BTreeSet::from([prop(7), prop(8)])
}
fn bootstrap_ledger() -> Ledger {
    let mut ledger = Ledger::new();
    ledger
        .apply(
            id(1),
            Operation::CreateProperty {
                id: prop(7),
                issuer: owner("seller"),
                kind: PropertyKind::Fixed,
                initial_atoms: 1000,
            },
        )
        .unwrap();
    ledger
        .apply(
            id(2),
            Operation::CreateProperty {
                id: prop(8),
                issuer: owner("buyer"),
                kind: PropertyKind::Fixed,
                initial_atoms: 1000,
            },
        )
        .unwrap();
    ledger
}
fn bootstrap() -> SpotChainState {
    SpotChainState::new_unverified(
        network(),
        anchor(),
        SpotExchange::from_ledger_unverified(bootstrap_ledger(), policy()).unwrap(),
        3,
    )
    .unwrap()
}
fn place(id_num: u8, seller: bool, height: u64, amount: u64, want: u64) -> UnverifiedSpotOperation {
    UnverifiedSpotOperation::PlaceType5(UnverifiedSpotPlacement {
        txid: id(id_num),
        actor: if seller {
            owner("seller")
        } else {
            owner("buyer")
        },
        height,
        intent: SpotIntent {
            offered: prop(if seller { 7 } else { 8 }),
            desired: prop(if seller { 8 } else { 7 }),
            offered_atoms: amount,
            expected_atoms: want,
            post: false,
            stop: false,
        },
    })
}
fn cancel(id_num: u8, height: u64, actor: &str) -> UnverifiedSpotOperation {
    UnverifiedSpotOperation::CancelType6(UnverifiedSpotCancellation {
        txid: id(id_num),
        actor: owner(actor),
        height,
        offered: prop(7),
        desired: prop(8),
        is_contract: false,
        selector: CancelSelector::All,
    })
}
fn block(previous: Tip, hash: u8, ops: Vec<UnverifiedSpotOperation>) -> UnverifiedSpotBlock {
    UnverifiedSpotBlock {
        chain: network(),
        header: BlockHeader {
            height: previous.height + 1,
            id: BlockId([hash; 32]),
            parent: previous.id,
        },
        operations: ops,
    }
}
fn swap_root_crc(bytes: &mut [u8]) {
    let body_len = bytes.len() - 4;
    let mut crc = !0u32;
    for byte in &bytes[..body_len] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    bytes[body_len..].copy_from_slice(&(!crc).to_be_bytes());
}

#[test]
fn full_spot_book_and_history_survive_checkpoint_and_two_rollbacks() {
    let mut state = bootstrap();
    let before = state.clone();
    let b100 = block(state.tip(), 100, vec![place(10, true, 100, 100, 200)]);
    state.apply_unverified_block(&b100).unwrap();
    assert_eq!(state.tip().height, 100);
    assert_eq!(state.state().open_count(), 1);
    assert_eq!(
        state
            .state()
            .ledger()
            .available_atoms(prop(7), &owner("seller")),
        900
    );
    let after100 = state.clone();
    let b101 = block(state.tip(), 101, vec![place(11, false, 101, 110, 50)]);
    state.apply_unverified_block(&b101).unwrap();
    assert_eq!(state.state().fills().len(), 1);
    assert_eq!(state.state().fills()[0].base_atoms, 50);
    assert_eq!(state.state().fills()[0].quote_atoms, 100);
    assert_eq!(
        state
            .state()
            .ledger()
            .available_atoms(prop(8), &owner("seller")),
        100
    );
    assert_eq!(
        state
            .state()
            .open_orders(SpotPair::new(prop(7), prop(8)).unwrap())
            .len(),
        1
    );
    let after101 = state.clone();
    let bytes = state.checkpoint_bytes().unwrap();
    let recovered =
        SpotChainState::from_checkpoint_bytes(&bytes, network(), anchor(), 3, &policy()).unwrap();
    assert_eq!(recovered, state);
    assert_eq!(recovered.checkpoint_bytes().unwrap(), bytes);

    let mut restored = recovered;
    let b102 = block(restored.tip(), 102, vec![cancel(12, 102, "seller")]);
    restored.apply_unverified_block(&b102).unwrap();
    assert_eq!(restored.state().open_count(), 0);
    assert_eq!(
        restored
            .state()
            .ledger()
            .available_atoms(prop(7), &owner("seller")),
        950
    );
    assert_eq!(
        restored
            .state()
            .ledger()
            .available_atoms(prop(7), &owner("buyer")),
        50
    );
    restored.state().ledger().verify_conservation().unwrap();
    restored.state().verify_reserves().unwrap();

    restored.rollback_tip(restored.tip()).unwrap();
    assert_eq!(restored, after101);
    restored.rollback_tip(restored.tip()).unwrap();
    assert_eq!(restored, after100);
    restored.rollback_tip(restored.tip()).unwrap();
    assert_eq!(restored, before);
    assert!(matches!(
        restored.rollback_tip(restored.tip()),
        Err(SpotChainError::EmptyRollbackHistory)
    ));
}

#[test]
fn reorg_changes_fills_and_reopens_rolled_back_transaction_ids() {
    let mut state = bootstrap();
    state
        .apply_unverified_block(&block(
            state.tip(),
            100,
            vec![place(10, true, 100, 100, 200)],
        ))
        .unwrap();
    let saved = state.clone();
    state
        .apply_unverified_block(&block(
            state.tip(),
            101,
            vec![place(11, false, 101, 110, 50)],
        ))
        .unwrap();
    assert_eq!(state.state().fills().len(), 1);
    let old_tip = state.tip();
    state.rollback_tip(old_tip).unwrap();
    assert_eq!(state, saved);
    state
        .apply_unverified_block(&block(state.tip(), 210, vec![]))
        .unwrap();
    assert!(state.state().fills().is_empty());
    assert_eq!(state.tip().id, BlockId([210; 32]));
    state
        .apply_unverified_block(&block(
            state.tip(),
            211,
            vec![place(11, false, 102, 220, 100)],
        ))
        .unwrap();
    assert_eq!(state.state().fills().len(), 1);
    assert_eq!(state.state().fills()[0].base_atoms, 100);
    assert_eq!(state.state().fills()[0].quote_atoms, 200);
    assert_eq!(
        state
            .state()
            .ledger()
            .available_atoms(prop(8), &owner("buyer")),
        800
    );
    let round_trip = SpotChainState::from_checkpoint_bytes(
        &state.checkpoint_bytes().unwrap(),
        network(),
        anchor(),
        3,
        &policy(),
    )
    .unwrap();
    assert_eq!(state, round_trip);
}

#[test]
fn failed_middle_operation_has_no_partial_reservations_or_history() {
    let mut state = bootstrap();
    let before = state.clone();
    let b100 = block(
        state.tip(),
        100,
        vec![
            place(10, true, 100, 800, 800),
            place(11, true, 100, 300, 300),
        ],
    );
    assert!(matches!(
        state.apply_unverified_block(&b100),
        Err(SpotChainError::Spot(_))
    ));
    assert_eq!(state, before);
    let poisoned = block(
        state.tip(),
        100,
        vec![
            place(10, true, 100, 100, 200),
            UnverifiedSpotOperation::Type3UtxoTrade,
        ],
    );
    assert!(matches!(
        state.apply_unverified_block(&poisoned),
        Err(SpotChainError::Spot(
            SpotError::AuthenticatedUtxoProofRequired
        ))
    ));
    assert_eq!(state, before);
    state
        .apply_unverified_block(&block(
            state.tip(),
            100,
            vec![place(10, true, 100, 100, 200)],
        ))
        .unwrap();
    let saved = state.clone();
    let malicious = block(state.tip(), 101, vec![cancel(12, 101, "buyer")]);
    assert!(state.apply_unverified_block(&malicious).is_err());
    assert_eq!(state, saved);
    // Cancel-before-placement at a block boundary is explicitly unsupported.
    let forbidden = block(
        state.tip(),
        101,
        vec![place(13, false, 101, 220, 100), cancel(14, 101, "buyer")],
    );
    assert!(state.apply_unverified_block(&forbidden).is_err());
    assert_eq!(state, saved);
}

#[test]
fn wrong_header_or_claimed_heights_and_replays_fail_closed() {
    let mut state = bootstrap();
    let before = state.clone();
    let mut b = block(state.tip(), 100, vec![place(10, true, 100, 100, 200)]);
    b.chain = ChainId([0x98; 32]);
    assert!(matches!(
        state.apply_unverified_block(&b),
        Err(SpotChainError::WrongChain)
    ));
    b.chain = network();
    b.header.parent = BlockId([0x56; 32]);
    assert!(matches!(
        state.apply_unverified_block(&b),
        Err(SpotChainError::ParentMismatch)
    ));
    b.header.parent = anchor().id;
    b.header.height = 103;
    assert!(matches!(
        state.apply_unverified_block(&b),
        Err(SpotChainError::NonSequentialHeight)
    ));
    b.header.height = 100;
    b.operations = vec![place(10, true, 101, 100, 200)];
    assert!(matches!(
        state.apply_unverified_block(&b),
        Err(SpotChainError::OperationHeightMismatch)
    ));
    b.operations = vec![place(10, true, 100, 100, 200)];
    b.header.id = anchor().id;
    assert!(matches!(
        state.apply_unverified_block(&b),
        Err(SpotChainError::DuplicateBlockId)
    ));
    assert_eq!(state, before);
    b.header.id = BlockId([100; 32]);
    state.apply_unverified_block(&b).unwrap();
    let saved = state.clone();
    let replay = block(state.tip(), 101, vec![place(10, true, 101, 100, 200)]);
    assert!(matches!(
        state.apply_unverified_block(&replay),
        Err(SpotChainError::Spot(SpotError::Replay(_)))
    ));
    assert_eq!(state, saved);
}

#[test]
fn corrupted_checkpoint_wrong_policy_chain_anchor_and_recomputed_crc_are_rejected() {
    let mut state = bootstrap();
    state
        .apply_unverified_block(&block(
            state.tip(),
            100,
            vec![place(10, true, 100, 100, 200)],
        ))
        .unwrap();
    let bytes = state.checkpoint_bytes().unwrap();
    let parse = |b: &[u8], c: ChainId, a: Tip, h: usize, p: &BTreeSet<PropertyId>| {
        SpotChainState::from_checkpoint_bytes(b, c, a, h, p).is_err()
    };
    let mut corrupt = bytes.clone();
    corrupt[45] ^= 0x22;
    assert!(parse(&corrupt, network(), anchor(), 3, &policy()));
    assert!(parse(&bytes, ChainId([0x98; 32]), anchor(), 3, &policy()));
    assert!(parse(
        &bytes,
        network(),
        Tip {
            height: 98,
            id: anchor().id
        },
        3,
        &policy()
    ));
    assert!(parse(&bytes, network(), anchor(), 2, &policy()));
    assert!(parse(
        &bytes,
        network(),
        anchor(),
        3,
        &BTreeSet::from([prop(7)])
    ));
    assert!(parse(
        &bytes[..bytes.len() - 1],
        network(),
        anchor(),
        3,
        &policy()
    ));
    let mut appended = bytes.clone();
    appended.push(0);
    assert!(parse(&appended, network(), anchor(), 3, &policy()));
    // Fabricate a sequentially inconsistent tip yet give it a valid outer CRC.
    let mut forged = bytes.clone();
    let tip_height_offset = 5 + 32 + 40;
    forged[tip_height_offset + 7] ^= 3;
    swap_root_crc(&mut forged);
    assert!(parse(&forged, network(), anchor(), 3, &policy()));
    assert_eq!(state.checkpoint_bytes().unwrap(), bytes);
}

#[test]
fn checkpoint_conflict_preserves_in_memory_state_and_existing_file() {
    let mut state = bootstrap();
    let before = state.clone();
    let path = std::env::temp_dir().join(format!(
        "tradelayer-spot-stage20-conflict-{}-{}",
        std::process::id(),
        before.tip().height
    ));
    std::fs::write(&path, b"existing").unwrap();
    assert!(state
        .apply_unverified_block_and_checkpoint(
            &block(state.tip(), 100, vec![place(10, true, 100, 100, 200)]),
            &path
        )
        .is_err());
    assert_eq!(state, before);
    assert_eq!(std::fs::read(&path).unwrap(), b"existing");
    std::fs::remove_file(path).unwrap();

    let path = std::env::temp_dir().join(format!(
        "tradelayer-spot-stage20-success-{}-{}",
        std::process::id(),
        before.tip().height
    ));
    state
        .apply_unverified_block_and_checkpoint(
            &block(state.tip(), 100, vec![place(10, true, 100, 100, 200)]),
            &path,
        )
        .unwrap();
    let recovered =
        SpotChainState::load_checkpoint(&path, network(), anchor(), 3, &policy()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(state, recovered);
}

#[test]
fn invalid_foreign_spot_escrow_is_rejected_before_replay() {
    let mut ledger = bootstrap_ledger();
    ledger
        .apply(
            id(55),
            Operation::Transfer {
                id: prop(7),
                from: owner("seller"),
                to: owner("~spot:fake"),
                amount_atoms: 7,
            },
        )
        .unwrap();
    assert!(matches!(
        SpotExchange::from_ledger_unverified(ledger, policy()),
        Err(SpotError::EscrowCollision)
    ));
}

#[test]
fn history_limit_evicts_old_reversals_but_preserves_tip_snapshots() {
    let mut state = bootstrap();
    for height in 100..106u64 {
        state
            .apply_unverified_block(&block(state.tip(), height as u8, vec![]))
            .unwrap();
    }
    assert_eq!(state.rollback_depth(), 3);
    let bytes = state.checkpoint_bytes().unwrap();
    let mut restored =
        SpotChainState::from_checkpoint_bytes(&bytes, network(), anchor(), 3, &policy()).unwrap();
    for _ in 0..3 {
        let tip = restored.tip();
        restored.rollback_tip(tip).unwrap();
    }
    assert_eq!(restored.tip().height, 102);
    assert!(matches!(
        restored.rollback_tip(restored.tip()),
        Err(SpotChainError::EmptyRollbackHistory)
    ));
}

#[test]
fn queued_preblock_snapshot_roundtrip_retains_replay_ids() {
    let mut spot = SpotExchange::from_ledger_unverified(bootstrap_ledger(), policy()).unwrap();
    let UnverifiedSpotOperation::PlaceType5(pending) = place(15, true, 100, 100, 200) else {
        panic!("placement fixture");
    };
    spot.enqueue_type5_unverified(pending.clone()).unwrap();
    assert_eq!(spot.pending_count(), 1);
    assert_eq!(spot.processed_through(), None);
    let raw = spot.snapshot_bytes().unwrap();
    let mut restored = SpotExchange::from_snapshot_bytes(&raw, &policy()).unwrap();
    assert_eq!(restored, spot);
    assert_eq!(restored.snapshot_bytes().unwrap(), raw);
    assert!(matches!(
        restored.enqueue_type5_unverified(pending),
        Err(SpotError::Replay(_))
    ));
    restored.process_through_unverified(100).unwrap();
    assert_eq!(restored.pending_count(), 0);
    assert_eq!(restored.open_count(), 1);
    restored.verify_reserves().unwrap();
}

#[test]
fn forged_resting_reserve_with_valid_snapshot_crc_is_rejected() {
    let mut chain = bootstrap();
    chain
        .apply_unverified_block(&block(
            chain.tip(),
            100,
            vec![place(10, true, 100, 100, 200)],
        ))
        .unwrap();
    let original = chain.state().snapshot_bytes().unwrap();
    let mut forged = original.clone();
    // The full order txid appears in the open-book section, followed by
    // owner (one-byte size + bytes), height, offered/desired and amounts.
    let needle = id(10).as_bytes();
    let start = forged
        .windows(32)
        .position(|window| window == needle)
        .expect("order stored with full txid");
    let owner_bytes = usize::from(forged[start + 32]);
    let reserve_offset = start + 32 + 1 + owner_bytes + 8 + 4 + 4 + 8 + 8;
    assert_eq!(forged[reserve_offset + 7], 100);
    forged[reserve_offset + 7] = 99;
    swap_root_crc(&mut forged);
    assert!(SpotExchange::from_snapshot_bytes(&forged, &policy()).is_err());
    assert_eq!(chain.state().snapshot_bytes().unwrap(), original);
}

#[test]
fn recovered_block_checkpoint_rejects_old_txid_and_duplicate_ancestor_hash() {
    let mut chain = bootstrap();
    chain
        .apply_unverified_block(&block(
            chain.tip(),
            100,
            vec![place(10, true, 100, 100, 200)],
        ))
        .unwrap();
    let original = chain.checkpoint_bytes().unwrap();
    let mut recovered =
        SpotChainState::from_checkpoint_bytes(&original, network(), anchor(), 3, &policy())
            .unwrap();
    let saved = recovered.clone();
    let replay = block(recovered.tip(), 101, vec![place(10, true, 101, 50, 100)]);
    assert!(matches!(
        recovered.apply_unverified_block(&replay),
        Err(SpotChainError::Spot(SpotError::Replay(_)))
    ));
    let duplicate_ancestor = block(recovered.tip(), 0x44, vec![]);
    assert!(matches!(
        recovered.apply_unverified_block(&duplicate_ancestor),
        Err(SpotChainError::DuplicateBlockId)
    ));
    assert_eq!(recovered, saved);
}
