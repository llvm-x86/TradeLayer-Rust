//! Stage 12: isolated, unverified type23 NET_SETTLE and type31 KING_SETTLE.
//! All signed-chain, trade proof, and external authorization is synthetic.
use std::collections::BTreeMap;
use tradelayer_core::chain::{BlockHeader, BlockId, CandidateBlock, ChainId, Tip};
use tradelayer_core::execution::{ExecutionError, ProtocolOperation, ProtocolState, UnverifiedTx};
use tradelayer_core::ledger::{
    AccountId, ChannelSide, LedgerError, PropertyId, PropertyKind, SettlementPreview, TxId,
};
use tradelayer_core::registry::RegistryAction;
use tradelayer_core::unified_chain::{
    AuthenticatedProtocolOperation, TrustedProtocolAdapter, UnifiedChainState,
};

fn chain() -> ChainId {
    ChainId([42; 32])
}
fn a(value: &str) -> AccountId {
    AccountId::try_from(value).unwrap()
}
fn p() -> PropertyId {
    PropertyId::try_from(7).unwrap()
}
fn txid(value: u8) -> TxId {
    TxId::from_bytes([value; 32])
}
fn tx(value: u8, actor: &str, height: u64, op: ProtocolOperation) -> UnverifiedTx {
    UnverifiedTx {
        chain: chain(), txid: txid(value), claimed_actor: a(actor),
        claimed_height: height, operation: op,
    }
}
fn activation() -> ProtocolOperation {
    ProtocolOperation::Registry(RegistryAction::Activate {
        types: vec![0, 1, 2, 4, 21, 22, 23, 31], code_hash: [7; 32],
    })
}
fn issue(atoms: u128) -> ProtocolOperation {
    ProtocolOperation::Issue { id: p(), kind: PropertyKind::Fixed, initial_atoms: atoms }
}
fn send_to_bob(atoms: u128) -> ProtocolOperation {
    ProtocolOperation::Send { id: p(), recipient: a("bob"), amount_atoms: atoms }
}
fn commit(side: ChannelSide, atoms: u128) -> ProtocolOperation {
    ProtocolOperation::ChannelCommit {
        id: p(), channel: a("chan"), side, amount_atoms: atoms,
    }
}
fn withdraw(side: ChannelSide, atoms: u128) -> ProtocolOperation {
    ProtocolOperation::ChannelWithdraw {
        id: p(), channel: a("chan"), side, amount_atoms: atoms,
    }
}
fn claim(side: ChannelSide, atoms: u128, reference: u8, expiry: u64) -> ProtocolOperation {
    ProtocolOperation::ChannelNetSettle {
        id: p(), payer_side: side, amount_atoms: atoms,
        trade_reference: txid(reference), expiry_height: expiry,
    }
}
fn king(preview: &SettlementPreview, start: u64, end: u64) -> ProtocolOperation {
    ProtocolOperation::ChannelKingSettle {
        id: p(), start_height: start, end_height: end,
        expected_root: preview.root, expected_count: preview.count,
        payer_side: preview.payer_side, net_atoms: preview.net_atoms,
    }
}
fn funded() -> ProtocolState {
    let mut s = ProtocolState::new_unverified(chain(), a("admin"));
    s.apply_unverified(tx(1, "admin", 100, activation())).unwrap();
    s.apply_unverified(tx(2, "alice", 101, issue(200))).unwrap();
    s.apply_unverified(tx(3, "alice", 102, send_to_bob(100))).unwrap();
    s.apply_unverified(tx(4, "alice", 103, commit(ChannelSide::A, 100))).unwrap();
    s.apply_unverified(tx(5, "bob", 103, commit(ChannelSide::B, 100))).unwrap();
    s
}
fn preview(s: &ProtocolState, start: u64, end: u64, at: u64) -> SettlementPreview {
    s.ledger().preview_king_settlement_unverified(&a("chan"), p(), start, end, at).unwrap()
}
fn assert_balances(s: &ProtocolState, a_units: u128, b_units: u128) {
    assert_eq!(s.ledger().channel_atoms(p(), &a("chan"), ChannelSide::A), a_units);
    assert_eq!(s.ledger().channel_atoms(p(), &a("chan"), ChannelSide::B), b_units);
    s.ledger().verify_conservation().unwrap();
}

