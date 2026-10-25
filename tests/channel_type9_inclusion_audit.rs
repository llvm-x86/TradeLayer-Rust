//! Stage25N Bitcoin relative type9 origin: synthetic Merkle+raw payload
//! necessary evidence only, with permanent historical admission denial.
use tradelayer_core::bitcoin_inclusion::{BitcoinPowProfile, CheckedHeaderSegment, MerklePath};
use tradelayer_core::chain::{BlockId, Tip};
use tradelayer_core::channel_admission_audit::{
    AdmissionAuditError, ClaimedDirectAction, ClaimedDirectAttestation, ClaimedEventPosition,
    HistoricalAdmissionKind, PropertyWhitelist, Type19ChannelListClaim, Type4CommitListClaim,
};
use tradelayer_core::channel_type9_inclusion_audit::{
    audit_bitcoin_type9_relative_history_unverified, inspect_raw_type9_candidate_unverified,
    ClaimedIncludedType9, Type9OriginError, MAX_RELATIVE_TYPE9_PROOFS,
};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn fixture(key: &str) -> String {
    include_str!("../fixtures/channel_type9_merkle_synthetic.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .find_map(|line| {
            let (name, value) = line.split_once('\t')?;
            (name == key).then_some(value.to_owned())
        })
        .unwrap_or_else(|| panic!("missing frozen Stage25N fixture {key}"))
}
fn exact32(hex: &str) -> [u8; 32] {
    decode_hex(hex).unwrap().try_into().unwrap()
}
fn raw(i: usize) -> Vec<u8> {
    decode_hex(&fixture(&format!("tx{i}_hex"))).unwrap()
}
fn path(i: usize) -> MerklePath {
    MerklePath {
        tx_index: u32::try_from(i).unwrap(),
        tx_count: 5,
        siblings_raw: fixture(&format!("tx{i}_merkle"))
            .split(',')
            .map(exact32)
            .collect(),
    }
}
fn segment() -> CheckedHeaderSegment {
    let header: [u8; 80] = decode_hex(&fixture("header_hex")).unwrap().try_into().unwrap();
    CheckedHeaderSegment::check_against_claimed_anchor(
        Tip {
            height: 100,
            id: BlockId([0x6a; 32]),
        },
        &[header],
        BitcoinPowProfile::RegtestLike,
    )
    .unwrap()
}
fn point(i: usize) -> ClaimedEventPosition {
    ClaimedEventPosition {
        height: 101,
        tx_index: u32::try_from(i).unwrap(),
        block_hash: exact32(&fixture("block_hash_display")),
    }
}
fn event<'a>(
    i: usize,
    raw: &'a [u8],
    path: &'a MerklePath,
    list: u32,
    address: &'a str,
    action: ClaimedDirectAction,
) -> ClaimedIncludedType9<'a> {
    ClaimedIncludedType9 {
        claim: ClaimedDirectAttestation {
            network: TradeLayerMainnet::Bitcoin,
            at: point(i),
            txid_raw: inspect_transaction(raw).unwrap().txid,
            clearlist_id: list,
            address,
            action,
        },
        raw_transaction: raw,
        merkle_path: path,
    }
}
fn sample_events<'a>(
    tx: &'a [Vec<u8>; 5],
    paths: &'a [MerklePath; 5],
) -> [ClaimedIncludedType9<'a>; 3] {
    [
        event(0, &tx[0], &paths[0], 7, "alice", ClaimedDirectAction::Attest),
        event(1, &tx[1], &paths[1], 11, "bob", ClaimedDirectAction::Attest),
        event(2, &tx[2], &paths[2], 7, "alice", ClaimedDirectAction::Revoke),
    ]
}
fn source_raws() -> [Vec<u8>; 5] {
    [raw(0), raw(1), raw(2), raw(3), raw(4)]
}
fn source_paths() -> [MerklePath; 5] {
    [path(0), path(1), path(2), path(3), path(4)]
}

