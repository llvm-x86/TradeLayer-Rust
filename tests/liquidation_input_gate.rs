//! Stage 25G: current-block mark and channel *assertions*, never authorization.
use tradelayer_core::ledger::{AccountId, PropertyId, TxId};
use tradelayer_core::liquidation_input_gate::{
    audit_current_block_mark_unverified, audit_liquidation_claim_unverified, AssertedChannelUpdate,
    AssertedPriceRecord, AssertedPriceRoute, ContractMarkExpectation, ContractMarkSource,
    MissingIndependentProof, SourceGateError, ALL_MISSING_PROOFS, MAX_UNVERIFIED_CLEARLIST_IDS,
    MAX_UNVERIFIED_MARK_ROWS,
};
use tradelayer_core::network_profile::TradeLayerMainnet;

const ONE: u64 = 100_000_000;

fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn a(name: &str) -> AccountId {
    AccountId::try_from(name).unwrap()
}
fn tx(marker: u8) -> TxId {
    TxId::from_bytes([marker; 32])
}
fn expected_oracle() -> ContractMarkExpectation {
    ContractMarkExpectation {
        network: TradeLayerMainnet::Litecoin,
        contract_id: 19,
        collateral_property: p(7),
        source: ContractMarkSource::Oracle { oracle_id: 42 },
        target_height: 480,
        previous_mark: Some((479, 100 * ONE)),
    }
}
fn expected_native() -> ContractMarkExpectation {
    ContractMarkExpectation {
        source: ContractMarkSource::NativeVolumeIndex {
            notional_property: p(8),
        },
        ..expected_oracle()
    }
}
fn oracle_record() -> AssertedPriceRecord {
    AssertedPriceRecord {
        network: TradeLayerMainnet::Litecoin,
        contract_id: 19,
        height: 480,
        price_e8: 101 * ONE,
        route: AssertedPriceRoute::OraclePublication {
            oracle_id: 42,
            publication_txid: tx(5),
        },
    }
}
fn native_record() -> AssertedPriceRecord {
    AssertedPriceRecord {
        route: AssertedPriceRoute::NativeVolumeIndex {
            notional_property: p(8),
            collateral_property: p(7),
        },
        ..oracle_record()
    }
}
fn channel_claim() -> AssertedChannelUpdate {
    AssertedChannelUpdate {
        network: TradeLayerMainnet::Litecoin,
        contract_id: 19,
        update_height: 480,
        expires_at_height: 483,
        channel: a("channel2of2"),
        participant_a: a("alice"),
        participant_b: a("bob"),
        claimed_state_reference: [9; 32],
        declared_clearlist_ids: vec![4, 8],
    }
}
fn valid_mark() -> tradelayer_core::liquidation_input_gate::UnverifiedFreshMark {
    audit_current_block_mark_unverified(expected_oracle(), &[oracle_record()]).unwrap()
}

#[test]
fn structurally_correct_oracle_mark_is_explicitly_unverified() {
    let prior = oracle_record();
    let mut older = prior;
    older.height = 479;
    older.price_e8 = 100 * ONE;
    let selected = audit_current_block_mark_unverified(expected_oracle(), &[older, prior]).unwrap();
    assert_eq!(selected.height(), 480);
    assert_eq!(selected.price_e8(), 101 * ONE);
    assert_eq!(selected.contract_id(), 19);
    assert_eq!(selected.network(), TradeLayerMainnet::Litecoin);
    assert_eq!(selected.asserted_route(), prior.route);
    let claim = audit_liquidation_claim_unverified(selected, channel_claim()).unwrap();
    assert_eq!(claim.mark(), selected);
    assert_eq!(claim.channel().participant_a, a("alice"));
    assert_eq!(claim.missing_proofs(), &ALL_MISSING_PROOFS);
    assert_eq!(
        claim.require_authenticated_liquidation(),
        Err(SourceGateError::AuthenticationUnavailable)
    );
}

#[test]
fn native_current_block_mark_requires_matching_index_pair() {
    let s = audit_current_block_mark_unverified(expected_native(), &[native_record()]).unwrap();
    assert_eq!(s.price_e8(), 101 * ONE);
    let mut bad = native_record();
    bad.route = AssertedPriceRoute::NativeVolumeIndex {
        notional_property: p(8),
        collateral_property: p(9),
    };
    assert_eq!(
        audit_current_block_mark_unverified(expected_native(), &[bad]),
        Err(SourceGateError::WrongPriceRoute)
    );
    bad.route = AssertedPriceRoute::NativeVolumeIndex {
        notional_property: p(9),
        collateral_property: p(7),
    };
    assert_eq!(
        audit_current_block_mark_unverified(expected_native(), &[bad]),
        Err(SourceGateError::WrongPriceRoute)
    );
}