#[test]
fn two_sided_netting_journal_moves_only_net_value_and_preserves_supply() {
    let mut s = funded();
    s.apply_unverified(tx(6, "chan", 104, claim(ChannelSide::A, 30, 40, 200))).unwrap();
    s.apply_unverified(tx(7, "chan", 105, claim(ChannelSide::B, 10, 41, 200))).unwrap();
    assert_eq!(s.ledger().pending_net_settlement_count(), 2);
    assert_balances(&s, 100, 100);
    let expected = preview(&s, 104, 105, 106);
    assert_eq!(expected.count, 2);
    assert_eq!(expected.payer_side, ChannelSide::A);
    assert_eq!(expected.net_atoms, 20);
    let initial = s.clone();
    s.apply_unverified(tx(8, "chan", 106, king(&expected, 104, 105))).unwrap();
    assert_balances(&s, 80, 120);
    assert_eq!(s.ledger().pending_net_settlement_count(), 0);
    assert_eq!(s.ledger().swept_net_settlement_count(), 2);
    assert_eq!(s.ledger().king_settlement_count(), 1);

    assert_eq!(
        s.apply_unverified(tx(9, "chan", 107, claim(ChannelSide::A, 1, 40, 200))),
        Err(ExecutionError::Ledger(LedgerError::DuplicateSettlementReference))
    );
    assert_eq!(s.ledger().processed_count(), 7);
    let ledger_bytes = s.ledger().snapshot_bytes().unwrap();
    assert!(ledger_bytes.starts_with(b"TLL\x03"));
    assert_eq!(s.ledger(), &tradelayer_core::ledger::Ledger::from_snapshot_bytes(&ledger_bytes).unwrap());
    let bytes = s.snapshot_bytes().unwrap();
    assert_eq!(s, ProtocolState::from_snapshot_bytes(&bytes, chain(), &a("admin")).unwrap());
    assert_eq!(s.snapshot_bytes().unwrap(), bytes);
    assert_ne!(s, initial);
}

#[test]
fn invalid_roots_wrong_amount_and_wrong_count_leave_everything_atomic() {
    let mut s = funded();
    s.apply_unverified(tx(6, "chan", 104, claim(ChannelSide::A, 15, 40, 200))).unwrap();
    let expected = preview(&s, 104, 104, 105);
    let before = s.clone();

    let mut wrong = king(&expected, 104, 104);
    if let ProtocolOperation::ChannelKingSettle { ref mut expected_root, .. } = wrong {
        expected_root[0] ^= 1;
    }
    assert_eq!(
        s.apply_unverified(tx(7, "chan", 105, wrong)),
        Err(ExecutionError::Ledger(LedgerError::SettlementCommitmentMismatch))
    );
    wrong = king(&expected, 104, 104);
    if let ProtocolOperation::ChannelKingSettle { ref mut expected_count, .. } = wrong {
        *expected_count += 1;
    }
    assert_eq!(
        s.apply_unverified(tx(7, "chan", 105, wrong)),
        Err(ExecutionError::Ledger(LedgerError::SettlementCommitmentMismatch))
    );
    wrong = king(&expected, 104, 104);
    if let ProtocolOperation::ChannelKingSettle { ref mut net_atoms, .. } = wrong {
        *net_atoms += 1;
    }
    assert_eq!(
        s.apply_unverified(tx(7, "chan", 105, wrong)),
        Err(ExecutionError::Ledger(LedgerError::SettlementCommitmentMismatch))
    );
    wrong = king(&expected, 104, 104);
    if let ProtocolOperation::ChannelKingSettle { ref mut payer_side, .. } = wrong {
        *payer_side = ChannelSide::B;
    }
    assert_eq!(
        s.apply_unverified(tx(7, "chan", 105, wrong)),
        Err(ExecutionError::Ledger(LedgerError::SettlementCommitmentMismatch))
    );
    assert_eq!(s, before);
    s.apply_unverified(tx(7, "chan", 105, king(&expected, 104, 104))).unwrap();
    assert_balances(&s, 85, 115);
}

