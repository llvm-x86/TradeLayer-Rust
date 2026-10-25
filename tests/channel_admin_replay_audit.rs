//! Stage25P: source-conditional historical type7/8/9 replay remains unverified.
use tradelayer_core::bitcoin_inclusion::{BitcoinPowProfile, CheckedHeaderSegment, MerklePath};
use tradelayer_core::chain::{BlockId, Tip};
use tradelayer_core::channel_admin_replay_audit::{
    audit_bitcoin_projected_type789_history_unverified, ClaimedType789Event, ProjectedAdminError,
    UnverifiedProjectedAdminHistory,
};
use tradelayer_core::channel_admission_audit::{
    AdmissionAuditError, ClaimedEventPosition, HistoricalAdmissionKind, PropertyWhitelist,
    Type4CommitListClaim,
};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn f(key: &str) -> String {
    include_str!("../fixtures/channel_admin_789_merkle_synthetic.tsv")
        .lines()
        .find_map(|line| {
            let (k, value) = line.split_once('\t')?;
            (k == key).then_some(value.to_owned())
        })
        .unwrap()
}
fn raw(i: usize) -> Vec<u8> {
    decode_hex(&f(&format!("tx{i}_hex"))).unwrap()
}
fn hash(s: &str) -> [u8; 32] {
    decode_hex(s).unwrap().try_into().unwrap()
}
fn point(i: usize) -> ClaimedEventPosition {
    ClaimedEventPosition {
        height: 101,
        tx_index: i as u32,
        block_hash: hash(&f("block_hash_display")),
    }
}
fn path(i: usize) -> MerklePath {
    MerklePath {
        tx_index: i as u32,
        tx_count: 7,
        siblings_raw: f(&format!("tx{i}_merkle")).split(',').map(hash).collect(),
    }
}
fn segment() -> CheckedHeaderSegment {
    let h: [u8; 80] = decode_hex(&f("header_hex")).unwrap().try_into().unwrap();
    CheckedHeaderSegment::check_against_claimed_anchor(
        Tip { height: 100, id: BlockId([0x66; 32]) },
        &[h], BitcoinPowProfile::RegtestLike,
    ).unwrap()
}
fn events<'a>(
    tx: &'a [Vec<u8>; 7], paths: &'a [MerklePath; 7],
    index: usize, sender: &'a str, next: Option<u32>,
) -> ClaimedType789Event<'a> {
    ClaimedType789Event {
        at: point(index), raw_transaction: &tx[index], path: &paths[index],
        claimed_first_input_sender: sender, claimed_type_active: true,
        expected_new_list_id: next,
    }
}
fn data() -> ([Vec<u8>; 7], [MerklePath; 7]) {
    ([raw(0),raw(1),raw(2),raw(3),raw(4),raw(5),raw(6)],
     [path(0),path(1),path(2),path(3),path(4),path(5),path(6)])
}
fn check(
    target: usize, tx: &[Vec<u8>; 7], paths: &[MerklePath; 7],
    records: &[ClaimedType789Event<'_>],
) -> Result<UnverifiedProjectedAdminHistory, ProjectedAdminError> {
    audit_bitcoin_projected_type789_history_unverified(
        TradeLayerMainnet::Bitcoin, &segment(), point(target),
        &tx[target], &paths[target], HistoricalAdmissionKind::Type4Commit,
        1, records,
    )
}
fn alice() -> Type4CommitListClaim<'static> {
    Type4CommitListClaim {
        sender:"alice",property_id:8,
        property_whitelist:PropertyWhitelist::AnyDirectList(&[1]),
        commit_clearlist_id:None,
    }
}

#[test]
fn creation_and_direct_attestation_before_rotation_are_projection_only() {
    let (tx, paths) = data();
    let history = [events(&tx,&paths,0,"adminA",Some(1)),
                   events(&tx,&paths,1,"adminA",None)];
    let report = check(2,&tx,&paths,&history).unwrap();
    assert_eq!(report.observed_source_event_count(),2);
    assert_eq!(report.next_unverified_list_id(),2);
    let list = report.projected_list(1).unwrap();
    assert_eq!(list.projected_primary_admin(),"adminA");
    assert_eq!(list.projected_backup(),"backupB");
    assert!(list.projected_direct_member("alice"));
    assert_eq!(report.type8_lookup_ambiguity_count(),0);
    let sub = report.type4_projected_direct_subset(alice()).unwrap();
    assert_eq!(sub.matched_list_id(),Some(1));
    assert_eq!(sub.require_authenticated_channel_admission(),
        Err(AdmissionAuditError::IndependentAdmissionProofUnavailable));
    assert_eq!(report.require_authenticated_historical_admin(),
        Err(ProjectedAdminError::HistoricalSourceAuthorityUnavailable));
    assert_eq!(report.require_canonical_signed_channel_revision(),
        Err(ProjectedAdminError::CanonicalSignedRevisionUnspecified));
}

