//! File 4/49: pinned legacy validity.js read-only Type0/7/8/9 predicates.
//!
//! Source: tradelayer.js@dca0888b, src/validity.js:83-94,113-162,
//! 883-921,921-1001,1003-1099, and clearlist.js:65-98,112-130.
//! Only a carefully bounded source-claim model, not valid consensus input.
//!
//! Every dependency (sender, registry activation, address-wrapper booleans,
//! clearlist/admin row, output ref and current direct membership) is an
//! externally CLAIMED state; no Bitcoin/Litecoin/Dogecoin script/UTXO, signed
//! transaction, full node, activation history, Type8 DB registry state or
//! canonical off-chain channel revision is verified in this module.
//!
//! Distinct historical source behaviors deliberately modeled:
//! - Validity.isActivated returns TRUE unconditionally even for an early tx,
//!   separately from activationInstance.isTxTypeActive(7/8/9).
//! - Type7 address fallback accepts wrapper.isValid OR wrapper.isvalid,
//!   whereas Type8 accepts ONLY lowercase wrapper.isvalid.
//! - Type8 whitelist check uses ClearList.getList(id), whose source
//!   implementation reads ATTESTATIONS (not the CLEArLISTS collection).
//! - Type9 checks ONLY primary admin for positive list ID; list zero uses
//!   sender=self for country self-cert, or PROTOCOL admin for BANLIST.
//!   Any requested ref must resolve against actual vout for acceptance.
//! - Type0 filters array items after Number conversion using isNaN; it
//!   allows noninteger fractional IDs 0..35, although Activation.activate
//!   later parseInt-truncates the transaction type: a source discrepancy.
//!
//! MatchedUnderClaims means only that the reproduced narrow source predicate
//! evaluates true on user-supplied context. All authority methods fail.
//! Existing stricter Rust Registry, Ledger, Clearing and source diagnostics
//! are unchanged. No trusted adapter can be constructed from this report.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceValidityKind {
    Type0Activate,
    Type7CreateWhitelist,
    Type8UpdateWhitelistAdmin,
    Type9Attestation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceValidityDisposition {
    RejectedBySourcePredicate,
    MatchedUnderUnverifiedClaims,
    UnmodeledSourceContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceValidityFailure {
    InvalidActivationTypes,
    WrongProtocolAdmin,
    ActivationTypeOutOfBounds,
    TypeInactive(u8),
    MissingBackupAddress,
    InvalidBackupAddress,
    InvalidNameType,
    InvalidNewAddressType,
    InvalidNewAddress,
    ExactlyOneAdminTarget,
    Type8WhitelistLookupMissing,
    Type8SenderNotAdminOrBackup,
    MissingTargetAddress,
    RequestedReferenceUnresolved,
    PositiveListNotFound,
    Type9SenderNotPrimaryAdmin,
    BanlistProtocolAdminRequired,
    BanlistCountryCodesInvalid,
    CountrySelfCertRequiresSenderTarget,
    CountrySelfCertCodeInvalid,
    RevokeWithoutActiveDirectAttestation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceValidityCaveat {
    /// Source Validity.isActivated ignores block/txid and returns true.
    ActivatedAtBlockStubAlwaysTrue,
    Type8ReadsAttestationsCollection,
    Type8CamelCaseAddressFlagIgnored,
    Type0FractionalAcceptedThenIntTruncated,
    ReferenceOutputIsCallerClaimed,
    OracleOrTokenAdminTargetUnmodeled,
    ClearlistAndActorAreCallerClaimed,
    CountryAndBanlistStateUnverified,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnverifiedValidityObservation {
    kind: SourceValidityKind,
    failures: Vec<SourceValidityFailure>,
    caveats: Vec<SourceValidityCaveat>,
    disposition: SourceValidityDisposition,
    normalized_target: Option<String>,
    banlist_country_codes: Vec<String>,
    type0_sanitized_numbers: Vec<f64>,
}

impl UnverifiedValidityObservation {
    fn new(kind: SourceValidityKind) -> Self {
        Self {
            kind,
            failures: Vec::new(),
            caveats: Vec::new(),
            disposition: SourceValidityDisposition::MatchedUnderUnverifiedClaims,
            normalized_target: None,
            banlist_country_codes: Vec::new(),
            type0_sanitized_numbers: Vec::new(),
        }
    }

    fn finish(mut self, unresolved: bool) -> Self {
        self.disposition = if !self.failures.is_empty() {
            SourceValidityDisposition::RejectedBySourcePredicate
        } else if unresolved {
            SourceValidityDisposition::UnmodeledSourceContext
        } else {
            SourceValidityDisposition::MatchedUnderUnverifiedClaims
        };
        self
    }
    pub const fn kind(&self) -> SourceValidityKind {
        self.kind
    }
    pub const fn disposition(&self) -> SourceValidityDisposition {
        self.disposition
    }
    pub fn source_would_return_valid_if_claims_hold(&self) -> Option<bool> {
        match self.disposition {
            SourceValidityDisposition::RejectedBySourcePredicate => Some(false),
            SourceValidityDisposition::MatchedUnderUnverifiedClaims => Some(true),
            SourceValidityDisposition::UnmodeledSourceContext => None,
        }
    }
    pub fn failures(&self) -> &[SourceValidityFailure] {
        &self.failures
    }
    pub fn caveats(&self) -> &[SourceValidityCaveat] {
        &self.caveats
    }
    pub fn normalized_target(&self) -> Option<&str> {
        self.normalized_target.as_deref()
    }
    pub fn parsed_banlist_codes(&self) -> &[String] {
        &self.banlist_country_codes
    }
    pub fn type0_sanitized_numbers(&self) -> &[f64] {
        &self.type0_sanitized_numbers
    }
    pub const fn require_authenticated_transaction(&self) -> Result<(), SourceValidityError> {
        Err(SourceValidityError::HistoricalInputAndStateUnverified)
    }
    pub const fn require_accepted_historical_administration(
        &self,
    ) -> Result<(), SourceValidityError> {
        Err(SourceValidityError::HistoricalType8AdminStateUnverified)
    }
    pub const fn require_authorized_channel_settlement(&self) -> Result<(), SourceValidityError> {
        Err(SourceValidityError::CanonicalSignedRevisionUnavailable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceValidityError {
    HistoricalInputAndStateUnverified,
    HistoricalType8AdminStateUnverified,
    CanonicalSignedRevisionUnavailable,
}

/// Historical Validity.isActivated is an unconditional stub, and MUST NOT
/// be replaced with a real height or proof check in source characterization.
/// This is not an authorization method.
pub const fn source_validity_is_activated_stub(
    _height: u64,
    _transaction_type: u8,
) -> bool {
    true
}

/// Type0: only the ARRAY branch with already Number-coerced values is
/// modeled. A heterogeneous JS array with objects, nulls, strings, or
/// scalar fallback behavior requires a separate versioned coercion audit.
/// Source Number(x) + !isNaN retains fractional finite values: these pass
/// the 0..35 bounds, then Activation.activate uses parseInt and truncates.
pub fn source_validate_type0_array_unverified(
    sender: &str,
    claimed_protocol_admin: &str,
    already_coerced_numbers: &[f64],
) -> UnverifiedValidityObservation {
    let mut result = UnverifiedValidityObservation::new(SourceValidityKind::Type0Activate);
    result.type0_sanitized_numbers = already_coerced_numbers
        .iter()
        .copied()
        .filter(|value| !value.is_nan())
        .collect();
    if result.type0_sanitized_numbers.is_empty() {
        result.failures.push(SourceValidityFailure::InvalidActivationTypes);
        return result.finish(false);
    }
    if sender != claimed_protocol_admin {
        result.failures.push(SourceValidityFailure::WrongProtocolAdmin);
        return result.finish(false);
    }
    if result
        .type0_sanitized_numbers
        .iter()
        .any(|number| *number > 35.0 || *number < 0.0)
    {
        result.failures.push(SourceValidityFailure::ActivationTypeOutOfBounds);
    }
    if result
        .type0_sanitized_numbers
        .iter()
        .any(|number| number.is_finite() && number.fract() != 0.0)
    {
        result.caveats.push(SourceValidityCaveat::Type0FractionalAcceptedThenIntTruncated);
    }
    result.finish(false)
}

/// External, NOT independently observed results from the two separate
/// address verification functions used by the pinned validity.js source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimedAddressValidation {
    pub validate_address_accepted: bool,
    pub wrapper_is_valid: bool,
    pub wrapper_isvalid: bool,
}

impl ClaimedAddressValidation {
    fn type7_accepted(self) -> bool {
        self.validate_address_accepted || self.wrapper_is_valid || self.wrapper_isvalid
    }
    fn type8_accepted(self) -> bool {
        self.validate_address_accepted || self.wrapper_isvalid
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedType7<'a> {
    pub tx_type_7_active: bool,
    pub backup_address: Option<&'a str>,
    pub name: Option<&'a str>,
    pub address_validation: ClaimedAddressValidation,
}

/// Source names can be EMPTY strings (typeof "" === "string") while
/// backup must be truthy, string and source address-validated.
pub fn source_validate_type7_unverified(
    claim: ClaimedType7<'_>,
) -> UnverifiedValidityObservation {
    let mut o = UnverifiedValidityObservation::new(SourceValidityKind::Type7CreateWhitelist);
    o.caveats.push(SourceValidityCaveat::ActivatedAtBlockStubAlwaysTrue);
    if !claim.tx_type_7_active {
        o.failures.push(SourceValidityFailure::TypeInactive(7));
    }
    if claim.backup_address.is_none_or(str::is_empty) {
        o.failures.push(SourceValidityFailure::MissingBackupAddress);
    }
    if !claim.address_validation.type7_accepted() {
        o.failures.push(SourceValidityFailure::InvalidBackupAddress);
    }
    if claim.name.is_none() {
        o.failures.push(SourceValidityFailure::InvalidNameType);
    }
    o.finish(false)
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedType8LookupRow<'a> {
    /// Returned by ClearList.getList: ATTESTATIONS collection by numeric ID,
    /// not the regular clearlists collection. No row is authenticated.
    pub attestations_admin_address: Option<&'a str>,
    pub attestations_backup_address: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedType8<'a> {
    pub sender: &'a str,
    pub tx_type_8_active: bool,
    pub whitelist_flag: bool,
    pub oracle_flag: bool,
    pub token_flag: bool,
    pub new_address: Option<&'a str>,
    pub address_validation: ClaimedAddressValidation,
    pub claimed_get_list_attestations_row: Option<ClaimedType8LookupRow<'a>>,
}

pub fn source_validate_type8_unverified(
    claim: ClaimedType8<'_>,
) -> UnverifiedValidityObservation {
    let mut o = UnverifiedValidityObservation::new(SourceValidityKind::Type8UpdateWhitelistAdmin);
    o.caveats.push(SourceValidityCaveat::ActivatedAtBlockStubAlwaysTrue);
    o.caveats.push(SourceValidityCaveat::Type8ReadsAttestationsCollection);
    o.caveats.push(SourceValidityCaveat::Type8CamelCaseAddressFlagIgnored);
    if !claim.tx_type_8_active {
        o.failures.push(SourceValidityFailure::TypeInactive(8));
    }
    if claim.new_address.is_none() {
        o.failures.push(SourceValidityFailure::InvalidNewAddressType);
    }
    let targets = u8::from(claim.whitelist_flag)
        + u8::from(claim.oracle_flag)
        + u8::from(claim.token_flag);
    if targets != 1 {
        o.failures.push(SourceValidityFailure::ExactlyOneAdminTarget);
    }
    if claim.whitelist_flag {
        match claim.claimed_get_list_attestations_row {
            None => o.failures.push(SourceValidityFailure::Type8WhitelistLookupMissing),
            Some(row) => {
                if row.attestations_admin_address != Some(claim.sender)
                    && row.attestations_backup_address != Some(claim.sender)
                {
                    o.failures.push(SourceValidityFailure::Type8SenderNotAdminOrBackup);
                }
            }
        }
    }
    if !claim.address_validation.type8_accepted() {
        o.failures.push(SourceValidityFailure::InvalidNewAddress);
    }
    if claim.oracle_flag || claim.token_flag {
        o.caveats.push(SourceValidityCaveat::OracleOrTokenAdminTargetUnmodeled);
    }
    o.finish(claim.oracle_flag || claim.token_flag)
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedType9List<'a> {
    /// getClearlistById reads from clearlists and prefers adminAddress
    /// over admin, unlike Type8's getList (attestations collection).
    pub admin_address: Option<&'a str>,
    pub admin: Option<&'a str>,
}

/// Option::Some(None) means ref was PRESENT but couldn't be resolved.
/// Option::None means the ref property was absent/false and source skipped
/// output resolution. Some(Some(address)) is a claimed matching output.
#[derive(Debug, Clone, Copy)]
pub struct ClaimedType9<'a> {
    pub sender: &'a str,
    pub tx_type_9_active: bool,
    pub list_id: u32,
    pub target_address: &'a str,
    pub metadata: &'a str,
    pub revoke: bool,
    pub requested_ref_resolution: Option<Option<&'a str>>,
    pub source_get_clearlist: Option<ClaimedType9List<'a>>,
    pub claimed_direct_entry_active: bool,
    pub claimed_protocol_admin: Option<&'a str>,
}

fn source_country_list(metadata: &str) -> Vec<String> {
    metadata
        .split(|c: char| c == ',' || c == ';' || c == '|' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_uppercase)
        .collect()
}
fn ascii_country_code(raw: &str) -> bool {
    raw.len() == 2 && raw.as_bytes().iter().all(u8::is_ascii_alphabetic)
}
fn ban_country_code(raw: &str) -> bool {
    raw.len() == 2 && raw.as_bytes().iter().all(u8::is_ascii_uppercase)
}

/// This models only string-valued targets and metadata. JavaScript numeric
/// coercion, object metadata, xpub or derived-signer rules are not asserted.
/// A source-matching positive row is only a caller's claim.
pub fn source_validate_type9_unverified(
    claim: ClaimedType9<'_>,
) -> UnverifiedValidityObservation {
    let mut o = UnverifiedValidityObservation::new(SourceValidityKind::Type9Attestation);
    o.caveats.push(SourceValidityCaveat::ActivatedAtBlockStubAlwaysTrue);
    o.caveats.push(SourceValidityCaveat::ClearlistAndActorAreCallerClaimed);
    if !claim.tx_type_9_active {
        o.failures.push(SourceValidityFailure::TypeInactive(9));
    }
    let mut target = claim.target_address;
    if let Some(resolution) = claim.requested_ref_resolution {
        o.caveats.push(SourceValidityCaveat::ReferenceOutputIsCallerClaimed);
        match resolution {
            Some(resolved) if !resolved.is_empty() => target = resolved,
            _ => o.failures.push(SourceValidityFailure::RequestedReferenceUnresolved),
        }
    }
    if target.is_empty() {
        o.failures.push(SourceValidityFailure::MissingTargetAddress);
    }
    o.normalized_target = Some(target.to_owned());

    if claim.list_id != 0 {
        match claim.source_get_clearlist {
            None => o.failures.push(SourceValidityFailure::PositiveListNotFound),
            Some(info) => {
                let admin = info.admin_address.filter(|s| !s.is_empty()).or(info.admin);
                if admin != Some(claim.sender) {
                    o.failures.push(SourceValidityFailure::Type9SenderNotPrimaryAdmin);
                }
            }
        }
    } else {
        o.caveats.push(SourceValidityCaveat::CountryAndBanlistStateUnverified);
        if target.trim().eq_ignore_ascii_case("BANLIST") {
            if claim.claimed_protocol_admin != Some(claim.sender) {
                o.failures.push(SourceValidityFailure::BanlistProtocolAdminRequired);
            }
            o.banlist_country_codes = source_country_list(claim.metadata);
            if o.banlist_country_codes.is_empty()
                || o.banlist_country_codes.iter().any(|code| !ban_country_code(code))
            {
                o.failures.push(SourceValidityFailure::BanlistCountryCodesInvalid);
            }
        } else {
            if claim.sender != target {
                o.failures.push(SourceValidityFailure::CountrySelfCertRequiresSenderTarget);
            }
            if !ascii_country_code(claim.metadata.trim()) {
                o.failures.push(SourceValidityFailure::CountrySelfCertCodeInvalid);
            }
        }
    }
    // The source independently runs this check even in the list-zero paths.
    if claim.revoke && !claim.claimed_direct_entry_active {
        o.failures.push(SourceValidityFailure::RevokeWithoutActiveDirectAttestation);
    }
    o.finish(false)
}
