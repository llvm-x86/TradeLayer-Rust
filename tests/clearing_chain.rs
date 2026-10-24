//! TLCC v1 adversarial synthetic block, fork, durability and replay checks.
//! There is NO signed-chain adapter, on-chain consensus or payout authority.
use std::collections::BTreeSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tradelayer_core::chain::{BlockHeader, BlockId, Tip};
use tradelayer_core::clearing_chain::{
    ClearingChainError, ClearingChainState, UnverifiedClearingBlock, UnverifiedClearingEvent,
    MAX_CLEARING_BLOCK_EVENTS,
};
use tradelayer_core::clearing_math::PnlRounding;
use tradelayer_core::clearing_sandbox::{
    ClearingSandbox, CollateralPolicy, MarkPolicy, MarkTerms, MatchedContractTrade,
};
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::spot::{SpotExchange, SpotIntent, UnverifiedSpotPlacement};

const ATOMS: u64 = 100_000_000;

fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn a(value: &str) -> AccountId {
    AccountId::try_from(value).unwrap()
}
fn tx(n: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn tip(h: u64, id: u8) -> Tip {
    Tip {
        height: h,
        id: BlockId([id; 32]),
    }
}
fn anchor() -> Tip {
    tip(100, 1)
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
        price_e8: 100 * ATOMS,
        notional_e8: ATOMS,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn terms() -> MarkTerms {
    MarkTerms {
        notional_e8: ATOMS,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn permitted() -> BTreeSet<PropertyId> {
    BTreeSet::from([p(7), p(8)])
}
fn initial_state() -> ClearingSandbox {
    let mut ledger = Ledger::new();
    ledger
        .apply_batch([
            (
                tx(1),
                Operation::CreateProperty {
                    id: p(7),
                    issuer: a("alice"),
                    kind: PropertyKind::Fixed,
                    initial_atoms: 1_000_000_000,
                },
            ),
            (
                tx(2),
                Operation::CreateProperty {
                    id: p(8),
                    issuer: a("bob"),
                    kind: PropertyKind::Fixed,
                    initial_atoms: 1_000_000_000,
                },
            ),
            (
                tx(3),
                Operation::Transfer {
                    id: p(7),
                    from: a("alice"),
                    to: a("bob"),
                    amount_atoms: 300_000_000,
                },
            ),
            (
                tx(4),
                Operation::Transfer {
                    id: p(8),
                    from: a("bob"),
                    to: a("alice"),
                    amount_atoms: 200_000_000,
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
}
fn chain(limit: usize) -> ClearingChainState {
    ClearingChainState::new_unverified(
        TradeLayerMainnet::Litecoin,
        anchor(),
        initial_state(),
        limit,
    )
    .unwrap()
}
fn spot(id: u64, actor: &str, offer: u32, want: u32, height: u64) -> UnverifiedClearingEvent {
    UnverifiedClearingEvent::SpotPlace(UnverifiedSpotPlacement {
        txid: tx(id),
        actor: a(actor),
        height,
        intent: SpotIntent {
            offered: p(offer),
            desired: p(want),
            offered_atoms: 10_000_000,
            expected_atoms: 10_000_000,
            post: false,
            stop: false,
        },
    })
}
fn event_block(
    height: u64,
    block_id: u8,
    parent: u8,
    events: Vec<UnverifiedClearingEvent>,
) -> UnverifiedClearingBlock {
    UnverifiedClearingBlock {
        network: TradeLayerMainnet::Litecoin,
        header: BlockHeader {
            height,
            id: BlockId([block_id; 32]),
            parent: BlockId([parent; 32]),
        },
        events,
    }
}
fn first_block() -> UnverifiedClearingBlock {
    event_block(
        101,
        2,
        1,
        vec![
            UnverifiedClearingEvent::MarginDeposit {
                txid: tx(10),
                owner: a("alice"),
                atoms: 100_000_000,
            },
            UnverifiedClearingEvent::MarginDeposit {
                txid: tx(11),
                owner: a("bob"),
                atoms: 100_000_000,
            },
            UnverifiedClearingEvent::MatchedTrades {
                txid: tx(12),
                trades: vec![MatchedContractTrade {
                    long: a("alice"),
                    short: a("bob"),
                    contracts: 1,
                }],
            },
            spot(13, "alice", 7, 8, 101),
            spot(14, "bob", 8, 7, 101),
            UnverifiedClearingEvent::Funding {
                txid: tx(15),
                notional_atoms: ATOMS,
                rate_bps_e4: 100_000,
            },
            UnverifiedClearingEvent::Mark {
                txid: tx(16),
                new_price_e8: 100 * ATOMS + 1,
            },
        ],
    )
}
fn second_block() -> UnverifiedClearingBlock {
    event_block(
        102,
        3,
        2,
        vec![
            UnverifiedClearingEvent::Ordinary {
                txid: tx(17),
                operation: Operation::Transfer {
                    id: p(8),
                    from: a("bob"),
                    to: a("alice"),
                    amount_atoms: 1_000_000,
                },
            },
            UnverifiedClearingEvent::MatchedTrades {
                txid: tx(18),
                trades: vec![MatchedContractTrade {
                    long: a("bob"),
                    short: a("alice"),
                    contracts: 1,
                }],
            },
            UnverifiedClearingEvent::MarginWithdraw {
                txid: tx(19),
                owner: a("alice"),
                atoms: 1_000_000,
            },
            UnverifiedClearingEvent::Mark {
                txid: tx(20),
                new_price_e8: 100 * ATOMS + 2,
            },
        ],
    )
}
fn checkpoint_load(
    bytes: &[u8],
    limit: usize,
) -> Result<ClearingChainState, tradelayer_core::clearing_chain::ClearingChainCheckpointError> {
    ClearingChainState::from_checkpoint_bytes(
        bytes,
        TradeLayerMainnet::Litecoin,
        anchor(),
        19,
        p(7),
        &permitted(),
        Some(policy()),
        Some(terms()),
        limit,
    )
}
fn repair_crc(bytes: &mut [u8]) {
    let mut crc = !0u32;
    let end = bytes.len() - 4;
    for &byte in &bytes[..end] {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    bytes[end..].copy_from_slice(&(!crc).to_be_bytes());
}

#[test]
fn mixed_spot_clearing_funding_mark_and_token_sends_roundtrip_as_one_state() {
    let mut c = chain(4);
    c.apply_unverified_block(&first_block()).unwrap();
    assert_eq!(c.state().spot().fills().len(), 1);
    assert_eq!(c.state().spot().open_count(), 0);
    assert_eq!(c.state().positions()[&a("alice")], 1);
    assert_eq!(c.state().positions()[&a("bob")], -1);
    assert_eq!(c.state().spot().processed_through(), Some(101));
    assert_eq!(c.state().mark_policy().unwrap().height, 101);
    assert_eq!(c.state().ledger().verify_conservation(), Ok(()));
    assert!(c.state().spot().snapshot_bytes().is_err());
    c.apply_unverified_block(&second_block()).unwrap();
    assert!(c.state().positions().is_empty());
    assert_eq!(c.state().mark_policy().unwrap().height, 102);
    assert_eq!(c.rollback_depth(), 2);
    assert_eq!(c.state().ledger().verify_conservation(), Ok(()));
    let snapshot = c.checkpoint_bytes().unwrap();
    assert_eq!(&snapshot[..5], b"TLCC\x01");
    let restored = checkpoint_load(&snapshot, 4).unwrap();
    assert_eq!(restored, c);
    assert_eq!(restored.checkpoint_bytes().unwrap(), snapshot);
    assert_eq!(restored.state().verify(), Ok(()));
    assert!(restored.state().spot().snapshot_bytes().is_err());
}

#[test]
fn rollback_restores_every_account_position_order_and_replay_marker() {
    let mut state = chain(3);
    let original = state.clone();
    state.apply_unverified_block(&first_block()).unwrap();
    let after_first = state.clone();
    state.apply_unverified_block(&second_block()).unwrap();
    state.rollback_tip(tip(102, 3)).unwrap();
    assert_eq!(state, after_first);
    state.rollback_tip(tip(101, 2)).unwrap();
    assert_eq!(state, original);
    assert_eq!(state.rollback_depth(), 0);
    state.apply_unverified_block(&first_block()).unwrap();
    assert_eq!(state, after_first);
}

#[test]
fn checkpoint_recovery_allows_fork_replacement_but_not_stale_ids() {
    let mut c = chain(3);
    c.apply_unverified_block(&first_block()).unwrap();
    c.apply_unverified_block(&second_block()).unwrap();
    let bytes = c.checkpoint_bytes().unwrap();
    let mut recovered = checkpoint_load(&bytes, 3).unwrap();
    let old = recovered.clone();
    assert!(matches!(
        recovered.rollback_tip(tip(102, 99)),
        Err(ClearingChainError::WrongRollbackTip)
    ));
    assert_eq!(recovered, old);
    recovered.rollback_tip(tip(102, 3)).unwrap();
    let fork = event_block(
        102,
        44,
        2,
        vec![
            UnverifiedClearingEvent::Ordinary {
                txid: tx(99),
                operation: Operation::Transfer {
                    id: p(7),
                    from: a("alice"),
                    to: a("bob"),
                    amount_atoms: 1234,
                },
            },
            UnverifiedClearingEvent::Mark {
                txid: tx(100),
                new_price_e8: 100 * ATOMS + 3,
            },
        ],
    );
    recovered.apply_unverified_block(&fork).unwrap();
    assert_eq!(recovered.tip(), tip(102, 44));
    assert_eq!(recovered.state().ledger().verify_conservation(), Ok(()));
    assert!(recovered.checkpoint_bytes().is_ok());
    assert!(matches!(
        recovered.apply_unverified_block(&second_block()),
        Err(ClearingChainError::InvalidHeight)
    ));
}

#[test]
fn invalid_late_events_revert_spot_escrow_and_margin_atomically() {
    let mut c = chain(2);
    let original = c.clone();
    let mut events = first_block().events;
    events.push(UnverifiedClearingEvent::UnsupportedSettlement);
    let failing = event_block(101, 2, 1, events);
    assert!(matches!(
        c.apply_unverified_block(&failing),
        Err(ClearingChainError::MarkMustEndBlock)
    ));
    assert_eq!(c, original);
    let late_failure = event_block(
        101,
        2,
        1,
        vec![
            UnverifiedClearingEvent::MarginDeposit {
                txid: tx(10),
                owner: a("alice"),
                atoms: 100_000_000,
            },
            spot(13, "alice", 7, 8, 101),
            UnverifiedClearingEvent::UnsupportedSettlement,
        ],
    );
    assert!(matches!(
        c.apply_unverified_block(&late_failure),
        Err(ClearingChainError::UnsupportedEconomicSettlement)
    ));
    assert_eq!(c, original);
    c.apply_unverified_block(&first_block()).unwrap();
}

#[test]
fn wrong_chain_parent_height_duplicate_block_and_overlimit_default_deny() {
    let mut c = chain(2);
    let original = c.clone();
    let mut wrong_chain = first_block();
    wrong_chain.network = TradeLayerMainnet::Dogecoin;
    assert!(matches!(
        c.apply_unverified_block(&wrong_chain),
        Err(ClearingChainError::WrongNetwork)
    ));
    let mut wrong_parent = first_block();
    wrong_parent.header.parent = BlockId([8; 32]);
    assert!(matches!(
        c.apply_unverified_block(&wrong_parent),
        Err(ClearingChainError::ParentMismatch)
    ));
    let mut wrong_height = first_block();
    wrong_height.header.height = 105;
    assert!(matches!(
        c.apply_unverified_block(&wrong_height),
        Err(ClearingChainError::InvalidHeight)
    ));
    let huge = event_block(
        101,
        2,
        1,
        vec![UnverifiedClearingEvent::UnsupportedSettlement; MAX_CLEARING_BLOCK_EVENTS + 1],
    );
    assert!(matches!(
        c.apply_unverified_block(&huge),
        Err(ClearingChainError::TooManyEvents)
    ));
    assert_eq!(c, original);
    c.apply_unverified_block(&first_block()).unwrap();
    let duplicate = event_block(102, 2, 2, vec![]);
    assert!(matches!(
        c.apply_unverified_block(&duplicate),
        Err(ClearingChainError::DuplicateBlockId)
    ));
}

#[test]
fn insufficient_spot_or_margin_balance_prevents_partial_block_commit() {
    let mut c = chain(2);
    let original = c.clone();
    let bad = event_block(
        101,
        2,
        1,
        vec![
            UnverifiedClearingEvent::MarginDeposit {
                txid: tx(10),
                owner: a("alice"),
                atoms: 50_000_000,
            },
            spot(13, "alice", 7, 8, 101),
            UnverifiedClearingEvent::MarginDeposit {
                txid: tx(11),
                owner: a("bob"),
                atoms: u128::MAX,
            },
        ],
    );
    assert!(matches!(
        c.apply_unverified_block(&bad),
        Err(ClearingChainError::Clearing(_))
    ));
    assert_eq!(c, original);
}

#[test]
fn durable_file_collision_and_missing_directory_leave_all_state_unchanged() {
    let mut c = chain(3);
    let dir = std::env::temp_dir().join(format!(
        "tradelayer-stage25d-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("first.tlcc");
    fs::write(&path, b"do not overwrite").unwrap();
    let initial = c.clone();
    assert!(matches!(
        c.apply_unverified_block_and_checkpoint(&first_block(), &path),
        Err(ClearingChainError::Checkpoint(_))
    ));
    assert_eq!(c, initial);
    assert_eq!(fs::read(&path).unwrap(), b"do not overwrite");
    let missing_dir = dir.join("missing").join("fail.tlcc");
    assert!(c
        .apply_unverified_block_and_checkpoint(&first_block(), &missing_dir)
        .is_err());
    assert_eq!(c, initial);
    let success = dir.join("success.tlcc");
    c.apply_unverified_block_and_checkpoint(&first_block(), &success)
        .unwrap();
    let from_disk = ClearingChainState::load_checkpoint(
        &success,
        TradeLayerMainnet::Litecoin,
        anchor(),
        19,
        p(7),
        &permitted(),
        Some(policy()),
        Some(terms()),
        3,
    )
    .unwrap();
    assert_eq!(from_disk, c);
    assert_eq!(fs::read(&success).unwrap(), c.checkpoint_bytes().unwrap());
    assert!(c.write_new_checkpoint(&success).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn old_checkpoints_are_not_a_combined_history_and_crc_forgery_is_rejected() {
    let mut c = chain(3);
    c.apply_unverified_block(&first_block()).unwrap();
    let baseline = c.checkpoint_bytes().unwrap();
    assert!(c.state().spot().snapshot_bytes().is_err());
    assert!(checkpoint_load(&c.state().snapshot_bytes().unwrap(), 3).is_err());
    let mut changed = baseline.clone();
    changed[4] = 2;
    repair_crc(&mut changed);
    assert!(checkpoint_load(&changed, 3).is_err());
    let mut corrupt = baseline.clone();
    corrupt[5] ^= 1;
    repair_crc(&mut corrupt);
    assert!(checkpoint_load(&corrupt, 3).is_err());
    let mut trail = baseline.clone();
    trail.push(0);
    assert!(checkpoint_load(&trail, 3).is_err());
    for size in 0..baseline.len().min(192) {
        assert!(checkpoint_load(&baseline[..size], 3).is_err());
    }
    assert!(checkpoint_load(&baseline[..baseline.len() - 1], 3).is_err());
    assert!(ClearingChainState::from_checkpoint_bytes(
        &baseline,
        TradeLayerMainnet::Litecoin,
        anchor(),
        20,
        p(7),
        &permitted(),
        Some(policy()),
        Some(terms()),
        3
    )
    .is_err());
    assert!(ClearingChainState::from_checkpoint_bytes(
        &baseline,
        TradeLayerMainnet::Litecoin,
        anchor(),
        19,
        p(7),
        &permitted(),
        None,
        Some(terms()),
        3
    )
    .is_err());
    assert!(checkpoint_load(&baseline, 2).is_err());
}

#[test]
fn reorg_history_is_bounded_and_deep_rollback_denies_without_mutation() {
    let mut c = chain(2);
    c.apply_unverified_block(&first_block()).unwrap();
    c.apply_unverified_block(&second_block()).unwrap();
    c.apply_unverified_block(&event_block(103, 4, 3, vec![]))
        .unwrap();
    assert_eq!(c.rollback_depth(), 2);
    let encoded = c.checkpoint_bytes().unwrap();
    let mut restored = checkpoint_load(&encoded, 2).unwrap();
    assert_eq!(restored, c);
    restored.rollback_tip(tip(103, 4)).unwrap();
    restored.rollback_tip(tip(102, 3)).unwrap();
    assert_eq!(restored.tip(), tip(101, 2));
    let before = restored.clone();
    assert!(matches!(
        restored.rollback_tip(tip(101, 2)),
        Err(ClearingChainError::EmptyRollbackHistory)
    ));
    assert_eq!(restored, before);
}

#[test]
fn a_successful_empty_block_finalizes_spot_and_reversible_history() {
    let mut c = chain(2);
    c.apply_unverified_block(&event_block(101, 2, 1, vec![]))
        .unwrap();
    assert_eq!(c.state().spot().processed_through(), Some(101));
    assert_eq!(c.state().mark_policy().unwrap().height, 100);
    let bytes = c.checkpoint_bytes().unwrap();
    assert_eq!(checkpoint_load(&bytes, 2).unwrap(), c);
    c.rollback_tip(tip(101, 2)).unwrap();
    assert_eq!(c.tip(), anchor());
    assert_eq!(c.state().spot().processed_through(), None);
}
