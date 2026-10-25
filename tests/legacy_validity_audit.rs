//! File 4/49: source-pinned validity.js Type0/7/8/9 observations.
//! All source successes are conditional on caller-claimed data; no adapter.
use tradelayer_core::legacy_validity_audit::{
    source_validate_type0_array_unverified, source_validate_type7_unverified,
    source_validate_type8_unverified, source_validate_type9_unverified,
    source_validity_is_activated_stub, ClaimedAddressValidation, ClaimedType7, ClaimedType8,
    ClaimedType8LookupRow, ClaimedType9, ClaimedType9List, SourceValidityCaveat,
    SourceValidityDisposition, SourceValidityError, SourceValidityFailure,
};

fn address(library: bool, camel: bool, lower: bool) -> ClaimedAddressValidation {
    ClaimedAddressValidation {
        validate_address_accepted: library,
        wrapper_is_valid: camel,
        wrapper_isvalid: lower,
    }
}

fn type7<'a>(
    active: bool,
    backup: Option<&'a str>,
    name: Option<&'a str>,
    addr: ClaimedAddressValidation,
) -> ClaimedType7<'a> {
    ClaimedType7 {
        tx_type_7_active: active,
        backup_address: backup,
        name,
        address_validation: addr,
    }
}
fn whitelist_row<'a>(
    admin: Option<&'a str>,
    backup: Option<&'a str>,
) -> ClaimedType8LookupRow<'a> {
    ClaimedType8LookupRow {
        attestations_admin_address: admin,
        attestations_backup_address: backup,
    }
}
fn type8<'a>(
    sender: &'a str,
    active: bool,
    row: Option<ClaimedType8LookupRow<'a>>,
    addr: ClaimedAddressValidation,
) -> ClaimedType8<'a> {
    ClaimedType8 {
        sender,
        tx_type_8_active: active,
        whitelist_flag: true,
        oracle_flag: false,
        token_flag: false,
        new_address: Some("adminC"),
        address_validation: addr,
        claimed_get_list_attestations_row: row,
    }
}
fn type9<'a>(
    sender: &'a str,
    active: bool,
    list_id: u32,
    target: &'a str,
    meta: &'a str,
) -> ClaimedType9<'a> {
    ClaimedType9 {
        sender,
        tx_type_9_active: active,
        list_id,
        target_address: target,
        metadata: meta,
        revoke: false,
        requested_ref_resolution: None,
        source_get_clearlist: Some(ClaimedType9List {
            admin_address: None,
            admin: Some("adminA"),
        }),
        claimed_direct_entry_active: true,
        claimed_protocol_admin: Some("rootAdmin"),
    }
}

#[test]
fn activation_stub_does_not_satisfy_separate_7_8_9_active_flags() {
    for ty in [0, 4, 7, 8, 9, 19, 31, 35] {
        assert!(source_validity_is_activated_stub(0, ty));
        assert!(source_validity_is_activated_stub(u64::MAX, ty));
    }
    let missing7 = source_validate_type7_unverified(
        type7(false, Some("backupB"), Some("ListOne"), address(true, false, false)),
    );
    assert_eq!(
        missing7.source_would_return_valid_if_claims_hold(),
        Some(false)
    );
    assert_eq!(missing7.failures(), &[SourceValidityFailure::TypeInactive(7)]);
    assert!(missing7.caveats().contains(
        &SourceValidityCaveat::ActivatedAtBlockStubAlwaysTrue
    ));
    let missing8 = source_validate_type8_unverified(type8(
        "adminA",
        false,
        Some(whitelist_row(Some("adminA"), None)),
        address(true, false, false),
    ));
    assert!(missing8.failures().contains(&SourceValidityFailure::TypeInactive(8)));
    let missing9 = source_validate_type9_unverified(type9(
        "adminA", false, 7, "alice", "OK",
    ));
    assert!(missing9.failures().contains(&SourceValidityFailure::TypeInactive(9)));
}

