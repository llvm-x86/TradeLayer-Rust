//! Stage25M: historical DIRECT clearlist claims with point-in-time refusal.
//! These are synthetic time-ordered claims, never chain-authenticated events.
use tradelayer_core::channel_admission_audit::{
    AdmissionAuditError, ClaimedDirectAction, ClaimedDirectAttestation, ClaimedEventPosition,
    ClaimedHistoricalDirectView, DirectMatchSide, HistoricalAdmissionKind, PropertyWhitelist,
    Type19ChannelListClaim, Type4CommitListClaim, MAX_CLAIMED_ATTESTATIONS,
};
use tradelayer_core::network_profile::TradeLayerMainnet;

fn at(height: u32, tx_index: u32) -> ClaimedEventPosition {
    ClaimedEventPosition {
        height,
        tx_index,
        block_hash: [u8::try_from(height).unwrap(); 32],
    }
}

fn att<'a>(
    height: u32,
    tx_index: u32,
    clearlist_id: u32,
    address: &'a str,
    action: ClaimedDirectAction,
) -> ClaimedDirectAttestation<'a> {
    ClaimedDirectAttestation {
        network: TradeLayerMainnet::Litecoin,
        at: at(height, tx_index),
        txid_raw: [u8::try_from(tx_index + 1).unwrap(); 32],
        clearlist_id,
        address,
        action,
    }
}

fn view<'a>(
    at_point: ClaimedEventPosition,
    events: &'a [ClaimedDirectAttestation<'a>],
) -> Result<ClaimedHistoricalDirectView<'a>, AdmissionAuditError> {
    ClaimedHistoricalDirectView::audit(TradeLayerMainnet::Litecoin, at_point, events)
}

fn type4<'a>(
    sender: &'a str,
    whitelist: PropertyWhitelist<'a>,
    commit_list: Option<u32>,
) -> Type4CommitListClaim<'a> {
    Type4CommitListClaim {
        sender,
        property_id: 8,
        property_whitelist: whitelist,
        commit_clearlist_id: commit_list,
    }
}

fn type19<'a>(
    a: &'a str,
    b: &'a str,
    lists_a: &'a [u32],
    lists_b: &'a [u32],
    expiry_height: u32,
) -> Type19ChannelListClaim<'a> {
    Type19ChannelListClaim {
        claimed_sender_channel: "channel1",
        registered_channel: "channel1",
        participant_a: a,
        participant_b: b,
        column_a_lists: lists_a,
        column_b_lists: lists_b,
        expiry_height,
    }
}

#[test]
fn direct_type4_property_sender_and_optional_commit_list_are_distinct() {
    let events = [
        att(40, 1, 7, "alice", ClaimedDirectAction::Attest),
        att(40, 2, 9, "alice", ClaimedDirectAction::Attest),
        att(40, 3, 7, "bob", ClaimedDirectAction::Attest),
    ];
    let v = view(at(41, 1), &events).unwrap();
    let result = v
        .type4_commit_lists(type4(
            "alice",
            PropertyWhitelist::AnyDirectList(&[7]),
            Some(9),
        ))
        .unwrap();
    assert_eq!(result.kind(), HistoricalAdmissionKind::Type4Commit);
    assert_eq!(result.matched_list_id(), Some(9));
    assert_eq!(result.matched_side(), Some(DirectMatchSide::CommitSender));
    assert_eq!(result.claimed_position(), at(41, 1));
    assert_eq!(v.claimed_network(), TradeLayerMainnet::Litecoin);
    assert_eq!(
        result.require_authenticated_channel_admission(),
        Err(AdmissionAuditError::IndependentAdmissionProofUnavailable)
    );
    assert_eq!(
        result.require_canonical_signed_revision(),
        Err(AdmissionAuditError::CanonicalChannelRevisionUnspecified)
    );
    assert_eq!(
        v.type4_commit_lists(type4(
            "bob",
            PropertyWhitelist::AnyDirectList(&[7]),
            Some(9)
        )),
        Err(AdmissionAuditError::NoDirectCommitClearlistWitness)
    );
    assert_eq!(
        v.type4_commit_lists(type4("bob", PropertyWhitelist::AnyDirectList(&[9]), None)),
        Err(AdmissionAuditError::NoDirectPropertyWitness)
    );
}

