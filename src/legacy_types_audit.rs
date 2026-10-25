//! Source-pinned TradeLayer JS types.js encoder/decoder dispatch inventory.
//!
//! Historical pin: patrickdugan/tradelayer.js@dca0888b, src/types.js,
//! src/txEncoder.js and src/txDecoder.js (byte-identical archives in fixtures).
//! This is STATIC SOURCE CHARACTERIZATION and no transaction is decoded,
//! verified, admitted, signed, or authorized by this module.
//!
//! Source findings: generic encodePayload prepends tl+selector BEFORE calling
//! the individual encoder. Most encoders ALSO prefix tl+selector. Type2
//! sendAll is body-only but scalar send is doubly prefixed; types32-35 are
//! body-only; type4 refers to an absent encodeCommitToken export.
//! Type15's decoder yields a primitive number and dispatch invokes the
//! WRONG validator (publish, not close) after discarding the block field.
//! Type28/29/34/35 name decoders that are absent from the pinned exports.
//! Type32/33 decode but do not call validators (commented-out lines).
//! Type3 without both referenced outputs RETURNS unvalidated decoded params.
//! txIndex.processTransaction passes 3 args into the 9-arg decodePayload;
//! shifted marker argument returns an Error OBJECT, not thrown exception.
//! A source result with valid undefined would slip past logic.typeSwitch's
//! condition (params.valid == false), but no such behavior is authorized here.
//!
//! Never grant a permission from a route, encoder assembly or source claim.
//! All authorization methods ALWAYS return errors. The independently
//! maintained Rust wire parser and registry/ledger/clearing remain untouched.

pub const LEGACY_SOURCE_TYPE_COUNT: usize = 36;
pub const MAX_SOURCE_TYPES_ASCII: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyTypeRoute {
    pub type_id: u8,
    pub encoder: &'static str,
    pub encoder_exported: bool,
    pub decoder: &'static str,
    pub decoder_exported: bool,
    pub validator: Option<&'static str>,
    pub validator_invoked_by_source: bool,
}