#[test]
fn type0_array_keeps_fractional_ids_and_later_activation_truncates_them() {
    let source = source_validate_type0_array_unverified(
        "rootAdmin",
        "rootAdmin",
        &[f64::NAN, 1.5, 0.0, 35.0],
    );
    assert_eq!(
        source.disposition(),
        SourceValidityDisposition::MatchedUnderUnverifiedClaims
    );
    assert_eq!(source.source_would_return_valid_if_claims_hold(), Some(true));
    assert_eq!(source.type0_sanitized_numbers(), &[1.5, 0.0, 35.0]);
    assert!(source.caveats().contains(
        &SourceValidityCaveat::Type0FractionalAcceptedThenIntTruncated
    ));
    // JS Activation.activate later calls parseInt; this is not a new
    // consensus rule and no native-token type0 genesis is executed here.
    assert_eq!(1.5_f64.trunc() as u8, 1);
    assert_eq!(
        source.require_authenticated_transaction(),
        Err(SourceValidityError::HistoricalInputAndStateUnverified)
    );
    assert_eq!(
        source.require_authorized_channel_settlement(),
        Err(SourceValidityError::CanonicalSignedRevisionUnavailable)
    );
}

#[test]
fn type0_array_invalid_short_circuits_wrong_admin_and_out_of_bounds() {
    let empty = source_validate_type0_array_unverified(
        "other", "rootAdmin", &[f64::NAN],
    );
    assert_eq!(empty.failures(), &[SourceValidityFailure::InvalidActivationTypes]);
    let wrong = source_validate_type0_array_unverified(
        "other", "rootAdmin", &[255.0],
    );
    assert_eq!(wrong.failures(), &[SourceValidityFailure::WrongProtocolAdmin]);
    let bad = source_validate_type0_array_unverified(
        "rootAdmin", "rootAdmin", &[0.0, 36.0, -1.0],
    );
    assert_eq!(bad.failures(), &[SourceValidityFailure::ActivationTypeOutOfBounds]);
    let inf = source_validate_type0_array_unverified(
        "rootAdmin", "rootAdmin", &[f64::INFINITY],
    );
    assert_eq!(inf.failures(), &[SourceValidityFailure::ActivationTypeOutOfBounds]);
    let none = source_validate_type0_array_unverified(
        "rootAdmin", "rootAdmin", &[],
    );
    assert_eq!(none.failures(), &[SourceValidityFailure::InvalidActivationTypes]);
}

#[test]
fn type7_accepts_camel_case_wrapper_but_type8_only_lowercase() {
    let camel = address(false, true, false);
    let seven = source_validate_type7_unverified(type7(
        true, Some("backupB"), Some(""), camel,
    ));
    assert_eq!(seven.source_would_return_valid_if_claims_hold(), Some(true));
    assert!(seven.failures().is_empty()); // source typeof name; empty passes
    let eight = source_validate_type8_unverified(type8(
        "adminA", true, Some(whitelist_row(Some("adminA"), None)), camel,
    ));
    assert_eq!(eight.source_would_return_valid_if_claims_hold(), Some(false));
    assert_eq!(eight.failures(), &[SourceValidityFailure::InvalidNewAddress]);
    assert!(eight.caveats().contains(
        &SourceValidityCaveat::Type8CamelCaseAddressFlagIgnored
    ));
    let lower = address(false, false, true);
    assert_eq!(
        source_validate_type7_unverified(type7(
            true, Some("backupB"), Some("x"), lower,
        )).source_would_return_valid_if_claims_hold(),
        Some(true)
    );
    assert_eq!(
        source_validate_type8_unverified(type8(
            "adminA", true, Some(whitelist_row(Some("adminA"), None)), lower,
        )).source_would_return_valid_if_claims_hold(),
        Some(true)
    );
    assert_eq!(
        source_validate_type8_unverified(type8(
            "adminA", true, Some(whitelist_row(Some("adminA"), None)),
            address(true, false, false),
        )).source_would_return_valid_if_claims_hold(),
        Some(true)
    );
}

