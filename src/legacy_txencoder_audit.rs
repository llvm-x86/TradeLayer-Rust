//! File 7/49: bounded source-exact legacy txEncoder.js research output.
//!
//! Source: patrickdugan/tradelayer.js@dca0888b src/txEncoder.js (archived
//! byte-identically at fixtures/legacy_types_source_txEncoder.txt).
//! Values are deterministic SOURCE RETURN STRINGS, never an authorized or
//! send-ready transaction. Amount input is expressed in exact token atoms
//! with 1e8 units/token, limited to JavaScript's 2^53-1 safe integer range.
//!
//! Historical source discrepancies preserved in the audit:
//! - encodeAmount / encodeSend omit "~" on fractional amounts because the
//!   nullish-coalescing fallback is reached only when toString is nullish.
//!   Decoder treats the resulting base36 atomic count as TOKEN UNITS.
//! - encodeSend sendAll/array are body-only; scalar is already tl2-framed.
//! - encodeCommit emits tl4, but Types.encodePayload(4) names ABSENT
//!   encodeCommitToken. Multi-entry bracketed clearLists contain literal
//!   commas, which the source decoder splits before bracket inspection.
//! - encodeIssueOrRevokeAttestation interpolates revoke directly; boolean
//!   true becomes "true", while decode requires literal "1".
//! - encodeCloseOracle ignores its ID due to JS operator precedence,
//!   returning "tlf"; Types separately prefixes "tlf" again.
//! - encodeWithdrawal uses DECIMAL scaled amount, not base36, and emits
//!   "true"/"false" for boolean withdrawAll; decodeWithdrawal expects
//!   base36 amount and literal "1" for withdrawAll.
//! - send/commit/transfer have different inner/outer marker framing.
//! - types33-35 return BODY-only, while Type34 and 35 decoder exports are
//!   absent from the historically pinned Types dispatcher.
//!
//! No JSON, RPC, scripts, private keys, real addresses, Bitcoin/Litecoin/
//! Dogecoin UTXO, list admin, activation history or settlement is validated.
//! Extra strict ASCII, 4096-byte cap, exact atoms and safe integer bounds
//! are *research fail-closed restrictions*, not historical consensus rules.
//! No trusted transaction/ledger/clearing adapter is produced by this file.

use crate::legacy_types_audit::{
    source_generic_concat_unverified, LegacyTypesError, UnverifiedGenericAssembly,
};

