//! File 6/49: exact-source bounded txDecoder.js body layout observations.
//!
//! Pinned JS: patrickdugan/tradelayer.js@dca0888b, src/txDecoder.js.
//! This module only decodes caller-supplied BODY text: it never parses an
//! actual Bitcoin/Litecoin/Dogecoin transaction or verifies script, witness,
//! first-input sender, activation, list administrator, UTXO or signatures.
//!
//! Source-specific hazards included in the read-only research surface:
//! - Type4 comma split occurs BEFORE bracketed clearlist parsing. [7,b]
//!   becomes separate fields and produces NaN/field drift, not two lists.
//! - Type9 calls parts[3].toUpperCase() unconditionally, throwing if absent.
//! - Type4/22 call startsWith on a required indexed field, throwing if absent.
//! - Type4/22 references stay raw TEXT after ref:, Type21 ref is parseInt
//!   BASE TEN and becomes false if NaN, not base36 list-id parsing.
//! - Type23 decodeSettleChannelPNL is defined TWICE: the later base94
//!   method shadows the earlier base256 method. Neither is authorized here.
//! - decodeCloseOracle returns a primitive integer, not a decoded object.
//! - Standalone decodeAbstractionBridge exists but Types dispatch Type34
//!   invokes ABSENT decodeCrossLayerBridge. Never silently alias those.
//! - Types32/33 return decoded params WITHOUT a valid flag or validator.
//!
//! Defensive limitations beyond JS are explicit: canonical full-field
//! base36 and decimal integers, bounded printable ASCII, safe integer atoms,
//! and no implicit fallback for malformed/partial parseInt or malformed
//! property references. These limits are NOT claims about historical rules.
//!
//! Outputs are unverified observations, with no trusted adapter and no
//! method to turn them into ledger or channel authorization. Every permission
//! request permanently returns a denial.

use crate::wire::{parse_base36, WireError};

