//! Synthetic adapter tests only. No cryptographic chain authentication occurs.
use std::collections::BTreeMap;
use tradelayer_core::chain::{BlockHeader, BlockId, CandidateBlock, ChainId, Tip};
use tradelayer_core::execution::{ProtocolOperation, ProtocolState, UnverifiedTx};
use tradelayer_core::ledger::{AccountId, PropertyId, PropertyKind, TxId};
use tradelayer_core::registry::RegistryAction;
use tradelayer_core::unified_chain::{
    AuthenticatedProtocolOperation, TrustedProtocolAdapter, UnifiedChainError, UnifiedChainState,
};

fn account(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn chain() -> ChainId {
    ChainId([5; 32])
}
fn block_id(n: u8) -> BlockId {
    BlockId([n; 32])
}
fn txid(n: u8) -> TxId {
    TxId::from_bytes([n; 32])
}
fn property() -> PropertyId {
    PropertyId::try_from(1).unwrap()
}
fn anchor() -> Tip {
    Tip {
        height: 100,
        id: block_id(100),
    }
}
fn state() -> UnifiedChainState {
    UnifiedChainState::new_unverified(chain(), anchor(), account("admin"), 8).unwrap()
}
fn block(tip: Tip, id: u8, txs: &[u8]) -> CandidateBlock {
    CandidateBlock {
        chain: chain(),
        header: BlockHeader {
            height: tip.height + 1,
            id: block_id(id),
            parent: tip.id,
        },
        raw_transactions: txs.iter().map(|&id| vec![id]).collect(),
    }
}
#[derive(Default)]
struct FakeAdapter {
    allowed: BTreeMap<u8, AuthenticatedProtocolOperation>,
    reject_header: bool,
}
impl FakeAdapter {
    fn add(&mut self, key: u8, origin: &str, operation: ProtocolOperation) {
        self.allowed.insert(
            key,
            AuthenticatedProtocolOperation {
                txid: txid(key),
                actor: account(origin),
                operation,
            },
        );
    }
}
impl TrustedProtocolAdapter for FakeAdapter {
    fn verify_block(&self, _: &CandidateBlock) -> Result<(), String> {
        if self.reject_header {
            Err("bad header".into())
        } else {
            Ok(())
        }
    }
    fn authenticate_and_decode(
        &self,
        _: ChainId,
        _: &BlockHeader,
        raw: &[u8],
    ) -> Result<Option<AuthenticatedProtocolOperation>, String> {
        if raw == [255] {
            return Ok(None);
        }
        let key = *raw.first().ok_or("empty raw material")?;
        self.allowed
            .get(&key)
            .cloned()
            .map(Some)
            .ok_or_else(|| "invalid sender or bytes".into())
    }
}
fn activated() -> ProtocolOperation {
    ProtocolOperation::Registry(RegistryAction::Activate {
        types: vec![0, 1, 2, 7, 8, 9, 11, 12, 13, 14, 15],
        code_hash: [9; 32],
    })
}
fn whitelist() -> ProtocolOperation {
    ProtocolOperation::Registry(RegistryAction::CreateWhitelist {
        name: "Participants".into(),
        url: "".into(),
        description: "KYC".into(),
        backup: account("backup"),
    })
}
fn oracle() -> ProtocolOperation {
    ProtocolOperation::Registry(RegistryAction::CreateOracle {
        ticker: "LTCUSD".into(),
        url: "".into(),
        backup: account("backup"),
        lag: 1,
    })
}
fn issue(n: u128) -> ProtocolOperation {
    ProtocolOperation::Issue {
        id: property(),
        kind: PropertyKind::Managed,
        initial_atoms: n,
    }
}
fn send(n: u128) -> ProtocolOperation {
    ProtocolOperation::Send {
        id: property(),
        recipient: account("bob"),
        amount_atoms: n,
    }
}
fn setup() -> (UnifiedChainState, FakeAdapter) {
    let mut state = state();
    let mut auth = FakeAdapter::default();
    auth.add(1, "admin", activated());
    state
        .apply_block(&block(state.tip(), 101, &[1]), &auth)
        .unwrap();
    (state, auth)
}

#[test]
fn unified_checkpoint_restores_registry_ledger_and_rollbacks() {
    let (mut state, mut auth) = setup();
    auth.add(2, "alice", whitelist());
    auth.add(3, "alice", oracle());
    auth.add(4, "alice", issue(100));
    let b = block(state.tip(), 102, &[2, 3, 4]);
    state.apply_block(&b, &auth).unwrap();
    assert!(state.state().registry().whitelist(1).is_some());
    assert!(state.state().registry().oracle(1).is_some());
    assert_eq!(
        state
            .state()
            .ledger()
            .available_atoms(property(), &account("alice")),
        100
    );
    assert_eq!(state.state().processed_count(), 4);

    let bytes = state.checkpoint_bytes().unwrap();
    let restored =
        UnifiedChainState::from_checkpoint_bytes(&bytes, chain(), anchor(), &account("admin"), 8)
            .unwrap();
    assert_eq!(state, restored);
    assert_eq!(restored.checkpoint_bytes().unwrap(), bytes);
    let mut resumed = restored.clone();
    auth.add(5, "alice", send(40));
    resumed
        .apply_block(&block(resumed.tip(), 103, &[5]), &auth)
        .unwrap();
    assert_eq!(
        resumed
            .state()
            .ledger()
            .available_atoms(property(), &account("bob")),
        40
    );
    resumed.rollback_tip(resumed.tip()).unwrap();
    assert_eq!(resumed, restored);
    resumed
        .apply_block(&block(resumed.tip(), 104, &[5]), &auth)
        .unwrap();
    assert_eq!(
        resumed
            .state()
            .ledger()
            .available_atoms(property(), &account("bob")),
        40
    );
}

#[test]
fn multi_operation_failure_rolls_back_registry_balances_and_replay() {
    let (mut state, mut auth) = setup();
    auth.add(2, "alice", whitelist());
    auth.add(3, "alice", issue(10));
    auth.add(4, "alice", send(11));
    let prior = state.clone();
    assert!(matches!(
        state.apply_block(&block(state.tip(), 102, &[2, 3, 4]), &auth),
        Err(UnifiedChainError::Execution(_))
    ));
    assert_eq!(state, prior);
    assert!(state.state().registry().whitelist(1).is_none());
    auth.add(4, "alice", send(9));
    state
        .apply_block(&block(state.tip(), 102, &[2, 3, 4]), &auth)
        .unwrap();
    assert_eq!(
        state
            .state()
            .ledger()
            .available_atoms(property(), &account("bob")),
        9
    );
}

#[test]
fn skipped_l1_transactions_and_forks_preserve_consistency() {
    let (mut state, mut auth) = setup();
    state
        .apply_block(&block(state.tip(), 102, &[255]), &auth)
        .unwrap();
    assert_eq!(state.state().processed_count(), 1);
    auth.add(3, "alice", oracle());
    state
        .apply_block(&block(state.tip(), 103, &[3]), &auth)
        .unwrap();
    assert_eq!(state.state().registry().oracle_count(), 1);
    state.rollback_tip(state.tip()).unwrap();
    assert_eq!(state.state().registry().oracle_count(), 0);
    auth.add(4, "alice", whitelist());
    state
        .apply_block(&block(state.tip(), 104, &[4]), &auth)
        .unwrap();
    assert_eq!(state.state().registry().whitelist_count(), 1);
    assert_eq!(state.state().registry().oracle_count(), 0);
}

#[test]
fn corrupt_checkpoints_wrong_anchor_and_chain_fail_closed() {
    let (state, _) = setup();
    let bytes = state.checkpoint_bytes().unwrap();
    let mut damaged = bytes.clone();
    damaged[50] ^= 1;
    assert!(UnifiedChainState::from_checkpoint_bytes(
        &damaged,
        chain(),
        anchor(),
        &account("admin"),
        8
    )
    .is_err());
    assert!(UnifiedChainState::from_checkpoint_bytes(
        &bytes,
        ChainId([1; 32]),
        anchor(),
        &account("admin"),
        8
    )
    .is_err());
    assert!(UnifiedChainState::from_checkpoint_bytes(
        &bytes,
        chain(),
        anchor(),
        &account("mallory"),
        8
    )
    .is_err());
    assert!(UnifiedChainState::from_checkpoint_bytes(
        &bytes,
        chain(),
        anchor(),
        &account("admin"),
        7
    )
    .is_err());
    assert!(UnifiedChainState::from_checkpoint_bytes(
        &bytes,
        chain(),
        Tip {
            height: 100,
            id: block_id(2)
        },
        &account("admin"),
        8
    )
    .is_err());
    assert!(UnifiedChainState::from_checkpoint_bytes(
        &bytes[..bytes.len() - 1],
        chain(),
        anchor(),
        &account("admin"),
        8
    )
    .is_err());
}

#[test]
fn reject_misaligned_header_and_unauthenticated_operation() {
    let (mut state, mut auth) = setup();
    let prior = state.clone();
    auth.add(2, "alice", whitelist());
    let mut candidate = block(state.tip(), 102, &[2]);
    candidate.header.parent = block_id(7);
    assert!(matches!(
        state.apply_block(&candidate, &auth),
        Err(UnifiedChainError::ParentMismatch)
    ));
    candidate = block(state.tip(), 102, &[9]);
    assert!(matches!(
        state.apply_block(&candidate, &auth),
        Err(UnifiedChainError::Authentication(_))
    ));
    auth.reject_header = true;
    candidate = block(state.tip(), 102, &[2]);
    assert!(matches!(
        state.apply_block(&candidate, &auth),
        Err(UnifiedChainError::Authentication(_))
    ));
    assert_eq!(state, prior);
}

#[test]
fn immutable_checkpoint_conflict_keeps_old_state() {
    let (mut state, mut auth) = setup();
    auth.add(2, "alice", whitelist());
    let path = std::env::temp_dir().join(format!(
        "tlr-unified-conflict-{}-{}",
        std::process::id(),
        state.tip().height
    ));
    std::fs::write(&path, b"existing").unwrap();
    let before = state.clone();
    assert!(state
        .apply_block_and_checkpoint(&block(state.tip(), 102, &[2]), &auth, &path)
        .is_err());
    assert_eq!(state, before);
    assert_eq!(std::fs::read(&path).unwrap(), b"existing");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn protocol_snapshot_roundtrip_from_unverified_fixture() {
    let mut protocol = ProtocolState::new_unverified(chain(), account("admin"));
    protocol
        .apply_unverified(UnverifiedTx {
            chain: chain(),
            txid: txid(1),
            claimed_actor: account("admin"),
            claimed_height: 101,
            operation: activated(),
        })
        .unwrap();
    let bytes = protocol.snapshot_bytes().unwrap();
    let recovered = ProtocolState::from_snapshot_bytes(&bytes, chain(), &account("admin")).unwrap();
    assert_eq!(recovered, protocol);
    assert_eq!(recovered.snapshot_bytes().unwrap(), bytes);
    let mut corrupt = bytes;
    corrupt[12] ^= 1;
    assert!(ProtocolState::from_snapshot_bytes(&corrupt, chain(), &account("admin")).is_err());
}