pub const LEGACY_ENCODER_MAX_ASCII: usize = 4_096;
pub const ATOMS_PER_TOKEN: u128 = 100_000_000;
pub const JS_SAFE_INTEGER: u128 = 9_007_199_254_740_991;
pub const MAX_SOURCE_LIST_IDS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyEncoderError {
    MissingOrMalformedField,
    NonPrintableAscii,
    SourceReturnTooLong,
    TooManyListIds,
    MismatchedMultiSendArrays,
    NotExactSafeJavaScriptNumber,
    InvalidBase36Number,
    InvalidActivationHex,
    InvalidActivationType,
    HistoricalTransactionNotAuthenticated,
    HistoricalListAndActivationUnverified,
    CanonicalSignedRevisionUnavailable,
    TrustedEconomicExecutionForbidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceInnerEnvelope {
    SingleTypePrefixed,
    BodyOnly,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceEncoderCaveat {
    FractionalAmountMissingTilde,
    MultiSendMisparsedAsScalarByDecoder,
    MultiWhitelistCommaShiftsDecoderFields,
    GenericTypesEncoderMissingType4Export,
    CommitClearlistCommasBreakDecoder,
    LongAddressSubstitutedByCallerClaimedRef,
    RawBooleanRevokeDoesNotDecodeAsTrue,
    CloseOracleIgnoresIdAndReturnsPrefixOnly,
    WithdrawalDecimalAmountVersusBase36Decoder,
    WithdrawalBooleanLiteralVersusOneFlag,
    BodyOnlyWithAbsentSourceDecoder,
    BodyOnlyWithoutCalledValidator,
    HistoricalNativeGenesisUnverified,
    NoHistoricalSignerOrNetworkEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedSourceEncoding {
    source_type_id: u8,
    source_inner_result: String,
    prefix_shape: SourceInnerEnvelope,
    caveats: Vec<SourceEncoderCaveat>,
}
impl UnverifiedSourceEncoding {
    pub const fn type_id(&self) -> u8 {
        self.source_type_id
    }
    pub fn source_return(&self) -> &str {
        &self.source_inner_result
    }
    pub const fn source_inner_prefix_shape(&self) -> SourceInnerEnvelope {
        self.prefix_shape
    }
    pub fn caveats(&self) -> &[SourceEncoderCaveat] {
        &self.caveats
    }
    /// Invokes the File5/49 read-only generic concatenation diagnostic,
    /// which refuses Type4's MISSING encodeCommitToken export. This never
    /// returns an authorized, signature-bearing or send-ready transaction.
    pub fn source_generic_types_concat_unverified(
        &self,
    ) -> Result<UnverifiedGenericAssembly, LegacyTypesError> {
        source_generic_concat_unverified(self.source_type_id, &self.source_inner_result)
    }
    pub const fn require_historically_accepted_transaction(
        &self,
    ) -> Result<(), LegacyEncoderError> {
        Err(LegacyEncoderError::HistoricalTransactionNotAuthenticated)
    }
    pub const fn require_authenticated_list_activation(&self) -> Result<(), LegacyEncoderError> {
        Err(LegacyEncoderError::HistoricalListAndActivationUnverified)
    }
    pub const fn require_signed_channel_revision(&self) -> Result<(), LegacyEncoderError> {
        Err(LegacyEncoderError::CanonicalSignedRevisionUnavailable)
    }
    pub const fn require_trusted_execution(&self) -> Result<(), LegacyEncoderError> {
        Err(LegacyEncoderError::TrustedEconomicExecutionForbidden)
    }
}
fn source_output(
    ty: u8,
    result: String,
    prefix: SourceInnerEnvelope,
    mut caveats: Vec<SourceEncoderCaveat>,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    if result.len() > LEGACY_ENCODER_MAX_ASCII {
        return Err(LegacyEncoderError::SourceReturnTooLong);
    }
    if !result.is_ascii() || !result.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(LegacyEncoderError::NonPrintableAscii);
    }
    caveats.push(SourceEncoderCaveat::NoHistoricalSignerOrNetworkEvidence);
    Ok(UnverifiedSourceEncoding {
        source_type_id: ty,
        source_inner_result: result,
        prefix_shape: prefix,
        caveats,
    })
}
fn raw_text(raw: &str) -> Result<&str, LegacyEncoderError> {
    if raw.len() > LEGACY_ENCODER_MAX_ASCII {
        return Err(LegacyEncoderError::SourceReturnTooLong);
    }
    if !raw.is_ascii() || !raw.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err(LegacyEncoderError::NonPrintableAscii);
    }
    Ok(raw)
}
fn required_text(raw: &str) -> Result<&str, LegacyEncoderError> {
    let s = raw_text(raw)?;
    if s.is_empty() {
        Err(LegacyEncoderError::MissingOrMalformedField)
    } else {
        Ok(s)
    }
}
fn amount_safe(atoms: u128) -> Result<u128, LegacyEncoderError> {
    if atoms > JS_SAFE_INTEGER {
        Err(LegacyEncoderError::NotExactSafeJavaScriptNumber)
    } else {
        Ok(atoms)
    }
}
fn safe_id(id: u128) -> Result<u128, LegacyEncoderError> {
    if id > JS_SAFE_INTEGER {
        Err(LegacyEncoderError::InvalidBase36Number)
    } else {
        Ok(id)
    }
}
fn b36(mut n: u128) -> String {
    if n == 0 {
        return "0".into();
    }
    let mut text = Vec::new();
    const CHARS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    while n != 0 {
        text.push(CHARS[(n % 36) as usize]);
        n /= 36;
    }
    text.reverse();
    String::from_utf8(text).expect("base36 is ASCII")
}
fn b36_id(id: u128) -> Result<String, LegacyEncoderError> {
    Ok(b36(safe_id(id)?))
}
/// Source Encode.encodeAmount() and encodeSend() return amount.toString(36)
/// for integral token units. Nonintegral numbers are scaled to 1e8 and
/// encoded as base36 ATOMS, but the source accidentally omits the "~".
pub fn source_encode_amount_unverified(atoms: u128) -> Result<String, LegacyEncoderError> {
    amount_safe(atoms)?;
    if atoms.is_multiple_of(ATOMS_PER_TOKEN) {
        Ok(b36(atoms / ATOMS_PER_TOKEN))
    } else {
        Ok(b36(atoms))
    }
}

fn code_hash_base36(source_hex: &str) -> Result<String, LegacyEncoderError> {
    // BigInt("0x" + hex).toString(36) supports 256 bits. Reproduce it
    // without third-party crates using repeated division of base256 bytes.
    if source_hex.len() != 64 || !source_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(LegacyEncoderError::InvalidActivationHex);
    }
    let mut bytes: Vec<u8> = source_hex
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let n = std::str::from_utf8(pair).expect("hex ASCII checked");
            u8::from_str_radix(n, 16).expect("hex digit checked")
        })
        .collect();
    let mut output = Vec::new();
    while bytes.iter().any(|b| *b != 0) {
        let mut remainder: u16 = 0;
        for byte in &mut bytes {
            let dividend = (remainder << 8) + u16::from(*byte);
            *byte = (dividend / 36) as u8;
            remainder = dividend % 36;
        }
        output.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[usize::from(remainder)]);
    }
    if output.is_empty() {
        return Ok("0".into());
    }
    output.reverse();
    Ok(String::from_utf8(output).expect("base36 ASCII"))
}