#[test]
fn type7_backup_and_name_types_are_separate_conditions() {
    let report = source_validate_type7_unverified(type7(
        true, Some(""), None, address(false, false, false),
    ));
    assert!(report.failures().contains(&SourceValidityFailure::MissingBackupAddress));
    assert!(report.failures().contains(&SourceValidityFailure::InvalidBackupAddress));
    assert!(report.failures().contains(&SourceValidityFailure::InvalidNameType));
    assert_eq!(
        source_validate_type7_unverified(type7(
            true, None, Some("List"), address(true, false, false),
        )).source_would_return_valid_if_claims_hold(),
        Some(false)
    );
}

#[test]
fn type8_looks_up_numeric_key_in_attestations_not_created_clearlists() {
    let missing = source_validate_type8_unverified(type8(
        "adminA", true, None, address(true, false, false),
    ));
    assert_eq!(
        missing.failures(),
        &[SourceValidityFailure::Type8WhitelistLookupMissing]
    );
    assert!(missing.caveats().contains(
        &SourceValidityCaveat::Type8ReadsAttestationsCollection
    ));
    let legacy_fields = source_validate_type8_unverified(type8(
        "adminA", true, Some(whitelist_row(None, None)),
        address(true, false, false),
    ));
    assert_eq!(
        legacy_fields.failures(),
        &[SourceValidityFailure::Type8SenderNotAdminOrBackup]
    );
    // Even a caller-supplied matching row from the WRONG storage
    // collection cannot prove this was historically accepted.
    let imaginary = source_validate_type8_unverified(type8(
        "backupB", true,
        Some(whitelist_row(Some("adminA"), Some("backupB"))),
        address(true, false, false),
    ));
    assert_eq!(imaginary.source_would_return_valid_if_claims_hold(), Some(true));
    assert_eq!(
        imaginary.require_accepted_historical_administration(),
        Err(SourceValidityError::HistoricalType8AdminStateUnverified)
    );
}

#[test]
fn type8_exactly_one_target_and_unsupported_oracle_token_are_unmodeled() {
    let mut none = type8(
        "adminA", true, Some(whitelist_row(Some("adminA"), None)),
        address(true, false, false),
    );
    none.whitelist_flag = false;
    let no_target = source_validate_type8_unverified(none);
    assert_eq!(no_target.failures(), &[SourceValidityFailure::ExactlyOneAdminTarget]);
    assert_eq!(no_target.source_would_return_valid_if_claims_hold(), Some(false));
    let mut multi = type8(
        "adminA", true, Some(whitelist_row(Some("adminA"), None)),
        address(true, false, false),
    );
    multi.token_flag = true;
    let both = source_validate_type8_unverified(multi);
    assert!(both.failures().contains(&SourceValidityFailure::ExactlyOneAdminTarget));
    let mut oracle = none;
    oracle.oracle_flag = true;
    let unsupported = source_validate_type8_unverified(oracle);
    assert_eq!(
        unsupported.disposition(),
        SourceValidityDisposition::UnmodeledSourceContext
    );
    assert_eq!(unsupported.source_would_return_valid_if_claims_hold(), None);
    assert!(unsupported.caveats().contains(
        &SourceValidityCaveat::OracleOrTokenAdminTargetUnmodeled
    ));
}

#[test]
fn type9_positive_list_checks_primary_only_and_active_direct_revocation() {
    let valid = source_validate_type9_unverified(type9(
        "adminA", true, 7, "alice", "anything",
    ));
    assert_eq!(valid.source_would_return_valid_if_claims_hold(), Some(true));
    let mut backup = type9("backupB", true, 7, "alice", "meta");
    backup.source_get_clearlist = Some(ClaimedType9List {
        admin_address: Some("adminA"),
        admin: Some("backupB"),
    });
    assert_eq!(
        source_validate_type9_unverified(backup).failures(),
        &[SourceValidityFailure::Type9SenderNotPrimaryAdmin]
    );
    let mut missing = type9("adminA", true, 7, "alice", "x");
    missing.source_get_clearlist = None;
    assert_eq!(
        source_validate_type9_unverified(missing).failures(),
        &[SourceValidityFailure::PositiveListNotFound]
    );
    let mut revoke = type9("adminA", true, 7, "alice", "reason");
    revoke.revoke = true;
    revoke.claimed_direct_entry_active = false;
    assert_eq!(
        source_validate_type9_unverified(revoke).failures(),
        &[SourceValidityFailure::RevokeWithoutActiveDirectAttestation]
    );
    revoke.claimed_direct_entry_active = true;
    assert_eq!(
        source_validate_type9_unverified(revoke).source_would_return_valid_if_claims_hold(),
        Some(true)
    );
    assert_eq!(
        valid.require_authenticated_transaction(),
        Err(SourceValidityError::HistoricalInputAndStateUnverified)
    );
}

