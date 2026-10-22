//! Stage 10 synthetic channel accounting tests; signatures and blocks are fake.
use std::collections::BTreeMap;
use tradelayer_core::chain::{BlockHeader, BlockId, CandidateBlock, ChainId, Tip};
use tradelayer_core::execution::{ExecutionError, ProtocolOperation, ProtocolState, UnverifiedTx};
use tradelayer_core::ledger::{
    AccountId, ChannelSide, Ledger, LedgerError, Operation, PropertyId, PropertyKind, TxId,
};
use tradelayer_core::registry::RegistryAction;
use tradelayer_core::unified_chain::{
    AuthenticatedProtocolOperation, TrustedProtocolAdapter, UnifiedChainState,
};

fn a(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn p() -> PropertyId {
    PropertyId::try_from(7).unwrap()
}
fn chain() -> ChainId {
    ChainId([77; 32])
}
fn txid(n: u8) -> TxId {
    TxId::from_bytes([n; 32])
}
fn tx(n: u8, actor: &str, height: u64, operation: ProtocolOperation) -> UnverifiedTx {
    UnverifiedTx {
        chain: chain(),
        txid: txid(n),
        claimed_actor: a(actor),
        claimed_height: height,
        operation,
    }
}
fn activation() -> ProtocolOperation {
    ProtocolOperation::Registry(RegistryAction::Activate {
        types: vec![0, 1, 2, 4, 21, 22],
        code_hash: [6; 32],
    })
}
fn issue(amount_atoms: u128) -> ProtocolOperation {
    ProtocolOperation::Issue {
        id: p(),
        kind: PropertyKind::Fixed,
        initial_atoms: amount_atoms,
    }
}
fn commit(channel: &str, side: ChannelSide, amount_atoms: u128) -> ProtocolOperation {
    ProtocolOperation::ChannelCommit {
        id: p(),
        channel: a(channel),
        side,
        amount_atoms,
    }
}
fn withdraw(channel: &str, side: ChannelSide, amount_atoms: u128) -> ProtocolOperation {
    ProtocolOperation::ChannelWithdraw {
        id: p(),
        channel: a(channel),
        side,
        amount_atoms,
    }
}
fn move_channel(
    to_channel: &str,
    from_side: ChannelSide,
    to_side: ChannelSide,
    amount_atoms: u128,
) -> ProtocolOperation {
    ProtocolOperation::ChannelTransfer {
        id: p(),
        from_side,
        to_channel: a(to_channel),
        to_side,
        amount_atoms,
    }
}
fn bootstrap(initial: u128) -> ProtocolState {
    let mut s = ProtocolState::new_unverified(chain(), a("admin"));
    s.apply_unverified(tx(1, "admin", 100, activation()))
        .unwrap();
    s.apply_unverified(tx(2, "alice", 101, issue(initial)))
        .unwrap();
    s
}

#[test]
fn available_escrow_pending_supply_is_conserved_through_maturity() {
    let mut s = bootstrap(100);
    s.apply_unverified(tx(3, "alice", 102, commit("chan1", ChannelSide::A, 60)))
        .unwrap();
    assert_eq!(s.ledger().available_atoms(p(), &a("alice")), 40);
    assert_eq!(s.ledger().channel_atoms(p(), &a("chan1"), ChannelSide::A), 60);
    s.apply_unverified(tx(4, "alice", 103, withdraw("chan1", ChannelSide::A, 20)))
        .unwrap();
    assert_eq!(s.ledger().channel_atoms(p(), &a("chan1"), ChannelSide::A), 40);
    assert_eq!(s.ledger().pending_withdrawal_atoms(p(), &a("alice")), 20);
    s.ledger().verify_conservation().unwrap();

    // Reserved withdrawal cannot be transferred before maturity.
    let original = s.clone();
    assert!(matches!(
        s.apply_unverified(tx(5, "chan1", 104, move_channel("chan2", ChannelSide::A, ChannelSide::A, 41))),
        Err(ExecutionError::Ledger(LedgerError::InsufficientAvailable { .. }))
    ));
    assert_eq!(s, original);
    assert_eq!(s.finalize_block_unverified(109).unwrap(), 0);
    assert_eq!(s.ledger().pending_withdrawal_count(), 1);
    assert_eq!(s.finalize_block_unverified(110).unwrap(), 1);
    assert_eq!(s.finalize_block_unverified(110).unwrap(), 0);
    assert_eq!(s.ledger().available_atoms(p(), &a("alice")), 60);
    assert_eq!(s.ledger().channel_atoms(p(), &a("chan1"), ChannelSide::A), 40);
    assert_eq!(s.ledger().pending_withdrawal_count(), 0);
    s.ledger().verify_conservation().unwrap();
}

#[test]
fn channel_transfer_preserves_column_owner_and_requires_source_channel_actor() {
    let mut s = bootstrap(200);
    s.apply_unverified(tx(3, "alice", 102, commit("chan1", ChannelSide::A, 100)))
        .unwrap();
    s.apply_unverified(tx(4, "chan1", 103, move_channel("chan2", ChannelSide::B, ChannelSide::A, 15)))
        .unwrap_err(); // Empty B side cannot be moved.
    s.apply_unverified(tx(4, "chan1", 103, move_channel("chan2", ChannelSide::A, ChannelSide::B, 35)))
        .unwrap();
    assert_eq!(s.ledger().channel_atoms(p(), &a("chan1"), ChannelSide::A), 65);
    assert_eq!(s.ledger().channel_atoms(p(), &a("chan2"), ChannelSide::B), 35);
    assert_eq!(s.ledger().channel_owner(&a("chan2"), ChannelSide::B), Some(&a("alice")));
    let before = s.clone();
    assert_eq!(
        s.apply_unverified(tx(5, "mallory", 104, withdraw("chan2", ChannelSide::B, 10))),
        Err(ExecutionError::Ledger(LedgerError::ChannelSideMismatch(a("chan2"))))
    );
    assert_eq!(s, before);
    assert!(matches!(
        s.apply_unverified(tx(5, "mallory", 104, move_channel("chan3", ChannelSide::B, ChannelSide::A, 5))),
        Err(ExecutionError::Ledger(LedgerError::UnknownChannel(_)))
    ));
    assert_eq!(s, before);
    s.apply_unverified(tx(5, "alice", 104, withdraw("chan2", ChannelSide::B, 10)))
        .unwrap();
    assert_eq!(s.ledger().pending_withdrawal_atoms(p(), &a("alice")), 10);
    s.ledger().verify_conservation().unwrap();
}

#[test]
fn takeover_and_duplicate_column_assignment_are_atomic() {
    let mut s = bootstrap(100);
    s.apply_unverified(tx(3, "alice", 102, commit("chan1", ChannelSide::A, 20)))
        .unwrap();
    let before = s.clone();
    assert_eq!(
        s.apply_unverified(tx(4, "alice", 103, commit("chan1", ChannelSide::B, 5))),
        Err(ExecutionError::Ledger(LedgerError::ChannelSideOccupied(a("chan1"))))
    );
    assert_eq!(s, before);
    s.apply_unverified(tx(4, "alice", 103, ProtocolOperation::Send {
        id: p(), recipient: a("bob"), amount_atoms: 20,
    })).unwrap();
    assert_eq!(
        s.apply_unverified(tx(5, "bob", 104, commit("chan1", ChannelSide::A, 5))),
        Err(ExecutionError::Ledger(LedgerError::ChannelSideOccupied(a("chan1"))))
    );
    s.apply_unverified(tx(5, "bob", 104, commit("chan1", ChannelSide::B, 15)))
        .unwrap();
    assert_eq!(s.ledger().channel_owner(&a("chan1"), ChannelSide::B), Some(&a("bob")));
    let before = s.clone();
    assert!(matches!(
        s.apply_unverified(tx(6, "chan1", 105, move_channel("chan1", ChannelSide::A, ChannelSide::B, 1))),
        Err(ExecutionError::Ledger(LedgerError::SameChannel))
    ));
    assert_eq!(s, before);
    s.ledger().verify_conservation().unwrap();
}

#[test]
fn invalid_mixed_batch_does_not_reserve_funds_or_consume_txids() {
    let mut s = bootstrap(90);
    let before = s.clone();
    let bad = [
        tx(3, "alice", 102, commit("chan1", ChannelSide::A, 60)),
        tx(4, "alice", 102, commit("chan2", ChannelSide::A, 60)),
    ];
    assert!(matches!(
        s.apply_batch_unverified(bad),
        Err(ExecutionError::Ledger(LedgerError::InsufficientAvailable { .. }))
    ));
    assert_eq!(s, before);
    assert_eq!(s.ledger().channel_atoms(p(), &a("chan1"), ChannelSide::A), 0);
    s.apply_unverified(tx(3, "alice", 102, commit("chan1", ChannelSide::A, 60)))
        .unwrap();
}

#[test]
fn ledger_version_1_remains_readable_and_version_2_roundtrips() {
    let mut ledger = Ledger::new();
    let original = ledger.snapshot_bytes().unwrap();
    assert!(original.starts_with(b"TLL\x01"));
    assert_eq!(Ledger::from_snapshot_bytes(&original).unwrap(), ledger);
    ledger.apply(txid(1), Operation::CreateProperty {
        id: p(), issuer: a("alice"), kind: PropertyKind::Fixed, initial_atoms: 100,
    }).unwrap();
    ledger.apply(txid(2), Operation::ChannelCommit {
        id: p(), owner: a("alice"), channel: a("chan1"), side: ChannelSide::A, amount_atoms: 50,
    }).unwrap();
    ledger.apply(txid(3), Operation::ChannelWithdraw {
        id: p(), owner: a("alice"), channel: a("chan1"),
        side: ChannelSide::A, amount_atoms: 20, unlock_height: 108,
    }).unwrap();
    let bytes = ledger.snapshot_bytes().unwrap();
    assert!(bytes.starts_with(b"TLL\x02"));
    let restored = Ledger::from_snapshot_bytes(&bytes).unwrap();
    assert_eq!(restored, ledger);
    assert_eq!(restored.snapshot_bytes().unwrap(), bytes);
    let mut tampered = bytes.clone();
    tampered[40] ^= 1;
    assert!(Ledger::from_snapshot_bytes(&tampered).is_err());
    assert!(Ledger::from_snapshot_bytes(&bytes[..bytes.len() - 1]).is_err());
    let recovered_protocol = {
        let mut s = bootstrap(100);
        s.apply_unverified(tx(3, "alice", 102, commit("chan1", ChannelSide::A, 60))).unwrap();
        s.apply_unverified(tx(4, "alice", 103, withdraw("chan1", ChannelSide::A, 20))).unwrap();
        let snap = s.snapshot_bytes().unwrap();
        let recovered = ProtocolState::from_snapshot_bytes(&snap, chain(), &a("admin")).unwrap();
        assert_eq!(recovered.snapshot_bytes().unwrap(), snap);
        recovered
    };
    assert_eq!(recovered_protocol.ledger().pending_withdrawal_count(), 1);
}

#[derive(Default)]
struct SyntheticAdapter {
    decoded: BTreeMap<u8, AuthenticatedProtocolOperation>,
}
impl SyntheticAdapter {
    fn add(&mut self, n: u8, actor: &str, operation: ProtocolOperation) {
        self.decoded.insert(n, AuthenticatedProtocolOperation {
            txid: txid(n), actor: a(actor), operation,
        });
    }
}
impl TrustedProtocolAdapter for SyntheticAdapter {
    fn verify_block(&self, _: &CandidateBlock) -> Result<(), String> {
        Ok(())
    }
    fn authenticate_and_decode(
        &self, _: ChainId, _: &BlockHeader, raw: &[u8],
    ) -> Result<Option<AuthenticatedProtocolOperation>, String> {
        let id = *raw.first().ok_or_else(|| "missing fake id".to_string())?;
        self.decoded.get(&id).cloned().map(Some)
            .ok_or_else(|| "unknown fake operation".to_string())
    }
}
fn anchor() -> Tip {
    Tip { height: 100, id: BlockId([100; 32]) }
}
fn block(tip: Tip, block_num: u8, operations: &[u8]) -> CandidateBlock {
    CandidateBlock {
        chain: chain(),
        header: BlockHeader { height: tip.height + 1, id: BlockId([block_num; 32]), parent: tip.id },
        raw_transactions: operations.iter().map(|&op| vec![op]).collect(),
    }
}

#[test]
fn unified_checkpoints_and_tip_only_blocks_restore_pending_withdrawals_on_reorg() {
    let mut state = UnifiedChainState::new_unverified(chain(), anchor(), a("admin"), 16).unwrap();
    let mut adapter = SyntheticAdapter::default();
    adapter.add(1, "admin", activation());
    adapter.add(2, "alice", issue(100));
    adapter.add(3, "alice", commit("chan1", ChannelSide::A, 60));
    adapter.add(4, "alice", withdraw("chan1", ChannelSide::A, 20));
    for height in 101..=104 {
        state.apply_block(&block(state.tip(), height as u8, &[(height - 100) as u8]), &adapter)
            .unwrap();
    }
    assert_eq!(state.state().ledger().pending_withdrawal_count(), 1);
    let bytes = state.checkpoint_bytes().unwrap();
    let checkpoint = UnifiedChainState::from_checkpoint_bytes(
        &bytes, chain(), anchor(), &a("admin"), 16,
    ).unwrap();
    assert_eq!(checkpoint, state);
    for height in 105..=110 {
        state.apply_block(&block(state.tip(), height as u8, &[]), &adapter)
            .unwrap();
        assert_eq!(state.state().ledger().pending_withdrawal_count(), 1);
    }
    let before = state.clone();
    state.apply_block(&block(state.tip(), 111, &[]), &adapter)
        .unwrap();
    assert_eq!(state.state().ledger().pending_withdrawal_count(), 0);
    assert_eq!(state.state().ledger().available_atoms(p(), &a("alice")), 60);
    state.state().ledger().verify_conservation().unwrap();

    let expected = state.tip();
    state.rollback_tip(expected).unwrap();
    assert_eq!(state, before);
    assert_eq!(state.state().ledger().pending_withdrawal_count(), 1);
    state.apply_block(&block(state.tip(), 112, &[]), &adapter)
        .unwrap();
    assert_eq!(state.state().ledger().pending_withdrawal_count(), 0);
    assert_eq!(state.state().ledger().available_atoms(p(), &a("alice")), 60);
}