pub const MAX_DECODER_BODY_BYTES: usize = 4096;
pub const MAX_SOURCE_SAFE_INTEGER: u128 = 9_007_199_254_740_991;
pub const MAX_SOURCE_CLEARLISTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyDecoderError {
    UnsupportedOrUnportedLayout(u8),
    MissingSourceDecoderExport(u8),
    BodyTooLong,
    NonPrintableSourceText,
    NestedOrFramedBody,
    MissingIndexedFieldWouldThrow(&'static str),
    MissingAttestationMetadataWouldThrow,
    NonCanonicalLegacyNumeric(&'static str),
    AmountNotExactlyRepresentable,
    MalformedSyntheticProperty,
    BracketedListSplitBeforeParsing,
    TooManyClearlistIds,
    WrongSourceBodyType,
    HistoricalTransactionNotAuthenticated,
    RegistryAndActivationUnverified,
    CanonicalSignedChannelRevisionAbsent,
    TrustedEconomicExecutionForbidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourcePropertyId {
    Regular(u64),
    /// Source decodePropertyId accepts both s-<id>-<id> and s<id>-<id>.
    /// It converts the two base36 fields to decimal JS numeric IDs.
    Synthetic { collateral_id: u64, contract_id: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceChannelLocation {
    /// The source stores the channel address directly, including empty.
    Address(String),
    /// Type4/22 keep text after ref: without numeric normalization.
    RawReference(String),
    /// Type21 uses decimal parseInt, with NaN mapped to false.
    WithdrawalReference(Option<u64>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnverifiedSourceBody {
    Commit {
        property: SourcePropertyId,
        amount_atoms: u128,
        channel: SourceChannelLocation,
        pay_enabled: bool,
        clear_lists: Vec<u64>,
        colored_output: bool,
        commit_clearlist_id: Option<u64>,
    },
    CreateWhitelist {
        backup_address: String,
        name: String,
        url: String,
        description: String,
    },
    UpdateAdmin {
        new_address: String,
        whitelist: bool,
        oracle: bool,
        token: bool,
        list_id: u64,
        update_backup: bool,
    },
    Attestation {
        revoke: bool,
        list_id: u64,
        target_address: String,
        metadata_upper: String,
    },
    /// Legacy close oracle decoder returns a primitive number.
    CloseOraclePrimitive(u64),
    Withdrawal {
        withdraw_all: bool,
        property: SourcePropertyId,
        amount_atoms: u128,
        column_flag: bool,
        channel: SourceChannelLocation,
    },
    Transfer {
        property: SourcePropertyId,
        amount_atoms: u128,
        column_a: bool,
        destination: SourceChannelLocation,
        pay: bool,
        pay_reference: String,
    },
    StakeFraudRelay {
        action: u64,
        oracle_id: u64,
        staked_property_id: u64,
        amount_atoms: u128,
        accused_address: String,
        evidence_hash: String,
        relay_type: u64,
        state_hash: String,
        dlc_ref: String,
        settlement_state: String,
        relay_blob: String,
        auto_roll: bool,
        next_dlc_ref: String,
    },
    /// Source Type32 decoder returns only ordinalRevealJSON, no valid flag.
    ZkBatchOpaque { ordinal_reveal_json: String },
    /// Type33 has a decoder, but Types has its validator commented out.
    ColoredCoin {
        action_decimal: u64,
        property_id: u64,
        sats_ratio: u64,
        home_address: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDecoderCaveat {
    AmountFloatLimitedToExactSafeAtoms,
    CommitBracketedClearlistCommasMisparse,
    AttestationMissingMetadataThrows,
    TwoDifferentReferenceNumberRules,
    DuplicateSettlementDecoderShadowsFirst,
    CloseOraclePrimitiveReplacesBlockField,
    Types32And33ValidatorCommentedOut,
    StandaloneBridgeHelperNotDispatchedByType34,
    SourceJavaScriptAllowsPartialParseInt,
    NoChainOrHistoricalStateEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedDecodedBody {
    type_id: u8,
    fields: UnverifiedSourceBody,
    caveats: Vec<SourceDecoderCaveat>,
}
impl UnverifiedDecodedBody {
    pub const fn type_id(&self) -> u8 {
        self.type_id
    }
    pub fn source_fields(&self) -> &UnverifiedSourceBody {
        &self.fields
    }
    pub fn caveats(&self) -> &[SourceDecoderCaveat] {
        &self.caveats
    }
    pub const fn require_historically_accepted_transaction(&self)
        -> Result<(), LegacyDecoderError>
    {
        Err(LegacyDecoderError::HistoricalTransactionNotAuthenticated)
    }
    pub const fn require_accepted_registry_transition(&self)
        -> Result<(), LegacyDecoderError>
    {
        Err(LegacyDecoderError::RegistryAndActivationUnverified)
    }
    pub const fn require_canonical_signed_channel_revision(&self)
        -> Result<(), LegacyDecoderError>
    {
        Err(LegacyDecoderError::CanonicalSignedChannelRevisionAbsent)
    }
    pub const fn require_trusted_execution(&self) -> Result<(), LegacyDecoderError> {
        Err(LegacyDecoderError::TrustedEconomicExecutionForbidden)
    }
}

fn body_check(body: &str) -> Result<(), LegacyDecoderError> {
    if body.len() > MAX_DECODER_BODY_BYTES {
        return Err(LegacyDecoderError::BodyTooLong);
    }
    if !body.is_ascii() || !body.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(LegacyDecoderError::NonPrintableSourceText);
    }
    // Caller must pass the exact POST-TYPE body. Do not double-decode
    // generic Types outer prefixes or silently consume arbitrary selectors.
    if body.starts_with("tl") {
        return Err(LegacyDecoderError::NestedOrFramedBody);
    }
    Ok(())
}
fn source_field<'a>(parts: &'a [&str], index: usize) -> &'a str {
    parts.get(index).copied().unwrap_or("")
}
fn numeric(
    raw: &str,
    default_zero: bool,
    field: &'static str,
) -> Result<u64, LegacyDecoderError> {
    let value = if raw.is_empty() && default_zero { "0" } else { raw };
    if value.is_empty() {
        return Err(LegacyDecoderError::NonCanonicalLegacyNumeric(field));
    }
    let n = parse_base36(value)
        .map_err(|_err: WireError| LegacyDecoderError::NonCanonicalLegacyNumeric(field))?;
    if n > MAX_SOURCE_SAFE_INTEGER {
        return Err(LegacyDecoderError::NonCanonicalLegacyNumeric(field));
    }
    u64::try_from(n).map_err(|_| LegacyDecoderError::NonCanonicalLegacyNumeric(field))
}
fn amount_atoms(raw: &str) -> Result<u128, LegacyDecoderError> {
    // Source BigNumber(base36).div(1e8).toNumber; restrict to safe integer
    // atoms to avoid claiming equivalence beyond JS Number precision.
    let atoms = u128::from(numeric(raw, true, "amount")?);
    if atoms > MAX_SOURCE_SAFE_INTEGER {
        return Err(LegacyDecoderError::AmountNotExactlyRepresentable);
    }
    Ok(atoms)
}

pub fn source_decode_property_id_unverified(
    raw: &str,
) -> Result<SourcePropertyId, LegacyDecoderError> {
    if let Some(trim) = raw.strip_prefix('s') {
        let segments: Vec<_> = trim.split('-').collect();
        let (collateral, contract) = match segments.as_slice() {
            [a, b] if !a.is_empty() && !b.is_empty() => (*a, *b),
            ["", a, b] if !a.is_empty() && !b.is_empty() => (*a, *b),
            _ => return Err(LegacyDecoderError::MalformedSyntheticProperty),
        };
        return Ok(SourcePropertyId::Synthetic {
            collateral_id: numeric(collateral, false, "synthetic collateral")?,
            contract_id: numeric(contract, false, "synthetic contract")?,
        });
    }
    Ok(SourcePropertyId::Regular(numeric(raw, false, "property id")?))
}

fn literal_flag(value: &str) -> bool {
    value == "1"
}

fn source_decimal_ref(raw: &str) -> Option<u64> {
    // Historical parseInt(raw,10) takes an integer PREFIX, not a complete
    // canonical decimal number. Its NaN becomes false for withdrawal refs.
    let rest = raw.trim_start();
    let positive = rest.strip_prefix('+').unwrap_or(rest);
    if positive.starts_with('-') {
        return None;
    }
    let digits: String = positive.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok().filter(|n| u128::from(*n) <= MAX_SOURCE_SAFE_INTEGER)
}

fn location_string(raw: &str) -> SourceChannelLocation {
    match raw.strip_prefix("ref:") {
        Some(tail) => SourceChannelLocation::RawReference(
            tail.split(':').next().unwrap_or("").to_owned(),
        ),
        None => SourceChannelLocation::Address(raw.to_owned()),
    }
}

fn source_decode_commit(parts: &[&str]) -> Result<UnverifiedSourceBody, LegacyDecoderError> {
    let channel = parts.get(2).ok_or(
        LegacyDecoderError::MissingIndexedFieldWouldThrow("commit channel field"),
    )?;
    let property = source_decode_property_id_unverified(source_field(parts, 0))?;
    let raw_amount = source_field(parts, 1);
    let amount_atoms = amount_atoms(raw_amount)?;
    let mut clear_lists = Vec::new();
    let list_field = source_field(parts, 4);
    if list_field.starts_with('[') && list_field.ends_with(']') {
        let entries = &list_field[1..list_field.len() - 1];
        for value in entries.split(',') {
            clear_lists.push(numeric(value, false, "clearlist id")?);
        }
    } else if !list_field.is_empty() {
        if list_field.starts_with('[') || list_field.ends_with(']') {
            return Err(LegacyDecoderError::BracketedListSplitBeforeParsing);
        }
        clear_lists.push(numeric(list_field, false, "clearlist id")?);
    }
    if clear_lists.len() > MAX_SOURCE_CLEARLISTS {
        return Err(LegacyDecoderError::TooManyClearlistIds);
    }
    Ok(UnverifiedSourceBody::Commit {
        property,
        amount_atoms,
        channel: location_string(channel),
        pay_enabled: literal_flag(source_field(parts, 3)),
        clear_lists,
        colored_output: literal_flag(source_field(parts, 5)),
        commit_clearlist_id: match parts.get(6).filter(|s| !s.is_empty()) {
            None => None,
            Some(field) => Some(numeric(field, false, "commit clearlist id")?),
        },
    })
}

fn source_decode_withdrawal(parts: &[&str]) -> Result<UnverifiedSourceBody, LegacyDecoderError> {
    let place = source_field(parts, 4);
    let channel = match place.strip_prefix("ref:") {
        Some(raw) => SourceChannelLocation::WithdrawalReference(
            source_decimal_ref(raw.split(':').next().unwrap_or("")),
        ),
        None => SourceChannelLocation::Address(place.to_owned()),
    };
    Ok(UnverifiedSourceBody::Withdrawal {
        withdraw_all: literal_flag(source_field(parts, 0)),
        property: source_decode_property_id_unverified(source_field(parts, 1))?,
        amount_atoms: amount_atoms(source_field(parts, 2))?,
        column_flag: literal_flag(source_field(parts, 3)),
        channel,
    })
}

fn source_decode_transfer(parts: &[&str]) -> Result<UnverifiedSourceBody, LegacyDecoderError> {
    let place = parts.get(3).ok_or(
        LegacyDecoderError::MissingIndexedFieldWouldThrow("transfer destination"),
    )?;
    Ok(UnverifiedSourceBody::Transfer {
        property: source_decode_property_id_unverified(source_field(parts, 0))?,
        amount_atoms: amount_atoms(source_field(parts, 1))?,
        column_a: literal_flag(source_field(parts, 2)),
        destination: location_string(place),
        pay: literal_flag(source_field(parts, 4)),
        pay_reference: source_field(parts, 5).to_owned(),
    })
}

/// Caller supplies only the body AFTER the type selector. The actual JS
/// accepts malformed partial numeric strings and may yield NaN; this strict
/// Rust subset rejects those instead of inventing a safe economic value.
/// It does not accept a full OP_RETURN or a generic double-prefixed body.
pub fn source_decode_body_unverified(
    type_id: u8,
    body: &str,
) -> Result<UnverifiedDecodedBody, LegacyDecoderError> {
    body_check(body)?;
    if [28, 29, 34, 35].contains(&type_id) {
        return Err(LegacyDecoderError::MissingSourceDecoderExport(type_id));
    }
    let parts: Vec<_> = body.split(',').collect();
    let mut caveats = vec![SourceDecoderCaveat::NoChainOrHistoricalStateEvidence];
    let fields = match type_id {
        4 => {
            caveats.push(SourceDecoderCaveat::CommitBracketedClearlistCommasMisparse);
            caveats.push(SourceDecoderCaveat::AmountFloatLimitedToExactSafeAtoms);
            caveats.push(SourceDecoderCaveat::TwoDifferentReferenceNumberRules);
            source_decode_commit(&parts)?
        }
        7 => UnverifiedSourceBody::CreateWhitelist {
            backup_address: source_field(&parts, 0).to_owned(),
            name: source_field(&parts, 1).to_owned(),
            url: source_field(&parts, 2).to_owned(),
            description: source_field(&parts, 3).to_owned(),
        },
        8 => UnverifiedSourceBody::UpdateAdmin {
            new_address: source_field(&parts, 0).to_owned(),
            whitelist: literal_flag(source_field(&parts, 1)),
            oracle: literal_flag(source_field(&parts, 2)),
            token: literal_flag(source_field(&parts, 3)),
            list_id: numeric(source_field(&parts, 4), true, "admin target id")?,
            update_backup: literal_flag(source_field(&parts, 5)),
        },
        9 => {
            caveats.push(SourceDecoderCaveat::AttestationMissingMetadataThrows);
            let metadata = parts.get(3).ok_or(
                LegacyDecoderError::MissingAttestationMetadataWouldThrow,
            )?;
            UnverifiedSourceBody::Attestation {
                revoke: literal_flag(source_field(&parts, 0)),
                list_id: numeric(source_field(&parts, 1), true, "attestation list id")?,
                target_address: source_field(&parts, 2).to_owned(),
                metadata_upper: metadata.to_ascii_uppercase(),
            }
        }
        15 => UnverifiedSourceBody::CloseOraclePrimitive(
            numeric(body, true, "close oracle primitive")?,
        ),
        21 => {
            caveats.push(SourceDecoderCaveat::TwoDifferentReferenceNumberRules);
            caveats.push(SourceDecoderCaveat::AmountFloatLimitedToExactSafeAtoms);
            source_decode_withdrawal(&parts)?
        }
        22 => {
            caveats.push(SourceDecoderCaveat::TwoDifferentReferenceNumberRules);
            caveats.push(SourceDecoderCaveat::AmountFloatLimitedToExactSafeAtoms);
            source_decode_transfer(&parts)?
        }
        30 => {
            caveats.push(SourceDecoderCaveat::AmountFloatLimitedToExactSafeAtoms);
            UnverifiedSourceBody::StakeFraudRelay {
                action: numeric(source_field(&parts, 0), true, "stake action")?,
                oracle_id: numeric(source_field(&parts, 1), true, "stake oracle")?,
                staked_property_id: numeric(source_field(&parts, 2), true, "stake property")?,
                amount_atoms: amount_atoms(source_field(&parts, 3))?,
                accused_address: source_field(&parts, 4).to_owned(),
                evidence_hash: source_field(&parts, 5).to_owned(),
                relay_type: numeric(source_field(&parts, 6), true, "stake relay type")?,
                state_hash: source_field(&parts, 7).to_owned(),
                dlc_ref: source_field(&parts, 8).to_owned(),
                settlement_state: source_field(&parts, 9).to_owned(),
                relay_blob: source_field(&parts, 10).to_owned(),
                auto_roll: literal_flag(source_field(&parts, 11)),
                next_dlc_ref: source_field(&parts, 12).to_owned(),
            }
        }
        32 => {
            caveats.push(SourceDecoderCaveat::Types32And33ValidatorCommentedOut);
            UnverifiedSourceBody::ZkBatchOpaque {
                ordinal_reveal_json: body.to_owned(),
            }
        }
        33 => {
            caveats.push(SourceDecoderCaveat::Types32And33ValidatorCommentedOut);
            let action = source_field(&parts, 0);
            if action.is_empty() || !action.bytes().all(|x| x.is_ascii_digit()) {
                return Err(LegacyDecoderError::NonCanonicalLegacyNumeric("colored action"));
            }
            let action_decimal = action.parse::<u64>()
                .map_err(|_| LegacyDecoderError::NonCanonicalLegacyNumeric("colored action"))?;
            UnverifiedSourceBody::ColoredCoin {
                action_decimal,
                property_id: numeric(source_field(&parts, 1), false, "colored property")?,
                sats_ratio: numeric(source_field(&parts, 2), false, "colored ratio")?,
                home_address: source_field(&parts, 3).to_owned(),
            }
        }
        _ => return Err(LegacyDecoderError::UnsupportedOrUnportedLayout(type_id)),
    };
    Ok(UnverifiedDecodedBody { type_id, fields, caveats })
}

/// Historical Type34 Types dispatcher calls decodeCrossLayerBridge, which
/// DOES NOT exist in txDecoder.js. The standalone decodeAbstractionBridge
/// helper DOES exist but cannot be silently substituted into Type34.
/// This method is an isolated read-only helper observation only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedStandaloneBridge {
    property_id: u64,
    amount: u64,
}
impl UnverifiedStandaloneBridge {
    pub const fn property_id(&self) -> u64 {
        self.property_id
    }
    pub const fn amount(&self) -> u64 {
        self.amount
    }
    pub const fn require_trusted_cross_layer_bridge(&self) -> Result<(), LegacyDecoderError> {
        Err(LegacyDecoderError::TrustedEconomicExecutionForbidden)
    }
}
pub fn source_standalone_abstraction_bridge_helper_unverified(
    body: &str,
) -> Result<UnverifiedStandaloneBridge, LegacyDecoderError> {
    body_check(body)?;
    let parts: Vec<_> = body.split(',').collect();
    Ok(UnverifiedStandaloneBridge {
        property_id: numeric(source_field(&parts, 0), false, "bridge property")?,
        amount: numeric(source_field(&parts, 1), false, "bridge amount")?,
    })
}

/// Source first decodeSettleChannelPNL (base256) is SHADOWED by a second
/// same-named property (base94) in the same object literal. Runtime uses
/// the SECOND definition. Neither source Type23 settlement is authorized.
pub const fn source_type23_uses_later_base94_decoder() -> bool {
    true
}