#[test]
fn restricted_port_rejects_bad_activation_missing_owner_expiry_and_future_range() {
    let mut s = ProtocolState::new_unverified(chain(), a("admin"));
    s.apply_unverified(tx(1, "admin", 100, activation())).unwrap();
    s.apply_unverified(tx(2, "alice", 101, issue(200))).unwrap();
    s.apply_unverified(tx(3, "alice", 102, commit(ChannelSide::A, 90))).unwrap();
    assert_eq!(
        s.apply_unverified(tx(6, "chan", 103, claim(ChannelSide::A, 10, 40, 200))),
        Err(ExecutionError::Ledger(LedgerError::ChannelNotBilateral(a("chan"))))
    );
    s.apply_unverified(tx(4, "alice", 103, send_to_bob(100))).unwrap();
    s.apply_unverified(tx(5, "bob", 104, commit(ChannelSide::B, 70))).unwrap();
    assert_eq!(
        s.apply_unverified(tx(6, "chan", 105, claim(ChannelSide::A, 1, 40, 104))),
        Err(ExecutionError::Ledger(LedgerError::InvalidSettlementClaim))
    );
    assert_eq!(
        s.apply_unverified(tx(6, "chan", 105, claim(ChannelSide::A, 1, 6, 200))),
        Err(ExecutionError::Ledger(LedgerError::InvalidSettlementClaim))
    );
    s.apply_unverified(tx(6, "chan", 105, claim(ChannelSide::A, 15, 40, 105))).unwrap();
    assert_eq!(
        s.ledger().preview_king_settlement_unverified(&a("chan"), p(), 105, 105, 106),
        Err(LedgerError::InvalidSettlementClaim)
    );
    let before = s.clone();
    let fake = ProtocolOperation::ChannelKingSettle {
        id: p(), start_height: 105, end_height: 108,
        expected_root: [5; 32], expected_count: 1,
        payer_side: ChannelSide::A, net_atoms: 15,
    };
    assert_eq!(
        s.apply_unverified(tx(7, "chan", 106, fake)),
        Err(ExecutionError::Ledger(LedgerError::InvalidSettlementRange))
    );
    assert_eq!(s, before);
}

#[test]
fn double_claims_cannot_be_swept_twice_or_used_to_bypass_withdrawal_reserves() {
    let mut s = funded();
    s.apply_unverified(tx(6, "chan", 104, claim(ChannelSide::A, 90, 40, 200))).unwrap();
    let first = preview(&s, 104, 104, 106);
    // Reserve 30 tokens for a pending withdrawal before the sweep. The
    // claim is still pending but its larger payout cannot overdraw free escrow.
    s.apply_unverified(tx(7, "alice", 105, withdraw(ChannelSide::A, 30))).unwrap();
    let before = s.clone();
    assert!(matches!(
        s.apply_unverified(tx(8, "chan", 106, king(&first, 104, 104))),
        Err(ExecutionError::Ledger(LedgerError::InsufficientAvailable { .. }))
    ));
    assert_eq!(s, before);
    s.ledger().verify_conservation().unwrap();
    // An alternate sample with no competing withdrawal succeeds once.
    let mut clear = funded();
    clear.apply_unverified(tx(6, "chan", 104, claim(ChannelSide::A, 90, 40, 200))).unwrap();
    let root = preview(&clear, 104, 104, 105);
    clear.apply_unverified(tx(7, "chan", 105, king(&root, 104, 104))).unwrap();
    assert_eq!(
        clear.apply_unverified(tx(8, "chan", 106, king(&root, 104, 104))),
        Err(ExecutionError::Ledger(LedgerError::InvalidSettlementRange))
    );
    assert_balances(&clear, 10, 190);
}

#[test]
fn zero_net_claims_are_not_economically_mutated() {
    let mut s = funded();
    s.apply_unverified(tx(6, "chan", 104, claim(ChannelSide::A, 10, 40, 200))).unwrap();
    s.apply_unverified(tx(7, "chan", 104, claim(ChannelSide::B, 10, 41, 200))).unwrap();
    assert_eq!(
        s.ledger().preview_king_settlement_unverified(&a("chan"), p(), 104, 104, 105),
        Err(LedgerError::ZeroNetSettlement)
    );
    assert_balances(&s, 100, 100);
}