#[test]
fn type4_scalar_zero_and_commit_zero_do_not_construct_authority() {
    let v = view(at(55, 4), &[]).unwrap();
    let claim = type4("alice", PropertyWhitelist::UnrestrictedScalarZero, Some(0));
    let slice = v.type4_commit_lists(claim).unwrap();
    assert_eq!(slice.matched_list_id(), None);
    assert_eq!(
        slice.require_authenticated_channel_admission(),
        Err(AdmissionAuditError::IndependentAdmissionProofUnavailable)
    );
    assert_eq!(
        v.type4_commit_lists(type4(
            "alice",
            PropertyWhitelist::UnrestrictedScalarZero,
            Some(7)
        )),
        Err(AdmissionAuditError::NoDirectCommitClearlistWitness)
    );
    assert_eq!(
        v.type4_commit_lists(type4("alice", PropertyWhitelist::AnyDirectList(&[]), None)),
        Err(AdmissionAuditError::EmptyPropertyWhitelist)
    );
    assert_eq!(
        v.type4_commit_lists(type4("alice", PropertyWhitelist::AnyDirectList(&[0]), None)),
        Err(AdmissionAuditError::AmbiguousPropertyWhitelist)
    );
    assert_eq!(
        v.type4_commit_lists(type4(
            "alice",
            PropertyWhitelist::AnyDirectList(&[0, 7]),
            None
        )),
        Err(AdmissionAuditError::AmbiguousPropertyWhitelist)
    );
    let mut claim = type4("alice", PropertyWhitelist::UnrestrictedScalarZero, None);
    claim.property_id = 2;
    assert_eq!(
        v.type4_commit_lists(claim),
        Err(AdmissionAuditError::ForbiddenVestingProperty)
    );
    claim.property_id = 3;
    assert_eq!(
        v.type4_commit_lists(claim),
        Err(AdmissionAuditError::ForbiddenVestingProperty)
    );
}

#[test]
fn type19_either_side_union_matches_a_direct_member_and_stays_unverified() {
    let events = [
        att(40, 1, 7, "alice", ClaimedDirectAction::Attest),
        att(40, 2, 11, "bob", ClaimedDirectAction::Attest),
    ];
    let v = view(at(41, 2), &events).unwrap();
    let first = v
        .type19_channel_lists(type19("alice", "bob", &[17], &[11], 41))
        .unwrap();
    assert_eq!(first.matched_list_id(), Some(11));
    assert_eq!(first.matched_side(), Some(DirectMatchSide::ChannelB));
    assert_eq!(
        first.require_canonical_signed_revision(),
        Err(AdmissionAuditError::CanonicalChannelRevisionUnspecified)
    );
    assert_eq!(
        first.require_authenticated_channel_admission(),
        Err(AdmissionAuditError::IndependentAdmissionProofUnavailable)
    );
    let second = v
        .type19_channel_lists(type19("alice", "bob", &[7], &[17], 41))
        .unwrap();
    assert_eq!(second.matched_side(), Some(DirectMatchSide::ChannelA));
    assert_eq!(
        v.type19_channel_lists(type19("alice", "bob", &[8], &[10], 41)),
        Err(AdmissionAuditError::NoDirectChannelWitness)
    );
}

#[test]
fn type19_empty_lists_do_not_imply_any_valid_channel_revision() {
    let v = view(at(60, 1), &[]).unwrap();
    let result = v
        .type19_channel_lists(type19("alice", "bob", &[], &[], 60))
        .unwrap();
    assert_eq!(result.matched_list_id(), None);
    assert_eq!(result.matched_side(), None);
    assert_eq!(
        result.require_authenticated_channel_admission(),
        Err(AdmissionAuditError::IndependentAdmissionProofUnavailable)
    );
    assert_eq!(
        result.require_canonical_signed_revision(),
        Err(AdmissionAuditError::CanonicalChannelRevisionUnspecified)
    );
    assert_eq!(
        v.type19_channel_lists(type19("alice", "bob", &[], &[], 59)),
        Err(AdmissionAuditError::ExpiredChannelTrade)
    );
    let mut wrong_sender = type19("alice", "bob", &[], &[], 60);
    wrong_sender.claimed_sender_channel = "forged_channel";
    assert_eq!(
        v.type19_channel_lists(wrong_sender),
        Err(AdmissionAuditError::WrongChannelSender)
    );
    assert_eq!(
        v.type19_channel_lists(type19("alice", "alice", &[], &[], 60)),
        Err(AdmissionAuditError::IncompleteChannelParticipants)
    );
}

#[test]
fn revoke_at_prior_index_removes_only_the_revoked_membership() {
    let events = [
        att(40, 1, 7, "alice", ClaimedDirectAction::Attest),
        att(41, 1, 7, "alice", ClaimedDirectAction::Revoke),
        att(41, 2, 7, "bob", ClaimedDirectAction::Attest),
    ];
    let before = view(at(41, 1), &events[..1]).unwrap();
    assert!(before
        .type4_commit_lists(type4("alice", PropertyWhitelist::AnyDirectList(&[7]), None))
        .is_ok());
    let after_revoke = view(at(41, 2), &events[..2]).unwrap();
    assert_eq!(
        after_revoke.type4_commit_lists(type4(
            "alice",
            PropertyWhitelist::AnyDirectList(&[7]),
            None
        )),
        Err(AdmissionAuditError::NoDirectPropertyWitness)
    );
    assert_eq!(
        after_revoke.type19_channel_lists(type19("alice", "bob", &[7], &[], 41)),
        Err(AdmissionAuditError::NoDirectChannelWitness)
    );
    let after_bob = view(at(41, 3), &events).unwrap();
    let member = after_bob
        .type19_channel_lists(type19("alice", "bob", &[7], &[], 41))
        .unwrap();
    assert_eq!(member.matched_side(), Some(DirectMatchSide::ChannelB));
    assert_eq!(
        after_bob.type4_commit_lists(type4("alice", PropertyWhitelist::AnyDirectList(&[7]), None)),
        Err(AdmissionAuditError::NoDirectPropertyWitness)
    );
}