#[test]
fn type9_reference_override_is_required_when_ref_is_present() {
    let mut claim = type9("adminA", true, 7, "", "x");
    claim.requested_ref_resolution = Some(None);
    let missing = source_validate_type9_unverified(claim);
    assert!(missing.failures().contains(
        &SourceValidityFailure::RequestedReferenceUnresolved
    ));
    assert!(missing.failures().contains(
        &SourceValidityFailure::MissingTargetAddress
    ));
    claim.requested_ref_resolution = Some(Some("alice"));
    let resolved = source_validate_type9_unverified(claim);
    assert_eq!(resolved.normalized_target(), Some("alice"));
    assert_eq!(resolved.source_would_return_valid_if_claims_hold(), Some(true));
    assert!(resolved.caveats().contains(
        &SourceValidityCaveat::ReferenceOutputIsCallerClaimed
    ));
}

#[test]
fn type9_list_zero_country_self_cert_requires_same_sender_and_two_letters() {
    let mut own = type9("alice", true, 0, "alice", " us ");
    own.source_get_clearlist = None;
    assert_eq!(
        source_validate_type9_unverified(own).source_would_return_valid_if_claims_hold(),
        Some(true)
    );
    own.target_address = "bob";
    assert_eq!(
        source_validate_type9_unverified(own).failures(),
        &[SourceValidityFailure::CountrySelfCertRequiresSenderTarget]
    );
    own.target_address = "alice";
    own.metadata = "USA";
    assert_eq!(
        source_validate_type9_unverified(own).failures(),
        &[SourceValidityFailure::CountrySelfCertCodeInvalid]
    );
    own.metadata = "u9";
    assert_eq!(
        source_validate_type9_unverified(own).failures(),
        &[SourceValidityFailure::CountrySelfCertCodeInvalid]
    );
    own.metadata = "NZ";
    own.revoke = true;
    own.claimed_direct_entry_active = false;
    assert_eq!(
        source_validate_type9_unverified(own).failures(),
        &[SourceValidityFailure::RevokeWithoutActiveDirectAttestation]
    );
}

#[test]
fn type9_list_zero_banlist_uses_protocol_admin_and_split_country_codes() {
    let mut ban = type9("rootAdmin", true, 0, " banlist ", "us;KP | nz, gb");
    ban.source_get_clearlist = None;
    let accepted = source_validate_type9_unverified(ban);
    assert_eq!(accepted.source_would_return_valid_if_claims_hold(), Some(true));
    assert_eq!(
        accepted.parsed_banlist_codes(),
        &["US", "KP", "NZ", "GB"]
    );
    ban.sender = "bob";
    let denied = source_validate_type9_unverified(ban);
    assert!(denied.failures().contains(
        &SourceValidityFailure::BanlistProtocolAdminRequired
    ));
    ban.sender = "rootAdmin";
    ban.metadata = "US;USA";
    let bad = source_validate_type9_unverified(ban);
    assert!(bad.failures().contains(
        &SourceValidityFailure::BanlistCountryCodesInvalid
    ));
    ban.metadata = "";
    assert!(source_validate_type9_unverified(ban).failures().contains(
        &SourceValidityFailure::BanlistCountryCodesInvalid
    ));
    assert_eq!(
        accepted.require_accepted_historical_administration(),
        Err(SourceValidityError::HistoricalType8AdminStateUnverified)
    );
    assert_eq!(
        accepted.require_authorized_channel_settlement(),
        Err(SourceValidityError::CanonicalSignedRevisionUnavailable)
    );
}
