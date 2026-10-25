//! File 5/49: source types.js dispatch inventory and permanent no-admission.
//! No verifier or real transaction source is connected to this test.
use tradelayer_core::legacy_types_audit::{
    source_generic_concat_unverified, source_inspect_decode_invocation,
    source_resolve_reference_address, source_route, source_type11_claimed_recipient,
    ClaimedReference, GenericEnvelopeKind, LegacyTypesError, SourceDecodeCaveat,
    SourceDecodeOutcome, SourceDecoderInvocation, LEGACY_TYPE_ROUTES,
};
use tradelayer_core::wire::{parse_typed, WireError};

fn full(type_id: u8, marker: &str, refs: bool) -> SourceDecoderInvocation<'_> {
    SourceDecoderInvocation::FullNineArguments {
        type_id,
        marker,
        type3_both_output_refs_claimed: refs,
    }
}

#[test]
fn all_36_exact_source_routes_match_pinned_fixture_and_missing_methods() {
    let rows = include_str!("../fixtures/legacy_types_routes.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("id\t"))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 36);
    assert_eq!(LEGACY_TYPE_ROUTES.len(), 36);
    for (index, row) in rows.iter().enumerate() {
        let f = row.split('\t').collect::<Vec<_>>();
        assert_eq!(f.len(), 7);
        assert_eq!(f[0].parse::<usize>().unwrap(), index);
        let route = source_route(index as u8).unwrap();
        assert_eq!(route.type_id, index as u8);
        assert_eq!(route.encoder, f[1]);
        assert_eq!(route.encoder_exported, f[2] == "1");
        assert_eq!(route.decoder, f[3]);
        assert_eq!(route.decoder_exported, f[4] == "1");
        assert_eq!(route.validator, (f[5] != "-").then_some(f[5]));
        assert_eq!(route.validator_invoked_by_source, f[6] == "1");
    }
    assert!(source_route(36).is_none());
    assert!(source_route(u8::MAX).is_none());
    assert!(!source_route(4).unwrap().encoder_exported);
    assert_eq!(source_route(4).unwrap().encoder, "encodeCommitToken");
    assert_eq!(
        source_route(15).unwrap().validator,
        Some("validatePublishOracleData")
    );
    assert_eq!(
        source_route(31).unwrap().validator,
        Some("validateKingSettle")
    );
    for ty in [28, 29, 34, 35] {
        assert!(!source_route(ty).unwrap().decoder_exported, "{ty}");
    }
    for ty in 32..=35 {
        assert!(
            !source_route(ty).unwrap().validator_invoked_by_source,
            "{ty}"
        );
    }
    assert!(source_route(32).unwrap().decoder_exported);
    assert!(source_route(33).unwrap().decoder_exported);
    assert_eq!(source_route(34).unwrap().encoder, "encodeAbstractionBridge");
    assert_eq!(source_route(34).unwrap().decoder, "decodeCrossLayerBridge");
}

#[test]
fn generic_outer_prefix_doubles_prefixed_type9_2_scalar_and_type15() {
    let inner = "tl90,7,alice,OK";
    let a = source_generic_concat_unverified(9, inner).unwrap();
    assert_eq!(a.claimed_ascii(), "tl9tl90,7,alice,OK");
    assert_eq!(a.envelope_kind(), GenericEnvelopeKind::DuplicatedTypePrefix);
    assert_eq!(a.type_id(), 9);
    assert_eq!(
        parse_typed(a.claimed_ascii()),
        Err(WireError::NestedEnvelope)
    );
    assert_eq!(
        a.require_encoded_transaction_authority(),
        Err(LegacyTypesError::TrustedTransactionExecutionForbidden)
    );
    let type2scalar = source_generic_concat_unverified(2, "tl20;alice;5;2").unwrap();
    assert_eq!(type2scalar.claimed_ascii(), "tl2tl20;alice;5;2");
    assert_eq!(
        type2scalar.envelope_kind(),
        GenericEnvelopeKind::DuplicatedTypePrefix
    );
    let type2send_all = source_generic_concat_unverified(2, "1;alice").unwrap();
    assert_eq!(type2send_all.claimed_ascii(), "tl21;alice");
    assert_eq!(
        type2send_all.envelope_kind(),
        GenericEnvelopeKind::SingleOuterPrefix
    );
    let close = source_generic_concat_unverified(15, "tlf").unwrap();
    assert_eq!(close.claimed_ascii(), "tlftlf");
    assert_eq!(
        close.envelope_kind(),
        GenericEnvelopeKind::DuplicatedTypePrefix
    );
    let source32 = source_generic_concat_unverified(32, "synthetic-rollup").unwrap();
    assert_eq!(source32.claimed_ascii(), "tlwsynthetic-rollup");
    assert_eq!(
        source32.envelope_kind(),
        GenericEnvelopeKind::SingleOuterPrefix
    );
    let foreign = source_generic_concat_unverified(9, "tl4other").unwrap();
    assert_eq!(
        foreign.envelope_kind(),
        GenericEnvelopeKind::ForeignInnerPrefix
    );
    assert_eq!(foreign.claimed_ascii(), "tl9tl4other");
    assert_eq!(
        source_generic_concat_unverified(4, "tl4,1"),
        Err(LegacyTypesError::MissingSourceEncoder(4))
    );
    assert_eq!(
        source_generic_concat_unverified(36, "whatever"),
        Err(LegacyTypesError::UnknownSourceType(36))
    );
    assert_eq!(
        source_generic_concat_unverified(9, "x\nbad"),
        Err(LegacyTypesError::ClaimNotPrintableAscii)
    );
    assert_eq!(
        source_generic_concat_unverified(9, &"x".repeat(4094)),
        Err(LegacyTypesError::ClaimedPayloadTooLarge)
    );
}

#[test]
fn index_three_argument_call_cannot_run_historical_decoder() {
    // The historical call is Types.decodePayload(txid, marker, payload).
    // Its second argument "tl" becomes the type and its third becomes marker.
    let parsed = source_inspect_decode_invocation(SourceDecoderInvocation::TxIndexThreeArguments {
        payload_after_marker: "90,7,alice,OK",
    });
    assert_eq!(parsed.type_id(), None);
    assert_eq!(
        parsed.outcome(),
        SourceDecodeOutcome::ReturnedErrorObjectInvalidMarker
    );
    assert_eq!(
        parsed.caveats(),
        &[SourceDecodeCaveat::TxIndexMisorderedCallSignature]
    );
    let special =
        source_inspect_decode_invocation(SourceDecoderInvocation::TxIndexThreeArguments {
            payload_after_marker: "tl",
        });
    assert_eq!(
        special.outcome(),
        SourceDecodeOutcome::ReturnedErrorObjectUnknownType
    );
    assert_eq!(
        special.require_historical_transaction_admission(),
        Err(LegacyTypesError::InputAndHistoricStateNotAuthenticated)
    );
}

#[test]
fn full_decode_source_call_never_grants_authority_or_validated_output() {
    let normal = source_inspect_decode_invocation(full(7, "tl", false));
    assert_eq!(
        normal.outcome(),
        SourceDecodeOutcome::WouldInvokeValidatorOnCallerClaims
    );
    assert_eq!(
        normal.require_historical_transaction_admission(),
        Err(LegacyTypesError::InputAndHistoricStateNotAuthenticated)
    );
    assert_eq!(
        normal.require_trusted_execution(),
        Err(LegacyTypesError::TrustedTransactionExecutionForbidden)
    );
    assert_eq!(
        normal.require_canonical_channel_revision(),
        Err(LegacyTypesError::CanonicalTwoSignerChannelStateAbsent)
    );
    assert_eq!(
        source_inspect_decode_invocation(full(7, "NOT_TL", false)).outcome(),
        SourceDecodeOutcome::ReturnedErrorObjectInvalidMarker
    );
    assert_eq!(
        source_inspect_decode_invocation(full(36, "tl", false)).outcome(),
        SourceDecodeOutcome::ReturnedErrorObjectUnknownType
    );
    for id in [0, 1, 2, 4, 5, 9, 11, 14, 19, 23, 27, 30, 31] {
        assert_eq!(
            source_inspect_decode_invocation(full(id, "tl", true)).outcome(),
            SourceDecodeOutcome::WouldInvokeValidatorOnCallerClaims,
            "{id}"
        );
    }
}

#[test]
fn missing_reference_type3_returns_unvalidated_decoded_params() {
    let missing = source_inspect_decode_invocation(full(3, "tl", false));
    assert_eq!(
        missing.outcome(),
        SourceDecodeOutcome::Type3UnvalidatedReturnMissingReferences
    );
    assert!(missing
        .caveats()
        .contains(&SourceDecodeCaveat::Type3ValidatorSkippedIfRefsMissing));
    let complete = source_inspect_decode_invocation(full(3, "tl", true));
    assert_eq!(
        complete.outcome(),
        SourceDecodeOutcome::WouldInvokeValidatorOnCallerClaims
    );
    assert_eq!(
        complete.require_trusted_execution(),
        Err(LegacyTypesError::TrustedTransactionExecutionForbidden)
    );
}

#[test]
fn close_oracle_calls_wrong_validator_on_number_and_loses_block() {
    let report = source_inspect_decode_invocation(full(15, "tl", true));
    assert_eq!(
        report.outcome(),
        SourceDecodeOutcome::Type15CallsWrongValidatorWithNumber
    );
    assert!(report
        .caveats()
        .contains(&SourceDecodeCaveat::Type15PublishOracleValidatorInsteadOfClose));
    assert!(report
        .caveats()
        .contains(&SourceDecodeCaveat::Type15DecoderReturnsNumberAndBlockAssignmentLost));
    assert_eq!(
        report.require_historical_transaction_admission(),
        Err(LegacyTypesError::InputAndHistoricStateNotAuthenticated)
    );
}

#[test]
fn missing_decoders_and_comment_only_validators_are_not_approval() {
    for ty in [28, 29, 34, 35] {
        let report = source_inspect_decode_invocation(full(ty, "tl", true));
        assert_eq!(
            report.outcome(),
            SourceDecodeOutcome::WouldThrowMissingDecoder,
            "{ty}"
        );
        assert_eq!(
            report.require_trusted_execution(),
            Err(LegacyTypesError::TrustedTransactionExecutionForbidden)
        );
    }
    for ty in [32, 33] {
        let report = source_inspect_decode_invocation(full(ty, "tl", true));
        assert_eq!(
            report.outcome(),
            SourceDecodeOutcome::DecodedWithoutAnyValidator
        );
        assert!(report
            .caveats()
            .contains(&SourceDecodeCaveat::Type32To35ValidatorCallsCommentedOut));
    }
    let settle = source_inspect_decode_invocation(full(23, "tl", true));
    assert!(settle
        .caveats()
        .contains(&SourceDecodeCaveat::Type23ChannelAddressIsInputSender));
    let king = source_inspect_decode_invocation(full(31, "tl", true));
    assert!(king
        .caveats()
        .contains(&SourceDecodeCaveat::Type31KingSettleNameVsOtherRegistries));
}

#[test]
fn type11_reference_string_array_object_precedence_matches_js_helper() {
    let row = [None, Some(""), Some("first"), Some("second")];
    assert_eq!(
        source_resolve_reference_address(ClaimedReference::Missing),
        ""
    );
    assert_eq!(
        source_resolve_reference_address(ClaimedReference::String("literal")),
        "literal"
    );
    assert_eq!(
        source_resolve_reference_address(ClaimedReference::Array(&row)),
        "first"
    );
    assert_eq!(
        source_resolve_reference_address(ClaimedReference::Object {
            address: Some("preferred"),
            sender_address: Some("backup")
        }),
        "preferred"
    );
    assert_eq!(
        source_resolve_reference_address(ClaimedReference::Object {
            address: Some(""),
            sender_address: Some("backup")
        }),
        "backup"
    );
    assert_eq!(
        source_resolve_reference_address(ClaimedReference::UnsupportedPrimitive),
        ""
    );
    assert_eq!(
        source_type11_claimed_recipient("", ClaimedReference::Array(&row)),
        "first"
    );
    assert_eq!(
        source_type11_claimed_recipient("direct", ClaimedReference::Array(&row)),
        "direct"
    );
}
