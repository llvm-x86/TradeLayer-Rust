//! Synthetic tests for the trust boundary. FakeAdapter is intentionally not a
//! chain verifier. It may never be used with real funds or mainnet data.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use tradelayer_core::chain::{
    AuthenticatedOperation, BlockHeader, BlockId, CandidateBlock, ChainError, ChainId, ChainState,
    CheckpointError, Tip, TrustedChainAdapter,
};
use tradelayer_core::ledger::{AccountId, LedgerError, Operation, PropertyId, PropertyKind, TxId};

fn a(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn tx(n: u8) -> TxId {
    TxId::from_bytes([n; 32])
}
fn id(n: u8) -> BlockId {
    BlockId([n; 32])
}
fn chain() -> ChainId {
    ChainId([77; 32])
}
fn anchor() -> Tip {
    Tip {
        height: 100,
        id: id(5),
    }
}
fn state(limit: usize) -> ChainState {
    ChainState::new(chain(), anchor(), limit).unwrap()
}
fn issue(initial_atoms: u128) -> Operation {
    Operation::CreateProperty {
        id: p(8),
        issuer: a("alice"),
        kind: PropertyKind::Managed,
        initial_atoms,
    }
}
fn transfer(from: &str, to: &str, amount_atoms: u128) -> Operation {
    Operation::Transfer {
        id: p(8),
        from: a(from),
        to: a(to),
        amount_atoms,
    }
}
fn mint(issuer: &str, to: &str, atoms: u128) -> Operation {
    Operation::Mint {
        id: p(8),
        issuer: a(issuer),
        to: a(to),
        amount_atoms: atoms,
    }
}
fn burn(issuer: &str, atoms: u128) -> Operation {
    Operation::Burn {
        id: p(8),
        issuer: a(issuer),
        amount_atoms: atoms,
    }
}
fn block(parent: Tip, new_id: u8, keys: &[u8]) -> CandidateBlock {
    CandidateBlock {
        chain: chain(),
        header: BlockHeader {
            height: parent.height + 1,
            parent: parent.id,
            id: id(new_id),
        },
        raw_transactions: keys.iter().copied().map(|key| vec![key]).collect(),
    }
}

fn apply_next<A: TrustedChainAdapter>(
    state: &mut ChainState,
    new_id: u8,
    keys: &[u8],
    adapter: &A,
) -> Result<(), ChainError> {
    let next = block(state.tip(), new_id, keys);
    state.apply_block(&next, adapter)
}
fn rewind(state: &mut ChainState) -> Result<(), ChainError> {
    let expected = state.tip();
    state.rollback_tip(expected)
}
fn checkpoint_next<A: TrustedChainAdapter>(
    state: &mut ChainState,
    new_id: u8,
    keys: &[u8],
    adapter: &A,
    path: &std::path::Path,
) -> Result<(), ChainError> {
    let next = block(state.tip(), new_id, keys);
    state.apply_block_and_checkpoint(&next, adapter, path)
}

#[derive(Default)]
struct FakeAdapter {
    accepted: BTreeMap<u8, AuthenticatedOperation>,
    refuse_block: bool,
}

impl FakeAdapter {
    fn allow(&mut self, key: u8, txid: u8, origin: &str, operation: Operation) {
        self.accepted.insert(
            key,
            AuthenticatedOperation {
                txid: tx(txid),
                origin: a(origin),
                operation,
            },
        );
    }
}

impl TrustedChainAdapter for FakeAdapter {
    fn verify_block(&self, _: &CandidateBlock) -> Result<(), String> {
        if self.refuse_block {
            Err("unverified block".into())
        } else {
            Ok(())
        }
    }

    fn authenticate_and_decode(
        &self,
        _chain: ChainId,
        _header: &BlockHeader,
        raw: &[u8],
    ) -> Result<AuthenticatedOperation, String> {
        self.accepted
            .get(raw.first().ok_or("empty raw input")?)
            .cloned()
            .ok_or_else(|| "unsigned or unknown transaction".into())
    }
}

#[test]
fn verified_origin_must_match_mutating_actor() {
    let mut state = state(4);
    let mut verifier = FakeAdapter::default();
    verifier.allow(1, 1, "mallory", issue(100));
    let before = state.clone();
    assert!(matches!(
        state.apply_block(&block(anchor(), 6, &[1]), &verifier),
        Err(ChainError::ActorMismatch)
    ));
    assert_eq!(state, before);

    verifier.allow(1, 1, "alice", issue(100));
    state
        .apply_block(&block(anchor(), 6, &[1]), &verifier)
        .unwrap();
    assert_eq!(state.ledger().available_atoms(p(8), &a("alice")), 100);

    let before = state.clone();
    for operation in [
        transfer("alice", "bob", 1),
        mint("alice", "bob", 1),
        burn("alice", 1),
    ] {
        verifier.allow(2, 2, "mallory", operation);
        assert!(matches!(
            apply_next(&mut state, 7, &[2], &verifier),
            Err(ChainError::ActorMismatch)
        ));
        assert_eq!(state, before);
    }
}

#[test]
fn reject_unsigned_block_mismatched_parent_network_and_height() {
    let mut state = state(5);
    let mut verifier = FakeAdapter::default();
    verifier.allow(1, 1, "alice", issue(100));
    let before = state.clone();
    verifier.refuse_block = true;
    assert!(matches!(
        state.apply_block(&block(anchor(), 6, &[1]), &verifier),
        Err(ChainError::Authentication(_))
    ));
    assert_eq!(state, before);
    verifier.refuse_block = false;
    let mut candidate = block(anchor(), 6, &[1]);
    candidate.chain = ChainId([8; 32]);
    assert!(matches!(
        state.apply_block(&candidate, &verifier),
        Err(ChainError::WrongChain)
    ));
    candidate.chain = chain();
    candidate.header.parent = id(99);
    assert!(matches!(
        state.apply_block(&candidate, &verifier),
        Err(ChainError::BadParent)
    ));
    candidate.header.parent = anchor().id;
    candidate.header.height += 1;
    assert!(matches!(
        state.apply_block(&candidate, &verifier),
        Err(ChainError::BadHeight)
    ));
    assert_eq!(state, before);
}

#[test]
fn late_failure_rolls_back_whole_block_and_replay_markers() {
    let mut state = state(5);
    let mut adapter = FakeAdapter::default();
    adapter.allow(1, 1, "alice", issue(100));
    state
        .apply_block(&block(anchor(), 6, &[1]), &adapter)
        .unwrap();
    let before = state.clone();
    adapter.allow(2, 2, "alice", transfer("alice", "bob", 60));
    adapter.allow(3, 3, "alice", transfer("alice", "carol", 50));
    assert!(matches!(
        apply_next(&mut state, 7, &[2, 3], &adapter),
        Err(ChainError::Ledger(
            LedgerError::InsufficientAvailable { .. }
        ))
    ));
    assert_eq!(state, before);
    assert!(!state.ledger().is_processed(tx(2)));
    assert!(!state.ledger().is_processed(tx(3)));
    adapter.allow(3, 3, "alice", transfer("alice", "carol", 40));
    apply_next(&mut state, 7, &[2, 3], &adapter).unwrap();
    assert_eq!(state.ledger().available_atoms(p(8), &a("bob")), 60);
    assert_eq!(state.ledger().available_atoms(p(8), &a("carol")), 40);
    assert_eq!(state.ledger().verify_conservation(), Ok(()));
}

#[test]
fn rollback_unwinds_replay_and_replaces_tip_with_different_fork() {
    let mut state = state(3);
    let mut adapter = FakeAdapter::default();
    adapter.allow(1, 1, "alice", issue(100));
    state
        .apply_block(&block(anchor(), 6, &[1]), &adapter)
        .unwrap();
    let checkpoint = state.clone();
    adapter.allow(2, 2, "alice", transfer("alice", "bob", 60));
    apply_next(&mut state, 7, &[2], &adapter).unwrap();
    let tip_before = state.tip();
    assert!(matches!(
        state.rollback_tip(anchor()),
        Err(ChainError::WrongRollbackTip)
    ));
    state.rollback_tip(tip_before).unwrap();
    assert_eq!(state, checkpoint);
    adapter.allow(3, 3, "alice", transfer("alice", "carol", 20));
    apply_next(&mut state, 8, &[3], &adapter).unwrap();
    assert_eq!(state.ledger().available_atoms(p(8), &a("bob")), 0);
    assert_eq!(state.ledger().available_atoms(p(8), &a("carol")), 20);
    assert!(!state.ledger().is_processed(tx(2)));
    // Restored txid can be processed again if the fork includes it.
    rewind(&mut state).unwrap();
    apply_next(&mut state, 9, &[2], &adapter).unwrap();
    assert!(state.ledger().is_processed(tx(2)));
}

#[test]
fn checkpoint_recovers_history_without_mutating_supply() {
    let mut state = state(4);
    let mut adapter = FakeAdapter::default();
    adapter.allow(1, 1, "alice", issue(100));
    adapter.allow(2, 2, "alice", transfer("alice", "bob", 40));
    state
        .apply_block(&block(anchor(), 6, &[1]), &adapter)
        .unwrap();
    apply_next(&mut state, 7, &[2], &adapter).unwrap();
    let encoded = state.checkpoint_bytes().unwrap();
    let decoded = ChainState::from_checkpoint_bytes(&encoded, chain(), anchor(), 4).unwrap();
    assert_eq!(decoded, state);
    assert_eq!(decoded.checkpoint_bytes().unwrap(), encoded);
    let mut recovered = decoded;
    rewind(&mut recovered).unwrap();
    assert_eq!(
        recovered.tip(),
        Tip {
            height: 101,
            id: id(6)
        }
    );
    assert_eq!(recovered.ledger().available_atoms(p(8), &a("bob")), 0);
    assert!(!recovered.ledger().is_processed(tx(2)));
    assert_eq!(recovered.ledger().verify_conservation(), Ok(()));

    assert!(ChainState::from_checkpoint_bytes(&encoded, ChainId([1; 32]), anchor(), 4).is_err());
    assert!(ChainState::from_checkpoint_bytes(
        &encoded,
        chain(),
        Tip {
            height: 100,
            id: id(0)
        },
        4
    )
    .is_err());
    assert!(ChainState::from_checkpoint_bytes(&encoded, chain(), anchor(), 6).is_err());
    for n in 0..encoded.len() {
        assert!(ChainState::from_checkpoint_bytes(&encoded[..n], chain(), anchor(), 4).is_err());
    }
    let mut tampered = encoded;
    tampered[75] ^= 1;
    assert!(ChainState::from_checkpoint_bytes(&tampered, chain(), anchor(), 4).is_err());
}

#[test]
fn bounded_history_fails_closed_on_deep_reorg() {
    let mut state = state(2);
    let adapter = FakeAdapter::default();
    for n in 0..5 {
        let candidate = block(state.tip(), n + 10, &[]);
        state.apply_block(&candidate, &adapter).unwrap();
    }
    assert_eq!(state.rollback_depth(), 2);
    let recovered =
        ChainState::from_checkpoint_bytes(&state.checkpoint_bytes().unwrap(), chain(), anchor(), 2)
            .unwrap();
    assert_eq!(recovered, state);
    for _ in 0..2 {
        rewind(&mut state).unwrap();
    }
    let before = state.clone();
    assert!(matches!(
        rewind(&mut state),
        Err(ChainError::EmptyRollbackHistory)
    ));
    assert_eq!(state, before);
}

fn unique_checkpoint_path() -> PathBuf {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("tradelayer-test-{}-{n}.chk", std::process::id()))
}