#[derive(Default)]
struct FakeAdapter {
    entries: BTreeMap<u8, AuthenticatedProtocolOperation>,
}
impl FakeAdapter {
    fn add(&mut self, number: u8, actor: &str, operation: ProtocolOperation) {
        self.entries.insert(number, AuthenticatedProtocolOperation {
            txid: txid(number), actor: a(actor), operation,
        });
    }
}
impl TrustedProtocolAdapter for FakeAdapter {
    fn verify_block(&self, _: &CandidateBlock) -> Result<(), String> { Ok(()) }
    fn authenticate_and_decode(
        &self, _: ChainId, _: &BlockHeader, raw: &[u8],
    ) -> Result<Option<AuthenticatedProtocolOperation>, String> {
        let key = *raw.first().ok_or_else(|| "empty synthetic input".to_owned())?;
        self.entries.get(&key).cloned().map(Some)
            .ok_or_else(|| "unknown synthetic input".to_owned())
    }
}
fn anchor() -> Tip {
    Tip { height: 100, id: BlockId([100; 32]) }
}
fn block(tip: Tip, marker: u8, txs: &[u8]) -> CandidateBlock {
    CandidateBlock {
        chain: chain(),
        header: BlockHeader {
            height: tip.height + 1, id: BlockId([marker; 32]), parent: tip.id,
        },
        raw_transactions: txs.iter().map(|number| vec![*number]).collect(),
    }
}
#[test]
fn unified_chain_checkpoint_and_reorg_restore_sweep_and_consumed_claims() {
    let mut state = UnifiedChainState::new_unverified(chain(), anchor(), a("admin"), 12).unwrap();
    let mut fake = FakeAdapter::default();
    fake.add(1, "admin", activation());
    fake.add(2, "alice", issue(200));
    fake.add(3, "alice", send_to_bob(100));
    fake.add(4, "alice", commit(ChannelSide::A, 100));
    fake.add(5, "bob", commit(ChannelSide::B, 100));
    fake.add(6, "chan", claim(ChannelSide::A, 30, 40, 200));
    fake.add(7, "chan", claim(ChannelSide::B, 10, 41, 200));
    state.apply_block(&block(state.tip(), 101, &[1]), &fake).unwrap();
    state.apply_block(&block(state.tip(), 102, &[2]), &fake).unwrap();
    state.apply_block(&block(state.tip(), 103, &[3, 4, 5]), &fake).unwrap();
    state.apply_block(&block(state.tip(), 104, &[6]), &fake).unwrap();
    state.apply_block(&block(state.tip(), 105, &[7]), &fake).unwrap();
    let preview = state.state().ledger().preview_king_settlement_unverified(
        &a("chan"), p(), 104, 105, 106,
    ).unwrap();
    let checkpoint_before = state.checkpoint_bytes().unwrap();
    assert_eq!(
        state, UnifiedChainState::from_checkpoint_bytes(
            &checkpoint_before, chain(), anchor(), &a("admin"), 12
        ).unwrap()
    );
    fake.add(8, "chan", king(&preview, 104, 105));
    let prior = state.clone();
    state.apply_block(&block(state.tip(), 106, &[8]), &fake).unwrap();
    assert_balances(state.state(), 80, 120);
    assert_eq!(state.state().ledger().pending_net_settlement_count(), 0);
    let snap_after = state.checkpoint_bytes().unwrap();
    assert_eq!(state, UnifiedChainState::from_checkpoint_bytes(
        &snap_after, chain(), anchor(), &a("admin"), 12,
    ).unwrap());
    state.rollback_tip(state.tip()).unwrap();
    assert_eq!(state, prior);
    assert_eq!(state.state().ledger().pending_net_settlement_count(), 2);
    assert_balances(state.state(), 100, 100);
    state.apply_block(&block(state.tip(), 107, &[8]), &fake).unwrap();
    assert_balances(state.state(), 80, 120);
}