/// Actual method identifiers used by pinned types.js, and their *presence*
/// in pinned txEncoder/txDecoder. Metadata is NEVER a protocol activation.
pub const LEGACY_TYPE_ROUTES: [LegacyTypeRoute; LEGACY_SOURCE_TYPE_COUNT] = [
    LegacyTypeRoute { type_id: 0, encoder: "encodeActivateTradeLayer", encoder_exported: true, decoder: "decodeActivateTradeLayer", decoder_exported: true, validator: Some("validateActivateTradeLayer"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 1, encoder: "encodeTokenIssue", encoder_exported: true, decoder: "decodeTokenIssue", decoder_exported: true, validator: Some("validateTokenIssue"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 2, encoder: "encodeSend", encoder_exported: true, decoder: "decodeSend", decoder_exported: true, validator: Some("validateSend"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 3, encoder: "encodeTradeTokenForUTXO", encoder_exported: true, decoder: "decodeTradeTokenForUTXO", decoder_exported: true, validator: Some("validateTradeTokenForUTXO"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 4, encoder: "encodeCommitToken", encoder_exported: false, decoder: "decodeCommitToken", decoder_exported: true, validator: Some("validateCommit"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 5, encoder: "encodeOnChainTokenForToken", encoder_exported: true, decoder: "decodeOnChainTokenForToken", decoder_exported: true, validator: Some("validateOnChainTokenForToken"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 6, encoder: "encodeCancelOrder", encoder_exported: true, decoder: "decodeCancelOrder", decoder_exported: true, validator: Some("validateCancelOrder"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 7, encoder: "encodeCreateWhitelist", encoder_exported: true, decoder: "decodeCreateWhitelist", decoder_exported: true, validator: Some("validateCreateWhitelist"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 8, encoder: "encodeUpdateAdmin", encoder_exported: true, decoder: "decodeUpdateAdmin", decoder_exported: true, validator: Some("validateUpdateAdmin"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 9, encoder: "encodeIssueOrRevokeAttestation", encoder_exported: true, decoder: "decodeIssueOrRevokeAttestation", decoder_exported: true, validator: Some("validateIssueOrRevokeAttestation"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 10, encoder: "encodeAMMPool", encoder_exported: true, decoder: "decodeAMMPool", decoder_exported: true, validator: Some("validateAMMPool"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 11, encoder: "encodeGrantManagedToken", encoder_exported: true, decoder: "decodeGrantManagedToken", decoder_exported: true, validator: Some("validateGrantManagedToken"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 12, encoder: "encodeRedeemManagedToken", encoder_exported: true, decoder: "decodeRedeemManagedToken", decoder_exported: true, validator: Some("validateRedeemManagedToken"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 13, encoder: "encodeCreateOracle", encoder_exported: true, decoder: "decodeCreateOracle", decoder_exported: true, validator: Some("validateCreateOracle"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 14, encoder: "encodePublishOracleData", encoder_exported: true, decoder: "decodePublishOracleData", decoder_exported: true, validator: Some("validatePublishOracleData"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 15, encoder: "encodeCloseOracle", encoder_exported: true, decoder: "decodeCloseOracle", decoder_exported: true, validator: Some("validatePublishOracleData"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 16, encoder: "encodeCreateFutureContractSeries", encoder_exported: true, decoder: "decodeCreateFutureContractSeries", decoder_exported: true, validator: Some("validateCreateContractSeries"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 17, encoder: "encodeExerciseDerivative", encoder_exported: true, decoder: "decodeExerciseDerivative", decoder_exported: true, validator: Some("validateExerciseDerivative"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 18, encoder: "encodeTradeContractOnchain", encoder_exported: true, decoder: "decodeTradeContractOnchain", decoder_exported: true, validator: Some("validateTradeContractOnchain"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 19, encoder: "encodeTradeContractChannel", encoder_exported: true, decoder: "decodeTradeContractChannel", decoder_exported: true, validator: Some("validateTradeContractChannel"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 20, encoder: "encodeTradeTokensChannel", encoder_exported: true, decoder: "decodeTradeTokensChannel", decoder_exported: true, validator: Some("validateTradeTokensChannel"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 21, encoder: "encodeWithdrawal", encoder_exported: true, decoder: "decodeWithdrawal", decoder_exported: true, validator: Some("validateWithdrawal"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 22, encoder: "encodeTransfer", encoder_exported: true, decoder: "decodeTransfer", decoder_exported: true, validator: Some("validateTransfer"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 23, encoder: "encodeSettleChannelPNL", encoder_exported: true, decoder: "decodeSettleChannelPNL", decoder_exported: true, validator: Some("validateSettleChannelPNL"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 24, encoder: "encodeMintSynthetic", encoder_exported: true, decoder: "decodeMintSynthetic", decoder_exported: true, validator: Some("validateMintSynthetic"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 25, encoder: "encodeRedeemSynthetic", encoder_exported: true, decoder: "decodeRedeemSynthetic", decoder_exported: true, validator: Some("validateRedeemSynthetic"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 26, encoder: "encodePayToTokens", encoder_exported: true, decoder: "decodePayToTokens", decoder_exported: true, validator: Some("validatePayToTokens"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 27, encoder: "encodeOptionTrade", encoder_exported: true, decoder: "decodeOptionTrade", decoder_exported: true, validator: Some("validateOptionTrade"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 28, encoder: "encodeTradeBaiUrbun", encoder_exported: true, decoder: "decodeTradeBaiUrbun", decoder_exported: false, validator: Some("validateTradeBaiUrbun"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 29, encoder: "encodeTradeMurabaha", encoder_exported: true, decoder: "decodeTradeMurabaha", decoder_exported: false, validator: Some("validateTradeMurabaha"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 30, encoder: "encodeStakeFraudProof", encoder_exported: true, decoder: "decodeStakeFraudProof", decoder_exported: true, validator: Some("validateStakeFraudProof"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 31, encoder: "encodeKingSettle", encoder_exported: true, decoder: "decodeKingSettle", decoder_exported: true, validator: Some("validateKingSettle"), validator_invoked_by_source: true },
    LegacyTypeRoute { type_id: 32, encoder: "encodeBatchMoveZkRollup", encoder_exported: true, decoder: "decodeBatchMoveZkRollup", decoder_exported: true, validator: None, validator_invoked_by_source: false },
    LegacyTypeRoute { type_id: 33, encoder: "encodeColoredCoin", encoder_exported: true, decoder: "decodeColoredCoin", decoder_exported: true, validator: None, validator_invoked_by_source: false },
    LegacyTypeRoute { type_id: 34, encoder: "encodeAbstractionBridge", encoder_exported: true, decoder: "decodeCrossLayerBridge", decoder_exported: false, validator: None, validator_invoked_by_source: false },
    LegacyTypeRoute { type_id: 35, encoder: "encodeBindSmartContract", encoder_exported: true, decoder: "decodeSmartContractBind", decoder_exported: false, validator: None, validator_invoked_by_source: false },

];

pub fn source_route(type_id: u8) -> Option<&'static LegacyTypeRoute> {
    LEGACY_TYPE_ROUTES.get(usize::from(type_id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyTypesError {
    UnknownSourceType(u8),
    MissingSourceEncoder(u8),
    ClaimNotPrintableAscii,
    ClaimedPayloadTooLarge,
    InputAndHistoricStateNotAuthenticated,
    CanonicalTwoSignerChannelStateAbsent,
    TrustedTransactionExecutionForbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericEnvelopeKind {
    /// e.g. source scalar type2 or a source-prefixed type9 inner result.
    DuplicatedTypePrefix,
    /// e.g. source type32-35 and type2 sendAll inner body.
    SingleOuterPrefix,
    /// Caller provided a different type prefix; do not infer any validity.
    ForeignInnerPrefix,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedGenericAssembly {
    type_id: u8,
    claimed_generic_bytes: String,
    envelope_kind: GenericEnvelopeKind,
}
impl UnverifiedGenericAssembly {
    pub const fn type_id(&self) -> u8 {
        self.type_id
    }
    pub fn claimed_ascii(&self) -> &str {
        &self.claimed_generic_bytes
    }
    pub const fn envelope_kind(&self) -> GenericEnvelopeKind {
        self.envelope_kind
    }
    pub const fn require_encoded_transaction_authority(&self) -> Result<(), LegacyTypesError> {
        Err(LegacyTypesError::TrustedTransactionExecutionForbidden)
    }
}
fn source_type_digit(number: u8) -> char {
    if number < 10 {
        char::from(b'0' + number)
    } else {
        char::from(b'a' + number - 10)
    }
}

/// Source encodePayload computes "tl" + type.toString(36) + returnValue.
/// This function takes a CALLER-SUPPLIED inner return; it does not and
/// cannot invoke historical JS encoders. Type4 is refused because the exact
/// referenced encoder export is missing. Bare, nested and foreign carriers
/// remain merely characterized; no send-ready or trusted carrier is issued.
pub fn source_generic_concat_unverified(
    type_id: u8,
    caller_claimed_inner: &str,
) -> Result<UnverifiedGenericAssembly, LegacyTypesError> {
    let route = source_route(type_id).ok_or(LegacyTypesError::UnknownSourceType(type_id))?;
    if !route.encoder_exported {
        return Err(LegacyTypesError::MissingSourceEncoder(type_id));
    }
    if caller_claimed_inner.len() + 3 > MAX_SOURCE_TYPES_ASCII {
        return Err(LegacyTypesError::ClaimedPayloadTooLarge);
    }
    if !caller_claimed_inner.is_ascii()
        || !caller_claimed_inner.bytes().all(|c| (0x20..=0x7e).contains(&c))
    {
        return Err(LegacyTypesError::ClaimNotPrintableAscii);
    }
    let selector = source_type_digit(type_id);
    let envelope_kind = if let Some(after_marker) = caller_claimed_inner.strip_prefix("tl") {
        if after_marker.starts_with(selector) {
            GenericEnvelopeKind::DuplicatedTypePrefix
        } else {
            GenericEnvelopeKind::ForeignInnerPrefix
        }
    } else {
        GenericEnvelopeKind::SingleOuterPrefix
    };
    Ok(UnverifiedGenericAssembly {
        type_id,
        claimed_generic_bytes: format!("tl{selector}{caller_claimed_inner}"),
        envelope_kind,
    })
}

/// Actual decodePayload signature is nine arguments. The pinned txIndex
/// calls it with THREE: (txId, marker, payload). The second argument, "tl",
/// becomes the type; the third, the decoded after-marker payload, becomes
/// the marker. Usually the marker check returns Error("Invalid payload");
/// in the exceptional case payload == "tl", the switch receives a STRING
/// type and returns Error("Unknown transaction type"). Neither is thrown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDecoderInvocation<'a> {
    FullNineArguments {
        type_id: u8,
        marker: &'a str,
        /// For source type3 only: both vouts found in an externally supplied
        /// reference array (not authentic UTXO or source output evidence).
        type3_both_output_refs_claimed: bool,
    },
    TxIndexThreeArguments {
        payload_after_marker: &'a str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDecodeOutcome {
    ReturnedErrorObjectInvalidMarker,
    ReturnedErrorObjectUnknownType,
    WouldThrowMissingDecoder,
    Type3UnvalidatedReturnMissingReferences,
    Type15CallsWrongValidatorWithNumber,
    DecodedWithoutAnyValidator,
    WouldInvokeValidatorOnCallerClaims,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceDecodeCaveat {
    TxIndexMisorderedCallSignature,
    Type3ValidatorSkippedIfRefsMissing,
    Type15PublishOracleValidatorInsteadOfClose,
    Type15DecoderReturnsNumberAndBlockAssignmentLost,
    Type23ChannelAddressIsInputSender,
    Type31KingSettleNameVsOtherRegistries,
    Type32To35ValidatorCallsCommentedOut,
    Type34CrossLayerBridgeVsAbstractionBridgeName,
    UnverifiedExternalDecoderAndState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedTypesDispatch {
    selector: Option<u8>,
    outcome: SourceDecodeOutcome,
    caveats: Vec<SourceDecodeCaveat>,
}

impl UnverifiedTypesDispatch {
    pub const fn type_id(&self) -> Option<u8> {
        self.selector
    }
    pub const fn outcome(&self) -> SourceDecodeOutcome {
        self.outcome
    }
    pub fn caveats(&self) -> &[SourceDecodeCaveat] {
        &self.caveats
    }
    pub const fn require_historical_transaction_admission(&self) -> Result<(), LegacyTypesError> {
        Err(LegacyTypesError::InputAndHistoricStateNotAuthenticated)
    }
    pub const fn require_canonical_channel_revision(&self) -> Result<(), LegacyTypesError> {
        Err(LegacyTypesError::CanonicalTwoSignerChannelStateAbsent)
    }
    pub const fn require_trusted_execution(&self) -> Result<(), LegacyTypesError> {
        Err(LegacyTypesError::TrustedTransactionExecutionForbidden)
    }
}

/// Classifies only the *source JS dispatch call path*, without executing
/// any decoder, validator or handler, or blessing an encoded payload.
/// A source-validator invocation is NOT a positive validity verdict.
pub fn source_inspect_decode_invocation(
    call: SourceDecoderInvocation<'_>,
) -> UnverifiedTypesDispatch {
    match call {
        SourceDecoderInvocation::TxIndexThreeArguments { payload_after_marker } => {
            let outcome = if payload_after_marker == "tl" {
                SourceDecodeOutcome::ReturnedErrorObjectUnknownType
            } else {
                SourceDecodeOutcome::ReturnedErrorObjectInvalidMarker
            };
            UnverifiedTypesDispatch {
                selector: None,
                outcome,
                caveats: vec![SourceDecodeCaveat::TxIndexMisorderedCallSignature],
            }
        }
        SourceDecoderInvocation::FullNineArguments {
            type_id,
            marker,
            type3_both_output_refs_claimed,
        } => {
            let mut caveats = vec![SourceDecodeCaveat::UnverifiedExternalDecoderAndState];
            let outcome = if marker != "tl" {
                SourceDecodeOutcome::ReturnedErrorObjectInvalidMarker
            } else if let Some(route) = source_route(type_id) {
                if !route.decoder_exported {
                    SourceDecodeOutcome::WouldThrowMissingDecoder
                } else if type_id == 3 && !type3_both_output_refs_claimed {
                    caveats.push(SourceDecodeCaveat::Type3ValidatorSkippedIfRefsMissing);
                    SourceDecodeOutcome::Type3UnvalidatedReturnMissingReferences
                } else if type_id == 15 {
                    caveats.push(SourceDecodeCaveat::Type15PublishOracleValidatorInsteadOfClose);
                    caveats.push(SourceDecodeCaveat::Type15DecoderReturnsNumberAndBlockAssignmentLost);
                    SourceDecodeOutcome::Type15CallsWrongValidatorWithNumber
                } else if !route.validator_invoked_by_source {
                    caveats.push(SourceDecodeCaveat::Type32To35ValidatorCallsCommentedOut);
                    SourceDecodeOutcome::DecodedWithoutAnyValidator
                } else {
                    if type_id == 23 {
                        caveats.push(SourceDecodeCaveat::Type23ChannelAddressIsInputSender);
                    }
                    if type_id == 31 {
                        caveats.push(SourceDecodeCaveat::Type31KingSettleNameVsOtherRegistries);
                    }
                    SourceDecodeOutcome::WouldInvokeValidatorOnCallerClaims
                }
            } else {
                SourceDecodeOutcome::ReturnedErrorObjectUnknownType
            };
            UnverifiedTypesDispatch {
                selector: Some(type_id),
                outcome,
                caveats,
            }
        }
    }
}

/// Source Types.resolveReferenceAddress chooses a string unchanged, first
/// nonempty array entry.address, or object's address then senderAddress.
/// Actual references are untrusted; no output vout is verified by this helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimedReference<'a> {
    Missing,
    String(&'a str),
    Array(&'a [Option<&'a str>]),
    Object {
        address: Option<&'a str>,
        sender_address: Option<&'a str>,
    },
    UnsupportedPrimitive,
}
pub fn source_resolve_reference_address(reference: ClaimedReference<'_>) -> &str {
    match reference {
        ClaimedReference::String(s) => s,
        ClaimedReference::Array(rows) => rows.iter().flatten().copied().find(|s| !s.is_empty()).unwrap_or(""),
        ClaimedReference::Object { address, sender_address } => {
            address.filter(|s| !s.is_empty())
                .or_else(|| sender_address.filter(|s| !s.is_empty()))
                .unwrap_or("")
        }
        ClaimedReference::Missing | ClaimedReference::UnsupportedPrimitive => "",
    }
}

/// Source Type11 fills a missing decoded recipient from the claimed
/// reference's first relevant address; never authenticate a grant with it.
pub fn source_type11_claimed_recipient<'a>(
    decoded_recipient: &'a str,
    reference: ClaimedReference<'a>,
) -> &'a str {
    if decoded_recipient.is_empty() {
        source_resolve_reference_address(reference)
    } else {
        decoded_recipient
    }
}