#[test]
fn immutable_file_checkpoint_and_reopen() {
    let mut state = state(2);
    let mut adapter = FakeAdapter::default();
    adapter.allow(1, 1, "alice", issue(100));
    let path = unique_checkpoint_path();
    state
        .apply_block_and_checkpoint(&block(anchor(), 6, &[1]), &adapter, &path)
        .unwrap();
    let mut recovered = ChainState::load_checkpoint(&path, chain(), anchor(), 2).unwrap();
    assert_eq!(recovered, state);
    let original = state.clone();
    adapter.allow(2, 2, "alice", transfer("alice", "bob", 5));
    assert!(matches!(
        checkpoint_next(&mut state, 7, &[2], &adapter, &path),
        Err(ChainError::Checkpoint(CheckpointError::Io(_)))
    ));
    assert_eq!(state, original);
    assert_eq!(recovered, original);
    rewind(&mut recovered).unwrap();
    assert_eq!(recovered.tip(), anchor());
    fs::remove_file(path).unwrap();
}

#[test]
fn snapshot_handles_completely_redeemed_managed_token() {
    let mut state = state(2);
    let mut adapter = FakeAdapter::default();
    adapter.allow(1, 1, "alice", issue(5));
    adapter.allow(2, 2, "alice", burn("alice", 5));
    state
        .apply_block(&block(anchor(), 6, &[1]), &adapter)
        .unwrap();
    apply_next(&mut state, 7, &[2], &adapter).unwrap();
    assert_eq!(state.ledger().property(p(8)).unwrap().total_supply_atoms, 0);
    assert_eq!(state.ledger().verify_conservation(), Ok(()));
    assert_eq!(
        ChainState::from_checkpoint_bytes(&state.checkpoint_bytes().unwrap(), chain(), anchor(), 2)
            .unwrap(),
        state
    );
}

#[test]
fn replay_rejected_across_blocks_without_poisoning_state() {
    let mut state = state(2);
    let mut adapter = FakeAdapter::default();
    adapter.allow(1, 1, "alice", issue(100));
    state
        .apply_block(&block(anchor(), 6, &[1]), &adapter)
        .unwrap();
    let before = state.clone();
    assert!(matches!(
        apply_next(&mut state, 7, &[1], &adapter),
        Err(ChainError::Ledger(LedgerError::AlreadyProcessed(_)))
    ));
    assert_eq!(state, before);
}