#[test]
fn type9_merkle_origin_matches_exact_raw_txid_and_direct_claims_as_of_type4() {
    let tx = source_raws();
    let paths = source_paths();
    let events = sample_events(&tx, &paths);
    let report = audit_bitcoin_type9_relative_history_unverified(
        TradeLayerMainnet::Bitcoin,
        &segment(),
        point(3),
        &tx[3],
        &paths[3],
        HistoricalAdmissionKind::Type4Commit,
        &events,
    )
    .unwrap();

    assert_eq!(report.target().height(), 101);
    assert_eq!(report.target().tx_index(), 3);
    assert_eq!(report.included_event_count(), 3);
    assert_eq!(report.relative_events()[1].tx_index(), 1);
    assert_eq!(report.relative_events()[2].block_id().0, point(3).block_hash);
    for (i, bytes) in tx.iter().enumerate() {
        assert_eq!(
            inspect_transaction(bytes).unwrap().txid,
            exact32(&fixture(&format!("tx{i}_id_raw")))
        );
    }

    let bob = report
        .type4_direct_subset(Type4CommitListClaim {
            sender: "bob",
            property_id: 8,
            property_whitelist: PropertyWhitelist::AnyDirectList(&[11]),
            commit_clearlist_id: None,
        })
        .unwrap();
    assert_eq!(bob.matched_list_id(), Some(11));
    assert_eq!(bob.matched_commit_list_id(), None);
    assert_eq!(
        bob.require_authenticated_channel_admission(),
        Err(AdmissionAuditError::IndependentAdmissionProofUnavailable)
    );
    assert_eq!(
        report.type4_direct_subset(Type4CommitListClaim {
            sender: "alice",
            property_id: 8,
            property_whitelist: PropertyWhitelist::AnyDirectList(&[7]),
            commit_clearlist_id: None,
        }),
        Err(Type9OriginError::HistoricalClaim(
            AdmissionAuditError::NoDirectPropertyWitness
        ))
    );
    assert_eq!(
        report.require_authenticated_admission(),
        Err(Type9OriginError::OriginIsNotAuthorization)
    );
    assert_eq!(
        report.require_canonical_signed_revision(),
        Err(Type9OriginError::CanonicalRevisionUnspecified)
    );
}

#[test]
fn type19_either_side_semantics_still_deny_market_and_state_authority() {
    let tx = source_raws();
    let paths = source_paths();
    let events = sample_events(&tx, &paths);
    let report = audit_bitcoin_type9_relative_history_unverified(
        TradeLayerMainnet::Bitcoin,
        &segment(),
        point(4),
        &tx[4],
        &paths[4],
        HistoricalAdmissionKind::Type19ChannelTrade,
        &events,
    )
    .unwrap();
    let claim = Type19ChannelListClaim {
        claimed_sender_channel: "channelX",
        registered_channel: "channelX",
        participant_a: "alice",
        participant_b: "bob",
        column_a_lists: &[7],
        column_b_lists: &[11],
        expiry_height: 101,
    };
    let matched = report.type19_direct_subset(claim).unwrap();
    assert_eq!(matched.matched_list_id(), Some(11));
    assert_eq!(
        matched.require_canonical_signed_revision(),
        Err(AdmissionAuditError::CanonicalChannelRevisionUnspecified)
    );
    assert_eq!(
        report.type4_direct_subset(Type4CommitListClaim {
            sender: "bob",
            property_id: 8,
            property_whitelist: PropertyWhitelist::UnrestrictedScalarZero,
            commit_clearlist_id: None,
        }),
        Err(Type9OriginError::WrongAdmissionKind)
    );
    assert_eq!(
        report.require_authenticated_admission(),
        Err(Type9OriginError::OriginIsNotAuthorization)
    );
}

#[test]
fn forged_claim_fields_and_wrong_positions_reject_despite_valid_merkle_paths() {
    let tx = source_raws();
    let paths = source_paths();
    let events = sample_events(&tx, &paths);
    let run = |items: &[ClaimedIncludedType9<'_>]| {
        audit_bitcoin_type9_relative_history_unverified(
            TradeLayerMainnet::Bitcoin,
            &segment(),
            point(3),
            &tx[3],
            &paths[3],
            HistoricalAdmissionKind::Type4Commit,
            items,
        )
    };
    let mut bogus = events;
    bogus[1].claim.clearlist_id = 7;
    assert_eq!(run(&bogus), Err(Type9OriginError::EventSemanticMismatch));
    bogus = events;
    bogus[1].claim.address = "mallory";
    assert_eq!(run(&bogus), Err(Type9OriginError::EventSemanticMismatch));
    bogus = events;
    bogus[1].claim.action = ClaimedDirectAction::Revoke;
    assert_eq!(run(&bogus), Err(Type9OriginError::EventSemanticMismatch));
    bogus = events;
    bogus[1].claim.txid_raw[0] ^= 1;
    assert_eq!(run(&bogus), Err(Type9OriginError::ConflictingClaimedLocation));
    bogus = events;
    bogus[1].claim.at.tx_index = 4;
    assert_eq!(run(&bogus), Err(Type9OriginError::ConflictingClaimedLocation));
    bogus = events;
    bogus[1].claim.at.block_hash[0] ^= 1;
    assert_eq!(run(&bogus), Err(Type9OriginError::ConflictingClaimedLocation));
    bogus = events;
    bogus[1].claim.network = TradeLayerMainnet::Litecoin;
    assert_eq!(run(&bogus), Err(Type9OriginError::UnsupportedNetwork));
    assert_eq!(
        run(&[events[0], events[0]]),
        Err(Type9OriginError::DuplicateIncludedTransaction)
    );
    assert_eq!(
        run(&[events[2], events[0]]),
        Err(Type9OriginError::HistoricalClaim(
            AdmissionAuditError::OutOfOrderEvidence
        ))
    );
}