#[test]
fn funding_twap_or_vwap_can_never_impersonate_a_fresh_liquidation_mark() {
    let mut oracle_funding = oracle_record();
    oracle_funding.route = AssertedPriceRoute::OracleFundingTwap { oracle_id: 42 };
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[oracle_funding]),
        Err(SourceGateError::WrongPriceRoute)
    );
    let mut native_funding = native_record();
    native_funding.route = AssertedPriceRoute::NativeFundingVwap {
        notional_property: p(8),
        collateral_property: p(7),
    };
    assert_eq!(
        audit_current_block_mark_unverified(expected_native(), &[native_funding]),
        Err(SourceGateError::WrongPriceRoute)
    );
    // Cross-feed substitution is also prohibited.
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[native_record()]),
        Err(SourceGateError::WrongPriceRoute)
    );
    assert_eq!(
        audit_current_block_mark_unverified(expected_native(), &[oracle_record()]),
        Err(SourceGateError::WrongPriceRoute)
    );
}

#[test]
fn old_cached_price_does_not_create_a_current_block_event() {
    let mut row = oracle_record();
    row.height = 479;
    row.price_e8 = 101 * ONE;
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[row]),
        Err(SourceGateError::MissingCurrentBlockMark)
    );
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[]),
        Err(SourceGateError::MissingCurrentBlockMark)
    );
    // The pinned JS returned updated=false when the current mark equals
    // the last value. This must not start a new liquidation mark.
    row.height = 480;
    row.price_e8 = 100 * ONE;
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[row]),
        Err(SourceGateError::UnchangedMark)
    );
}

#[test]
fn duplicated_publications_are_not_selected_by_database_encounter_order() {
    let first = oracle_record();
    let mut second = first;
    second.price_e8 = 102 * ONE;
    second.route = AssertedPriceRoute::OraclePublication {
        oracle_id: 42,
        publication_txid: tx(6),
    };
    for docs in [[first, second], [second, first]] {
        assert_eq!(
            audit_current_block_mark_unverified(expected_oracle(), &docs),
            Err(SourceGateError::DuplicateCurrentBlockMark)
        );
    }
    // JS finds rows[0]; we explicitly refuse to choose consensus from
    // an ambiguous DB result without a reviewed ordering rule.
}

#[test]
fn source_identity_and_height_are_not_authorized_by_a_plausible_price() {
    let valid = oracle_record();
    let mut foreign = valid;
    foreign.network = TradeLayerMainnet::Dogecoin;
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[foreign]),
        Err(SourceGateError::WrongNetwork)
    );
    foreign = valid;
    foreign.contract_id = 20;
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[foreign]),
        Err(SourceGateError::WrongContract)
    );
    foreign = valid;
    foreign.height = 481;
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[foreign]),
        Err(SourceGateError::FutureRecord)
    );
    foreign = valid;
    foreign.price_e8 = 0;
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[foreign]),
        Err(SourceGateError::InvalidPrice)
    );
    foreign = valid;
    foreign.route = AssertedPriceRoute::OraclePublication {
        oracle_id: 43,
        publication_txid: tx(5),
    };
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[foreign]),
        Err(SourceGateError::WrongPriceRoute)
    );
    foreign = valid;
    foreign.route = AssertedPriceRoute::OraclePublication {
        oracle_id: 42,
        publication_txid: tx(0),
    };
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &[foreign]),
        Err(SourceGateError::InvalidOraclePublicationId)
    );
}

#[test]
fn invalid_preceding_epoch_contract_or_price_window_fails_closed() {
    let bad = ContractMarkExpectation {
        contract_id: 0,
        ..expected_oracle()
    };
    assert_eq!(
        audit_current_block_mark_unverified(bad, &[oracle_record()]),
        Err(SourceGateError::InvalidContractExpectation)
    );
    let bad = ContractMarkExpectation {
        target_height: 0,
        ..expected_oracle()
    };
    assert_eq!(
        audit_current_block_mark_unverified(bad, &[oracle_record()]),
        Err(SourceGateError::InvalidContractExpectation)
    );
    for old in [(480, 100 * ONE), (481, 100 * ONE), (479, 0)] {
        let bad = ContractMarkExpectation {
            previous_mark: Some(old),
            ..expected_oracle()
        };
        assert_eq!(
            audit_current_block_mark_unverified(bad, &[oracle_record()]),
            Err(SourceGateError::InvalidPreviousMark)
        );
    }
    let unbounded = vec![oracle_record(); MAX_UNVERIFIED_MARK_ROWS + 1];
    assert_eq!(
        audit_current_block_mark_unverified(expected_oracle(), &unbounded),
        Err(SourceGateError::TooManyPriceRecords)
    );
    let bad = ContractMarkExpectation {
        source: ContractMarkSource::Oracle { oracle_id: 0 },
        ..expected_oracle()
    };
    assert_eq!(
        audit_current_block_mark_unverified(bad, &[oracle_record()]),
        Err(SourceGateError::InvalidContractExpectation)
    );
}