/// The original method encodes txTypeToActivate joined by semicolon and
/// BigInt(hex) as base36. It does not initialize native tokens itself.
/// This narrower diagnostic accepts only canonical 0..35 source type IDs.
pub fn source_encode_activate_unverified(
    type_ids: &[u8],
    hash_hex_256: &str,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    if type_ids.iter().any(|ty| *ty > 35) {
        return Err(LegacyEncoderError::InvalidActivationType);
    }
    let hash = code_hash_base36(hash_hex_256)?;
    let types = type_ids
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(";");
    source_output(
        0,
        format!("tl0{types},{hash}"),
        SourceInnerEnvelope::SingleTypePrefixed,
        vec![SourceEncoderCaveat::HistoricalNativeGenesisUnverified],
    )
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedTokenIssue<'a> {
    pub initial_amount: u128,
    pub ticker: &'a str,
    pub whitelists: &'a [u64],
    pub managed: bool,
    pub backup_address: &'a str,
    pub nft: bool,
    pub colored_coin_hybrid: bool,
    pub procedural_type: Option<u64>,
}
pub fn source_encode_token_issue_unverified(
    c: ClaimedTokenIssue<'_>,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    required_text(c.ticker)?;
    raw_text(c.backup_address)?;
    if c.whitelists.len() > MAX_SOURCE_LIST_IDS {
        return Err(LegacyEncoderError::TooManyListIds);
    }
    let whites = c
        .whitelists
        .iter()
        .map(|n| b36_id(u128::from(*n)))
        .collect::<Result<Vec<_>, _>>()?
        .join(",");
    let procedural = c
        .procedural_type
        .map(|n| b36_id(u128::from(n)))
        .transpose()?
        .unwrap_or_default();
    let raw = format!(
        "tl1{},{},{},{},{},{},{},{}",
        b36_id(c.initial_amount)?,
        c.ticker,
        whites,
        u8::from(c.managed),
        c.backup_address,
        u8::from(c.nft),
        u8::from(c.colored_coin_hybrid),
        procedural
    );
    source_output(
        1,
        raw,
        SourceInnerEnvelope::SingleTypePrefixed,
        if c.whitelists.len() > 1 {
            vec![SourceEncoderCaveat::MultiWhitelistCommaShiftsDecoderFields]
        } else {
            vec![]
        },
    )
}

