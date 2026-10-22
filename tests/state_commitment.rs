//! Pure, read-only TLPT statement tests. No consensus authentication/proofs.
use tradelayer_core::chain::{BlockHeader, BlockId, CandidateBlock, ChainId, Tip};
use tradelayer_core::execution::ProtocolState;
use tradelayer_core::ledger::AccountId;
use tradelayer_core::state_commitment::{
    chain_head_root_unverified, CommitmentError, TransitionStatement, TRANSITION_BYTES,
};
use tradelayer_core::unified_chain::{
    AuthenticatedProtocolOperation, TrustedProtocolAdapter, UnifiedChainState,
};

fn account(input: &str) -> AccountId {
    AccountId::try_from(input).unwrap()
}
fn chain() -> ChainId {
    ChainId([7; 32])
}
fn anchor() -> Tip {
    Tip {
        height: 100,
        id: BlockId([100; 32]),
    }
}
fn state() -> UnifiedChainState {
    UnifiedChainState::new_unverified(chain(), anchor(), account("admin"), 4).unwrap()
}
fn block(tip: Tip, id: u8) -> CandidateBlock {
    CandidateBlock {
        chain: chain(),
        header: BlockHeader {
            height: tip.height + 1,
            id: BlockId([id; 32]),
            parent: tip.id,
        },
        raw_transactions: vec![vec![255]],
    }
}
struct SyntheticOnly;
impl TrustedProtocolAdapter for SyntheticOnly {
    fn verify_block(&self, _: &CandidateBlock) -> Result<(), String> {
        Ok(())
    }
    fn authenticate_and_decode(
        &self,
        _: ChainId,
        _: &BlockHeader,
        raw: &[u8],
    ) -> Result<Option<AuthenticatedProtocolOperation>, String> {
        if raw == [255] {
            Ok(None)
        } else {
            Err("unsupported synthetic raw fixture".to_owned())
        }
    }
}

fn hex(input: &str) -> Vec<u8> {
    assert!(input.len().is_multiple_of(2));
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16).unwrap() as u8;
            let lo = (pair[1] as char).to_digit(16).unwrap() as u8;
            (hi << 4) | lo
        })
        .collect()
}
fn encode_hex(input: &[u8]) -> String {
    input.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn independently_calculated_public_input_vector_roundtrips_byte_exact() {
    let fixture = include_str!("../fixtures/tlpt_v1_vectors.tsv").trim();
    let (id, canonical) = fixture.split_once('\t').unwrap();
    let bytes = hex(canonical);
    assert_eq!(bytes.len(), TRANSITION_BYTES);
    let statement = TransitionStatement::from_canonical_bytes_unverified(&bytes).unwrap();
    assert_eq!(encode_hex(&statement.id()), id);
    assert_eq!(statement.chain(), ChainId([1; 32]));
    assert_eq!(
        statement.parent(),
        Tip {
            height: 123,
            id: BlockId([2; 32]),
        }
    );
    assert_eq!(
        statement.child(),
        Tip {
            height: 124,
            id: BlockId([3; 32]),
        }
    );
    assert_eq!(statement.old_protocol_root(), [9; 32]);
    assert_eq!(statement.new_protocol_root(), [8; 32]);
    assert_eq!(statement.canonical_bytes(), bytes);

    let reference = chain_head_root_unverified(ChainId([1; 32]), statement.parent(), [9; 32]);
    assert_eq!(
        encode_hex(&reference),
        "71f25d1ec7a279636484a8c178ac8146f7c30029f38b92bb4ab075f3d1c39842"
    );
}

#[test]
fn successor_transition_binding_follows_reversible_rollbacks_and_forks() {
    let mut after = state();
    let before = after.clone();
    let before_commitment = before.state_commitment_unverified().unwrap();
    let original_protocol = ProtocolState::new_unverified(chain(), account("admin"));
    assert_eq!(
        before_commitment.protocol_root,
        original_protocol.state_root_unverified().unwrap()
    );

    after
        .apply_block(&block(after.tip(), 101), &SyntheticOnly)
        .unwrap();
    let after_commitment = after.state_commitment_unverified().unwrap();
    assert_ne!(
        after_commitment.protocol_root,
        before_commitment.protocol_root
    );
    assert_ne!(after_commitment.head_root, before_commitment.head_root);

    let statement =
        TransitionStatement::from_reversible_states_unverified(&before, &after).unwrap();
    assert_eq!(statement.chain(), chain());
    assert_eq!(statement.parent(), before.tip());
    assert_eq!(statement.child(), after.tip());
    assert_eq!(
        statement.old_protocol_root(),
        before_commitment.protocol_root
    );
    assert_eq!(
        statement.new_protocol_root(),
        after_commitment.protocol_root
    );
    let bytes = statement.canonical_bytes();
    assert_eq!(bytes.len(), TRANSITION_BYTES);
    assert_eq!(
        statement,
        TransitionStatement::from_canonical_bytes_unverified(&bytes).unwrap()
    );

    let checkpoint = after.checkpoint_bytes().unwrap();
    let resumed = UnifiedChainState::from_checkpoint_bytes(
        &checkpoint,
        chain(),
        anchor(),
        &account("admin"),
        4,
    )
    .unwrap();
    assert_eq!(
        resumed.state_commitment_unverified().unwrap(),
        after_commitment
    );

    let mut tampered = bytes;
    tampered[120] ^= 1;
    assert!(TransitionStatement::from_canonical_bytes_unverified(&tampered).is_err());

    // A valid local successor can be replaced by an alternative block at the
    // same height after a reorg, yielding a distinct public statement.
    let mut alternative = after.clone();
    alternative.rollback_tip(alternative.tip()).unwrap();
    assert_eq!(alternative, before);
    alternative
        .apply_block(&block(alternative.tip(), 111), &SyntheticOnly)
        .unwrap();
    let other =
        TransitionStatement::from_reversible_states_unverified(&before, &alternative).unwrap();
    assert_ne!(statement.id(), other.id());
}

#[test]
fn wrong_ancestry_wrong_network_or_nonsequential_tips_fail_closed() {
    let mut after = state();
    let before = after.clone();
    assert!(matches!(
        TransitionStatement::from_reversible_states_unverified(&before, &before),
        Err(CommitmentError::NotSuccessive)
    ));
    after
        .apply_block(&block(after.tip(), 101), &SyntheticOnly)
        .unwrap();
    let different_admin =
        UnifiedChainState::new_unverified(chain(), anchor(), account("mallory"), 4).unwrap();
    assert!(matches!(
        TransitionStatement::from_reversible_states_unverified(&different_admin, &after),
        Err(CommitmentError::MissingRollbackAncestor)
    ));
    let other_chain =
        UnifiedChainState::new_unverified(ChainId([5; 32]), anchor(), account("admin"), 4).unwrap();
    assert!(matches!(
        TransitionStatement::from_reversible_states_unverified(&other_chain, &after),
        Err(CommitmentError::WrongChain)
    ));
    assert!(TransitionStatement::from_canonical_bytes_unverified(&[0; 10]).is_err());
    let mut bytes = TransitionStatement::from_reversible_states_unverified(&before, &after)
        .unwrap()
        .canonical_bytes();
    bytes[0] ^= 1;
    assert!(TransitionStatement::from_canonical_bytes_unverified(&bytes).is_err());
}