#[test]
fn channel_ids_participants_expiry_and_state_refs_are_assertions_only() {
    let mark = valid_mark();
    let mut claim = channel_claim();
    claim.network = TradeLayerMainnet::Bitcoin;
    assert_eq!(
        audit_liquidation_claim_unverified(mark, claim),
        Err(SourceGateError::WrongNetwork)
    );
    claim = channel_claim();
    claim.contract_id = 18;
    assert_eq!(
        audit_liquidation_claim_unverified(mark, claim),
        Err(SourceGateError::WrongContract)
    );
    claim = channel_claim();
    claim.update_height = 479;
    assert_eq!(
        audit_liquidation_claim_unverified(mark, claim),
        Err(SourceGateError::WrongChannelHeight)
    );
    claim = channel_claim();
    claim.expires_at_height = 479;
    assert_eq!(
        audit_liquidation_claim_unverified(mark, claim),
        Err(SourceGateError::ExpiredChannelUpdate)
    );
    claim = channel_claim();
    claim.claimed_state_reference = [0; 32];
    assert_eq!(
        audit_liquidation_claim_unverified(mark, claim),
        Err(SourceGateError::MissingChannelStateReference)
    );
    for who in ["alice", "bob", "~clrm:spoof"] {
        claim = channel_claim();
        claim.channel = a(who);
        assert_eq!(
            audit_liquidation_claim_unverified(mark, claim),
            Err(SourceGateError::InvalidChannelIdentity)
        );
    }
    claim = channel_claim();
    claim.participant_a = claim.participant_b.clone();
    assert_eq!(
        audit_liquidation_claim_unverified(mark, claim),
        Err(SourceGateError::InvalidChannelIdentity)
    );
    claim = channel_claim();
    claim.participant_a = a("~ins:spoof");
    assert_eq!(
        audit_liquidation_claim_unverified(mark, claim),
        Err(SourceGateError::InvalidChannelIdentity)
    );
}

#[test]
fn declared_clearlist_ids_cannot_become_historical_attestation_proofs() {
    let mark = valid_mark();
    let mut claim = channel_claim();
    claim.declared_clearlist_ids = vec![5, 5];
    assert_eq!(
        audit_liquidation_claim_unverified(mark, claim),
        Err(SourceGateError::DuplicateClearlistAssertion)
    );
    claim = channel_claim();
    claim.declared_clearlist_ids =
        (1..=u32::try_from(MAX_UNVERIFIED_CLEARLIST_IDS).unwrap() + 1).collect();
    assert_eq!(
        audit_liquidation_claim_unverified(mark, claim),
        Err(SourceGateError::TooManyClearlistAssertions)
    );
    // Even if both parties are *claimed* to be members and a channel
    // reference is present, membership and both signatures remain
    // unverified. Type-4 token clearlist and type-19 at-least-one
    // channel clearlist have distinct historical conditions.
    claim = channel_claim();
    claim.declared_clearlist_ids = vec![3, 5, 8];
    let candidate = audit_liquidation_claim_unverified(mark, claim).unwrap();
    assert_eq!(
        candidate.require_authenticated_liquidation(),
        Err(SourceGateError::AuthenticationUnavailable)
    );
}

#[test]
fn all_required_independent_verifiers_remain_explicitly_missing() {
    let mark = audit_current_block_mark_unverified(expected_native(), &[native_record()]).unwrap();
    let claim = audit_liquidation_claim_unverified(mark, channel_claim()).unwrap();
    let gaps = claim.missing_proofs();
    assert_eq!(gaps.len(), 6);
    assert!(
        gaps.contains(&MissingIndependentProof::AuthenticatedNetworkBlockAndTransactionInclusion)
    );
    assert!(gaps.contains(&MissingIndependentProof::HistoricalOracleAdminOrNativeVolumeIndex));
    assert!(gaps.contains(&MissingIndependentProof::HistoricalCommitAndClearlistState));
    assert!(gaps.contains(&MissingIndependentProof::GenuineTwoOfTwoSpendOrSignedChannelUpdate));
    assert!(gaps.contains(&MissingIndependentProof::ExecutedAmmFirstOrMakerFill));
    assert!(
        gaps.contains(&MissingIndependentProof::RatifiedLiquidationRoundingWaterfallAndActivation)
    );
    assert_eq!(
        claim.require_authenticated_liquidation(),
        Err(SourceGateError::AuthenticationUnavailable)
    );
}