#[test]
fn future_and_same_transaction_memberships_are_rejected_even_if_approved() {
    let grant = att(41, 5, 7, "alice", ClaimedDirectAction::Attest);
    assert!(matches!(
        view(at(41, 4), &[grant]),
        Err(AdmissionAuditError::FutureOrSameTransactionEvidence)
    ));
    assert!(matches!(
        view(at(41, 5), &[grant]),
        Err(AdmissionAuditError::FutureOrSameTransactionEvidence)
    ));
    assert!(matches!(
        view(at(40, 9), &[grant]),
        Err(AdmissionAuditError::FutureOrSameTransactionEvidence)
    ));
    let events = [grant];
    let v = view(at(41, 6), &events).unwrap();
    assert!(v
        .type4_commit_lists(type4("alice", PropertyWhitelist::AnyDirectList(&[7]), None))
        .is_ok());
}

#[test]
fn same_height_conflicting_block_id_cannot_prove_membership() {
    let mut grant = att(42, 1, 7, "alice", ClaimedDirectAction::Attest);
    grant.at.block_hash[0] ^= 1;
    assert!(matches!(
        view(at(42, 4), &[grant]),
        Err(AdmissionAuditError::ConflictingBlockAtHeight)
    ));
    let first = att(41, 1, 7, "alice", ClaimedDirectAction::Attest);
    let mut other_fork = att(41, 2, 9, "alice", ClaimedDirectAction::Attest);
    other_fork.at.block_hash[0] ^= 1;
    assert!(matches!(
        view(at(42, 4), &[first, other_fork]),
        Err(AdmissionAuditError::ConflictingBlockAtHeight)
    ));
}

#[test]
fn duplicate_out_of_order_and_cross_network_event_streams_refuse() {
    let a = att(40, 1, 7, "alice", ClaimedDirectAction::Attest);
    let b = att(40, 2, 9, "bob", ClaimedDirectAction::Attest);
    assert!(matches!(
        view(at(41, 1), &[b, a]),
        Err(AdmissionAuditError::OutOfOrderEvidence)
    ));
    let duplicate = att(40, 1, 9, "bob", ClaimedDirectAction::Attest);
    assert!(matches!(
        view(at(41, 1), &[a, duplicate]),
        Err(AdmissionAuditError::DuplicateTransactionPosition)
    ));
    let mut foreign = b;
    foreign.network = TradeLayerMainnet::Bitcoin;
    assert!(matches!(
        view(at(41, 1), &[a, foreign]),
        Err(AdmissionAuditError::ForeignNetworkEvidence)
    ));
    assert!(matches!(
        ClaimedHistoricalDirectView::audit(TradeLayerMainnet::Dogecoin, at(41, 1), &[]),
        Err(AdmissionAuditError::UnsupportedNetwork)
    ));
}

#[test]
fn malformed_positions_and_too_many_events_refuse() {
    let mut bad = att(40, 1, 7, "alice", ClaimedDirectAction::Attest);
    bad.txid_raw = [0; 32];
    assert!(matches!(
        view(at(41, 1), &[bad]),
        Err(AdmissionAuditError::BadEventPosition)
    ));
    assert!(matches!(
        view(
            ClaimedEventPosition {
                block_hash: [0; 32],
                ..at(41, 1)
            },
            &[]
        ),
        Err(AdmissionAuditError::BadEventPosition)
    ));
    bad = att(40, 1, 7, "alice", ClaimedDirectAction::Attest);
    bad.address = " alice ";
    assert!(matches!(
        view(at(41, 1), &[bad]),
        Err(AdmissionAuditError::BadSubject)
    ));
    let oversized =
        vec![att(40, 1, 7, "alice", ClaimedDirectAction::Attest); MAX_CLAIMED_ATTESTATIONS + 1];
    assert!(matches!(
        view(at(41, 1), &oversized),
        Err(AdmissionAuditError::TooManyObservations)
    ));
}

#[test]
fn derived_only_claims_do_not_fabricate_a_direct_member_or_consensus_veto() {
    // The historical validator can accept an xpub-derived address. Without
    // the derivation and its historical root, this direct-only model cannot
    // establish either legitimacy or historical invalidity.
    let events = [att(44, 1, 9, "rootOwner", ClaimedDirectAction::Attest)];
    let v = view(at(45, 3), &events).unwrap();
    assert_eq!(
        v.type19_channel_lists(type19("derived_child", "other", &[9], &[], 45)),
        Err(AdmissionAuditError::NoDirectChannelWitness)
    );
    assert_eq!(
        v.type4_commit_lists(type4(
            "derived_child",
            PropertyWhitelist::UnrestrictedScalarZero,
            Some(9)
        )),
        Err(AdmissionAuditError::NoDirectCommitClearlistWitness)
    );
}