#[test]
fn conditional_type8_rotation_then_type9_revocation_is_flagged() {
    let (tx, paths) = data();
    let history = [events(&tx,&paths,0,"adminA",Some(1)),
                   events(&tx,&paths,1,"adminA",None),
                   events(&tx,&paths,3,"adminA",None),
                   events(&tx,&paths,4,"adminC",None)];
    let report = check(5,&tx,&paths,&history).unwrap();
    let list = report.projected_list(1).unwrap();
    assert_eq!(list.projected_primary_admin(),"adminC");
    assert!(!list.projected_direct_member("alice"));
    assert_eq!(report.type8_lookup_ambiguity_count(),1);
    assert_eq!(report.require_verified_type8_transition(),
        Err(ProjectedAdminError::Type8ValidatorLookupUnresolved));
    assert_eq!(report.type4_projected_direct_subset(alice()),
        Err(ProjectedAdminError::DirectSubset(
            AdmissionAuditError::NoDirectPropertyWitness)));
    assert_eq!(report.require_authenticated_historical_admin(),
        Err(ProjectedAdminError::HistoricalSourceAuthorityUnavailable));
}

#[test]
fn sender_roles_and_list_allocation_are_only_source_conditional() {
    let (tx, paths) = data();
    let mut h = [events(&tx,&paths,0,"adminA",Some(2)),
                 events(&tx,&paths,1,"adminA",None)];
    assert_eq!(check(2,&tx,&paths,&h),Err(ProjectedAdminError::WrongCreatedListId));
    h[0].expected_new_list_id=Some(1);
    h[1].claimed_first_input_sender="backupB";
    assert_eq!(check(2,&tx,&paths,&h),
        Err(ProjectedAdminError::UnverifiedSourceSenderNotCurrentAdmin));
    h[1].claimed_first_input_sender="adminA";
    h[0].claimed_type_active=false;
    assert_eq!(check(2,&tx,&paths,&h),
        Err(ProjectedAdminError::UntrustedTypeReportedInactive));
    h[0].claimed_type_active=true;
    h[1].expected_new_list_id=Some(2);
    assert_eq!(check(2,&tx,&paths,&h),Err(ProjectedAdminError::UnexpectedCreatedListId));
}

#[test]
fn wrong_admin_and_missing_previous_direct_member_are_refused() {
    let (tx, paths) = data();
    let mut h = [events(&tx,&paths,0,"adminA",Some(1)),
                 events(&tx,&paths,1,"adminA",None),
                 events(&tx,&paths,3,"adminA",None),
                 events(&tx,&paths,4,"adminA",None)];
    assert_eq!(check(5,&tx,&paths,&h),
        Err(ProjectedAdminError::UnverifiedSourceSenderNotCurrentAdmin));
    h[3].claimed_first_input_sender="adminC";
    h[2].claimed_first_input_sender="other";
    assert_eq!(check(5,&tx,&paths,&h),
        Err(ProjectedAdminError::UnverifiedSourceSenderNotAdminOrBackup));
    let missing = [h[0],events(&tx,&paths,3,"adminA",None),
                   events(&tx,&paths,4,"adminC",None)];
    assert_eq!(check(5,&tx,&paths,&missing),
        Err(ProjectedAdminError::DirectAttestationAlreadyRevoked));
}

#[test]
fn claimed_merkle_event_position_and_chain_tampering_refuse() {
    let (tx, paths) = data();
    let e0 = events(&tx,&paths,0,"adminA",Some(1));
    let e1 = events(&tx,&paths,1,"adminA",None);
    assert_eq!(check(2,&tx,&paths,&[e1,e0]),
        Err(ProjectedAdminError::InvalidEventOrder));
    assert_eq!(check(2,&tx,&paths,&[e0,e0]),
        Err(ProjectedAdminError::DuplicateObservedTransaction));
    let mut invalid=e1;
    invalid.at.block_hash[0]^=1;
    assert_eq!(check(2,&tx,&paths,&[e0,invalid]),
        Err(ProjectedAdminError::ClaimedPositionMismatch));
    let mut wrong=paths.clone();
    wrong[1].siblings_raw[0][0]^=1;
    let forged=[events(&tx,&wrong,0,"adminA",Some(1)),
                events(&tx,&wrong,1,"adminA",None)];
    assert!(matches!(check(2,&tx,&paths,&forged),
                     Err(ProjectedAdminError::RelativeInclusion(_))));
    for i in 0..7 {
        assert_eq!(inspect_transaction(&tx[i]).unwrap().txid,
                   hash(&f(&format!("tx{i}_id_raw"))));
    }
}