#[derive(Debug, Clone, Copy)]
pub enum ClaimedSend<'a> {
    SendAll {
        recipient: &'a str,
    },
    Single {
        recipient: &'a str,
        property_id: u64,
        amount_atoms: u128,
    },
    Multi {
        property_ids: &'a [u64],
        amount_atoms: &'a [u128],
    },
}
pub fn source_encode_send_unverified(
    input: ClaimedSend<'_>,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    match input {
        ClaimedSend::SendAll { recipient } => {
            required_text(recipient)?;
            source_output(
                2,
                format!("1;{recipient}"),
                SourceInnerEnvelope::BodyOnly,
                vec![],
            )
        }
        ClaimedSend::Single {
            recipient,
            property_id,
            amount_atoms,
        } => {
            required_text(recipient)?;
            let mut caveats = vec![];
            if !amount_atoms.is_multiple_of(ATOMS_PER_TOKEN) {
                caveats.push(SourceEncoderCaveat::FractionalAmountMissingTilde);
            }
            source_output(
                2,
                format!(
                    "tl20;{recipient};{};{}",
                    b36_id(u128::from(property_id))?,
                    source_encode_amount_unverified(amount_atoms)?
                ),
                SourceInnerEnvelope::SingleTypePrefixed,
                caveats,
            )
        }
        ClaimedSend::Multi {
            property_ids,
            amount_atoms,
        } => {
            if property_ids.len() != amount_atoms.len() {
                return Err(LegacyEncoderError::MismatchedMultiSendArrays);
            }
            if property_ids.len() > MAX_SOURCE_LIST_IDS {
                return Err(LegacyEncoderError::TooManyListIds);
            }
            let ids = property_ids
                .iter()
                .map(|n| b36_id(u128::from(*n)))
                .collect::<Result<Vec<_>, _>>()?
                .join(",");
            let amounts = amount_atoms
                .iter()
                .map(|n| source_encode_amount_unverified(*n))
                .collect::<Result<Vec<_>, _>>()?
                .join(",");
            let mut caveats = vec![SourceEncoderCaveat::MultiSendMisparsedAsScalarByDecoder];
            if amount_atoms
                .iter()
                .any(|n| !n.is_multiple_of(ATOMS_PER_TOKEN))
            {
                caveats.push(SourceEncoderCaveat::FractionalAmountMissingTilde);
            }
            source_output(
                2,
                format!("0;;{ids};{amounts}"),
                SourceInnerEnvelope::BodyOnly,
                caveats,
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ClaimedClearLists<'a> {
    NotSet,
    Scalar(u64),
    Array(&'a [u64]),
}
#[derive(Debug, Clone, Copy)]
pub struct ClaimedCommit<'a> {
    pub property_id: u64,
    pub amount_atoms: u128,
    pub channel_address: &'a str,
    pub ref_vout: u64,
    pub pay_enabled: bool,
    pub clear_lists: ClaimedClearLists<'a>,
    pub colored_output: bool,
    pub commit_clearlist_id: Option<u64>,
}
fn optional_long_ref(address: &str, ref_vout: u64) -> Result<(String, bool), LegacyEncoderError> {
    required_text(address)?;
    if address.len() > 42 {
        Ok((format!("ref:{}", safe_id(u128::from(ref_vout))?), true))
    } else {
        Ok((address.into(), false))
    }
}
pub fn source_encode_commit_unverified(
    c: ClaimedCommit<'_>,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    amount_safe(c.amount_atoms)?;
    let (channel, long) = optional_long_ref(c.channel_address, c.ref_vout)?;
    let lists = match c.clear_lists {
        ClaimedClearLists::NotSet => String::new(),
        ClaimedClearLists::Scalar(n) => b36_id(u128::from(n))?,
        ClaimedClearLists::Array(items) => {
            if items.len() > MAX_SOURCE_LIST_IDS {
                return Err(LegacyEncoderError::TooManyListIds);
            }
            format!(
                "[{}]",
                items
                    .iter()
                    .map(|n| b36_id(u128::from(*n)))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(",")
            )
        }
    };
    let tail = c
        .commit_clearlist_id
        .map(|id| b36_id(u128::from(id)))
        .transpose()?
        .map(|v| format!(",{v}"))
        .unwrap_or_default();
    let mut caveats = vec![SourceEncoderCaveat::GenericTypesEncoderMissingType4Export];
    if matches!(c.clear_lists,ClaimedClearLists::Array(items) if items.len()>1) {
        caveats.push(SourceEncoderCaveat::CommitClearlistCommasBreakDecoder);
    }
    if long {
        caveats.push(SourceEncoderCaveat::LongAddressSubstitutedByCallerClaimedRef);
    }
    source_output(
        4,
        format!(
            "tl4{},{},{channel},{},{},{}{}",
            b36_id(u128::from(c.property_id))?,
            b36(c.amount_atoms),
            u8::from(c.pay_enabled),
            lists,
            u8::from(c.colored_output),
            tail
        ),
        SourceInnerEnvelope::SingleTypePrefixed,
        caveats,
    )
}

pub fn source_encode_create_whitelist_unverified(
    backup: &str,
    name: &str,
    url: &str,
    description: &str,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    for s in [backup, name, url, description] {
        raw_text(s)?;
    }
    source_output(
        7,
        format!("tl7{backup},{name},{url},{description}"),
        SourceInnerEnvelope::SingleTypePrefixed,
        vec![],
    )
}
pub fn source_encode_update_admin_unverified(
    new_address: &str,
    whitelist: bool,
    oracle: bool,
    token: bool,
    id: u64,
    update_backup: bool,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    raw_text(new_address)?;
    source_output(
        8,
        format!(
            "tl8{new_address},{},{},{},{},{}",
            u8::from(whitelist),
            u8::from(oracle),
            u8::from(token),
            b36_id(u128::from(id))?,
            u8::from(update_backup)
        ),
        SourceInnerEnvelope::SingleTypePrefixed,
        vec![],
    )
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRevokeArgument {
    Numeric(bool),
    Boolean(bool),
}
pub fn source_encode_attestation_unverified(
    revoke: SourceRevokeArgument,
    id: u64,
    target: &str,
    metadata: &str,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    raw_text(target)?;
    raw_text(metadata)?;
    let literal = match revoke {
        SourceRevokeArgument::Numeric(true) => "1",
        SourceRevokeArgument::Numeric(false) => "0",
        SourceRevokeArgument::Boolean(true) => "true",
        SourceRevokeArgument::Boolean(false) => "false",
    };
    source_output(
        9,
        format!(
            "tl9{literal},{},{target},{metadata}",
            b36_id(u128::from(id))?
        ),
        SourceInnerEnvelope::SingleTypePrefixed,
        if matches!(revoke, SourceRevokeArgument::Boolean(_)) {
            vec![SourceEncoderCaveat::RawBooleanRevokeDoesNotDecodeAsTrue]
        } else {
            vec![]
        },
    )
}
pub fn source_encode_close_oracle_unverified(
    _oracle_id: u64,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    // Operator precedence: (marker + typeStr?.toString(36)) ?? fallback.
    // The concatenation is never nullish; oracle ID is discarded.
    source_output(
        15,
        "tlf".into(),
        SourceInnerEnvelope::SingleTypePrefixed,
        vec![SourceEncoderCaveat::CloseOracleIgnoresIdAndReturnsPrefixOnly],
    )
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedWithdrawal<'a> {
    pub withdraw_all: bool,
    pub property_id: u64,
    pub amount_atoms: u128,
    pub column: u8,
    pub channel_address: &'a str,
    pub ref_vout: u64,
}
pub fn source_encode_withdrawal_unverified(
    c: ClaimedWithdrawal<'_>,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    amount_safe(c.amount_atoms)?;
    if c.column > 1 {
        return Err(LegacyEncoderError::MissingOrMalformedField);
    }
    let (channel, long) = optional_long_ref(c.channel_address, c.ref_vout)?;
    let mut caveats = vec![
        SourceEncoderCaveat::WithdrawalDecimalAmountVersusBase36Decoder,
        SourceEncoderCaveat::WithdrawalBooleanLiteralVersusOneFlag,
    ];
    if long {
        caveats.push(SourceEncoderCaveat::LongAddressSubstitutedByCallerClaimedRef);
    }
    source_output(
        21,
        format!(
            "tll{},{},{},{},{channel}",
            c.withdraw_all,
            b36_id(u128::from(c.property_id))?,
            c.amount_atoms,
            c.column
        ),
        SourceInnerEnvelope::SingleTypePrefixed,
        caveats,
    )
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedTransfer<'a> {
    pub property_id: u64,
    pub amount_atoms: u128,
    pub is_column_a: bool,
    pub destination_address: &'a str,
    pub ref_vout: u64,
    pub pay: bool,
    pub pay_ref: &'a str,
}
pub fn source_encode_transfer_unverified(
    c: ClaimedTransfer<'_>,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    amount_safe(c.amount_atoms)?;
    raw_text(c.pay_ref)?;
    let (destination, long) = optional_long_ref(c.destination_address, c.ref_vout)?;
    source_output(
        22,
        format!(
            "tlm{},{},{},{destination},{},{}",
            b36_id(u128::from(c.property_id))?,
            b36(c.amount_atoms),
            u8::from(c.is_column_a),
            u8::from(c.pay),
            c.pay_ref
        ),
        SourceInnerEnvelope::SingleTypePrefixed,
        if long {
            vec![SourceEncoderCaveat::LongAddressSubstitutedByCallerClaimedRef]
        } else {
            vec![]
        },
    )
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedStakeFraudRelay<'a> {
    pub action: u64,
    pub oracle_id: u64,
    pub staked_property_id: u64,
    pub amount_atoms: u128,
    pub accused_address: &'a str,
    pub evidence_hash: &'a str,
    pub relay_type: u64,
    pub state_hash: &'a str,
    pub dlc_ref: &'a str,
    pub settlement_state: &'a str,
    pub relay_blob: &'a str,
    pub auto_roll: bool,
    pub next_dlc_ref: &'a str,
}
pub fn source_encode_stake_fraud_relay_unverified(
    c: ClaimedStakeFraudRelay<'_>,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    amount_safe(c.amount_atoms)?;
    for s in [
        c.accused_address,
        c.evidence_hash,
        c.state_hash,
        c.dlc_ref,
        c.settlement_state,
        c.relay_blob,
        c.next_dlc_ref,
    ] {
        raw_text(s)?;
    }
    let fields = vec![
        b36_id(u128::from(c.action))?,
        b36_id(u128::from(c.oracle_id))?,
        b36_id(u128::from(c.staked_property_id))?,
        b36(c.amount_atoms),
        c.accused_address.into(),
        c.evidence_hash.into(),
        b36_id(u128::from(c.relay_type))?,
        c.state_hash.into(),
        c.dlc_ref.into(),
        c.settlement_state.into(),
        c.relay_blob.into(),
        u8::from(c.auto_roll).to_string(),
        c.next_dlc_ref.into(),
    ];
    source_output(
        30,
        format!("tlu{}", fields.join(",")),
        SourceInnerEnvelope::SingleTypePrefixed,
        vec![],
    )
}

pub fn source_encode_colored_coin_unverified(
    action: u8,
    property_id: u64,
    sats_ratio: u64,
    home_address: &str,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    raw_text(home_address)?;
    source_output(
        33,
        format!(
            "{action},{},{},{}",
            b36_id(u128::from(property_id))?,
            b36_id(u128::from(sats_ratio))?,
            home_address
        ),
        SourceInnerEnvelope::BodyOnly,
        vec![SourceEncoderCaveat::BodyOnlyWithoutCalledValidator],
    )
}
pub fn source_encode_bridge_unverified(
    property_id: u64,
    amount: u64,
    destination_address: &str,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    raw_text(destination_address)?;
    source_output(
        34,
        format!(
            "{},{},{}",
            b36_id(u128::from(property_id))?,
            b36_id(u128::from(amount))?,
            destination_address
        ),
        SourceInnerEnvelope::BodyOnly,
        vec![SourceEncoderCaveat::BodyOnlyWithAbsentSourceDecoder],
    )
}
pub fn source_encode_smart_bind_unverified(
    contract_hash: &str,
    contract_type: &str,
    host_chain: &str,
) -> Result<UnverifiedSourceEncoding, LegacyEncoderError> {
    for s in [contract_hash, contract_type, host_chain] {
        raw_text(s)?;
    }
    source_output(
        35,
        format!("{contract_hash},{contract_type},{host_chain}"),
        SourceInnerEnvelope::BodyOnly,
        vec![SourceEncoderCaveat::BodyOnlyWithAbsentSourceDecoder],
    )
}