#[test]
fn invalid_merkle_forks_target_markers_and_other_networks_refuse() {
    let tx = source_raws();
    let mut paths = source_paths();
    paths[3].siblings_raw[0][0] ^= 1;
    let events = sample_events(&tx, &paths);
    assert!(matches!(
        audit_bitcoin_type9_relative_history_unverified(
            TradeLayerMainnet::Bitcoin,
            &segment(),
            point(3),
            &tx[3],
            &paths[3],
            HistoricalAdmissionKind::Type4Commit,
            &events,
        ),
        Err(Type9OriginError::HeaderOrMerkle(_))
    ));
    let paths = source_paths();
    let events = sample_events(&tx, &paths);
    assert_eq!(
        audit_bitcoin_type9_relative_history_unverified(
            TradeLayerMainnet::Bitcoin,
            &segment(),
            point(3),
            &tx[3],
            &paths[3],
            HistoricalAdmissionKind::Type19ChannelTrade,
            &events,
        ),
        Err(Type9OriginError::NotTargetType)
    );
    for network in [TradeLayerMainnet::Litecoin, TradeLayerMainnet::Dogecoin] {
        assert_eq!(
            audit_bitcoin_type9_relative_history_unverified(
                network,
                &segment(),
                point(3),
                &tx[3],
                &paths[3],
                HistoricalAdmissionKind::Type4Commit,
                &events,
            ),
            Err(Type9OriginError::UnsupportedNetwork)
        );
    }
    let too_many = vec![events[0]; MAX_RELATIVE_TYPE9_PROOFS + 1];
    assert_eq!(
        audit_bitcoin_type9_relative_history_unverified(
            TradeLayerMainnet::Bitcoin,
            &segment(),
            point(3),
            &tx[3],
            &paths[3],
            HistoricalAdmissionKind::Type4Commit,
            &too_many,
        ),
        Err(Type9OriginError::TooManyEvents)
    );
}

fn raw_with_opreturns(payloads: &[&str]) -> Vec<u8> {
    let mut tx = 2u32.to_le_bytes().to_vec();
    tx.push(1); // one fake input
    tx.extend_from_slice(&[0x11; 32]);
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx.push(0); // empty scriptSig
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.push(payloads.len() as u8);
    for payload in payloads {
        assert!(payload.len() <= 75);
        let mut script = vec![0x6a, payload.len() as u8];
        script.extend_from_slice(payload.as_bytes());
        tx.extend_from_slice(&0u64.to_le_bytes());
        tx.push(script.len() as u8);
        tx.extend_from_slice(&script);
    }
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx
}

#[test]
fn raw_type9_parser_rejects_nested_ambiguous_and_special_semantics() {
    let normal = inspect_raw_type9_candidate_unverified(
        &raw_with_opreturns(&["tl90,7,alice,OK"]),
    )
    .unwrap();
    assert_eq!(normal.clearlist_id(), 7);
    assert_eq!(normal.target(), "alice");
    assert_eq!(normal.metadata(), "OK");
    assert_eq!(normal.action(), ClaimedDirectAction::Attest);
    assert_eq!(
        normal.require_authorized_attestation(),
        Err(Type9OriginError::OriginIsNotAuthorization)
    );
    assert_eq!(
        inspect_raw_type9_candidate_unverified(
            &raw_with_opreturns(&["tl9tl90,7,alice,OK"])
        ),
        Err(Type9OriginError::AmbiguousNestedEnvelope)
    );
    assert_eq!(
        inspect_raw_type9_candidate_unverified(
            &raw_with_opreturns(&["tl90,7,alice,OK", "tl90,b,bob,OK"])
        ),
        Err(Type9OriginError::BadCarrier)
    );
    for payload in [
        "tl90,7,alice,",
        "tl90,7,alice,OK,EXTRA",
        "tl9X,7,alice,OK",
        "tl90,07,alice,OK",
        "tl90,7, alice ,OK",
    ] {
        assert!(
            inspect_raw_type9_candidate_unverified(&raw_with_opreturns(&[payload])).is_err(),
            "invalid raw type9 accepted: {payload}"
        );
    }
    for payload in [
        "tl90,0,alice,US",
        "tl90,7,BANLIST,US",
        "tl90,7,ref:1,OK",
        "tl90,7,alice,xpub:example",
    ] {
        assert_eq!(
            inspect_raw_type9_candidate_unverified(&raw_with_opreturns(&[payload])),
            Err(Type9OriginError::ExcludedDerivedOrCountryEvent)
        );
    }
}
