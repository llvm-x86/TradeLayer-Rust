//! Pure builders for the direct legacy `src/txEncoder.js` payloads of types
//! 2 (simple send), 4 (commit to channel) and 9 (attestation).
//!
//! Each builder returns the single-prefix ASCII string that the JavaScript
//! encoder returns for that layout. The generic `Types.encodePayload` wrapper
//! adds a second prefix and is deliberately not reproduced.
//!
//! Byte parity with the JavaScript encoder is claimed only for the rows
//! marked `match` in `fixtures/payload_builder_js_vectors.tsv`. Every other
//! row is a documented divergence (see `docs/PAYLOAD_BUILDERS.md`), where this
//! module either emits a safer decodable form or refuses with an explicit
//! error. Withdrawal-from-channel (type 21) has no builder: the JavaScript
//! encoder writes fields that its own decoder reads differently.
//!
//! Not a transaction constructor: no OP_RETURN, signature, activation,
//! balance, or authority check happens here.

use crate::wire::MAX_PAYLOAD_BYTES;
use core::fmt;

/// Atomic units per token in the legacy amount convention.
pub const ATOMS_PER_TOKEN: u64 = 100_000_000;

/// Largest atomic amount that the legacy JavaScript decoders read exactly
/// (2^53 - 1). Larger values are refused rather than silently misread.
pub const MAX_LEGACY_SAFE_ATOMS: u64 = 9_007_199_254_740_991;

/// Longest channel address written inline. Longer channels must use `ref:<n>`.
pub const MAX_INLINE_CHANNEL_BYTES: usize = 42;

/// Defensive cap for recipient and attestation target fields.
pub const MAX_ADDRESS_BYTES: usize = 90;

const SEND_TYPE: char = '2';
const COMMIT_TYPE: char = '4';
const ATTEST_TYPE: char = '9';

/// Why a payload could not be built. Variant names are stable and are used by
/// the fixture-driven tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PayloadError {
    EmptyField(&'static str),
    FieldTooLong { field: &'static str, max: usize },
    ForbiddenByte { field: &'static str, byte: u8 },
    ReservedReferencePrefix,
    ChannelAddressRequiresReference,
    MetadataNotUppercaseStable,
    AmbiguousFramedBody,
    PayloadTooLong,
    InvalidAmount,
    NonIntegerAmount,
    NegativeAmount,
    AmountOverflow,
    AmountExceedsLegacyPrecision,
}

impl fmt::Display for PayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "{field} must not be empty"),
            Self::FieldTooLong { field, max } => write!(f, "{field} exceeds {max} bytes"),
            Self::ForbiddenByte { field, byte } => {
                write!(f, "{field} contains forbidden byte 0x{byte:02x}")
            }
            Self::ReservedReferencePrefix => write!(f, "channel address must not start with ref:"),
            Self::ChannelAddressRequiresReference => write!(
                f,
                "channel address longer than {MAX_INLINE_CHANNEL_BYTES} bytes requires an explicit reference"
            ),
            Self::MetadataNotUppercaseStable => {
                write!(f, "metadata must not contain lowercase ASCII letters")
            }
            Self::AmbiguousFramedBody => {
                write!(f, "body begins with tl and cannot be decoded unambiguously")
            }
            Self::PayloadTooLong => write!(f, "payload exceeds {MAX_PAYLOAD_BYTES} bytes"),
            Self::InvalidAmount => write!(f, "amount must be a decimal integer"),
            Self::NonIntegerAmount => write!(f, "amount must be an integer count of atomic units"),
            Self::NegativeAmount => write!(f, "amount must not be negative"),
            Self::AmountOverflow => write!(f, "amount exceeds u64"),
            Self::AmountExceedsLegacyPrecision => write!(
                f,
                "amount exceeds {MAX_LEGACY_SAFE_ATOMS} atomic units, the largest the legacy decoders read exactly"
            ),
        }
    }
}

impl std::error::Error for PayloadError {}

/// Parses a decimal atomic-unit amount from text.
///
/// Rejects floats (`.`, exponents), negative values, signs, whitespace, and
/// values above `u64::MAX`. Leading zeros are accepted.
pub fn parse_atomic_amount(text: &str) -> Result<u64, PayloadError> {
    if text.is_empty() {
        return Err(PayloadError::InvalidAmount);
    }
    if text.starts_with('-') {
        return Err(PayloadError::NegativeAmount);
    }
    if text.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
        return Err(PayloadError::NonIntegerAmount);
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PayloadError::InvalidAmount);
    }
    text.parse::<u64>()
        .map_err(|_| PayloadError::AmountOverflow)
}

/// Builds the direct simple-send payload `tl2 0;<recipient>;<property>;<amount>`.
///
/// Whole-token amounts are written as a base-36 token count, as the JavaScript
/// encoder does. Fractional amounts are written as base-36 atomic units with a
/// trailing `~`, which the legacy decoder reads as atomic units. The JavaScript
/// encoder omits the `~`, see the fixture.
pub fn build_simple_send(
    recipient: &str,
    property_id: u32,
    amount_atoms: u64,
) -> Result<String, PayloadError> {
    check_legacy_amount(amount_atoms)?;
    check_text("recipient", recipient, MAX_ADDRESS_BYTES, b",;")?;
    let amount = if amount_atoms.is_multiple_of(ATOMS_PER_TOKEN) {
        to_base36(amount_atoms / ATOMS_PER_TOKEN)
    } else {
        format!("{}~", to_base36(amount_atoms))
    };
    let property = to_base36(u64::from(property_id));
    finish(SEND_TYPE, format!("0;{recipient};{property};{amount}"))
}

/// Builds the direct attestation payload `tl9 <revoke>,<list>,<target>,<metadata>`.
///
/// `revoke` is written as `1` or `0`. The JavaScript encoder writes a Boolean
/// as `true` or `false`, which the decoder does not read as a revoke; see the
/// fixture. Metadata must be upper-case stable because the legacy decoder
/// upper-cases it.
pub fn build_attestation(
    revoke: bool,
    list_id: u32,
    target: &str,
    metadata: &str,
) -> Result<String, PayloadError> {
    check_text("target", target, MAX_ADDRESS_BYTES, b",;")?;
    check_metadata(metadata)?;
    let flag = flag(revoke);
    let list = to_base36(u64::from(list_id));
    finish(ATTEST_TYPE, format!("{flag},{list},{target},{metadata}"))
}

/// Channel destination of a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelLocation<'a> {
    /// Inline channel address, at most [`MAX_INLINE_CHANNEL_BYTES`] bytes.
    Address(&'a str),
    /// Explicit `ref:<n>` index, for channels too long to write inline.
    Reference(u32),
}

/// Clearlist field of a commit. The legacy array form `[a,b]` is not
/// representable: its decoder splits on commas before parsing brackets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearLists {
    Absent,
    Single(u32),
}

/// Inputs of a commit to channel (legacy type 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitToChannel<'a> {
    pub property_id: u32,
    pub amount_atoms: u64,
    pub channel: ChannelLocation<'a>,
    pub pay_enabled: bool,
    pub clear_lists: ClearLists,
    pub colored_output: bool,
    pub commit_clearlist_id: Option<u32>,
}

/// Builds the direct commit payload
/// `tl4 <property>,<amount>,<channel>,<pay>,<clearlists>,<colored>[,<clearlist>]`.
pub fn build_commit_to_channel(commit: &CommitToChannel<'_>) -> Result<String, PayloadError> {
    check_legacy_amount(commit.amount_atoms)?;
    let channel = match commit.channel {
        ChannelLocation::Address(address) => {
            check_channel_address(address)?;
            address.to_owned()
        }
        ChannelLocation::Reference(index) => format!("ref:{index}"),
    };
    let clear_lists = match commit.clear_lists {
        ClearLists::Absent => String::new(),
        ClearLists::Single(id) => to_base36(u64::from(id)),
    };
    let mut fields = vec![
        to_base36(u64::from(commit.property_id)),
        to_base36(commit.amount_atoms),
        channel,
        flag(commit.pay_enabled).to_string(),
        clear_lists,
        flag(commit.colored_output).to_string(),
    ];
    if let Some(id) = commit.commit_clearlist_id {
        fields.push(to_base36(u64::from(id)));
    }
    finish(COMMIT_TYPE, fields.join(","))
}

fn check_legacy_amount(atoms: u64) -> Result<(), PayloadError> {
    if atoms > MAX_LEGACY_SAFE_ATOMS {
        Err(PayloadError::AmountExceedsLegacyPrecision)
    } else {
        Ok(())
    }
}

/// Printable ASCII, non-empty, bounded length, and no delimiter bytes.
fn check_text(
    field: &'static str,
    text: &str,
    max: usize,
    forbidden: &[u8],
) -> Result<(), PayloadError> {
    if text.is_empty() {
        return Err(PayloadError::EmptyField(field));
    }
    if text.len() > max {
        return Err(PayloadError::FieldTooLong { field, max });
    }
    for &byte in text.as_bytes() {
        if !(0x21..=0x7e).contains(&byte) || forbidden.contains(&byte) {
            return Err(PayloadError::ForbiddenByte { field, byte });
        }
    }
    Ok(())
}

fn check_channel_address(address: &str) -> Result<(), PayloadError> {
    if address.len() > MAX_INLINE_CHANNEL_BYTES {
        return Err(PayloadError::ChannelAddressRequiresReference);
    }
    if address.starts_with("ref:") {
        return Err(PayloadError::ReservedReferencePrefix);
    }
    check_text("channel", address, MAX_INLINE_CHANNEL_BYTES, b",;")
}

fn check_metadata(metadata: &str) -> Result<(), PayloadError> {
    for &byte in metadata.as_bytes() {
        if !(0x20..=0x7e).contains(&byte) || byte == b',' {
            return Err(PayloadError::ForbiddenByte {
                field: "metadata",
                byte,
            });
        }
    }
    if metadata.bytes().any(|b| b.is_ascii_lowercase()) {
        return Err(PayloadError::MetadataNotUppercaseStable);
    }
    Ok(())
}

fn finish(type_id: char, body: String) -> Result<String, PayloadError> {
    if body.starts_with("tl") {
        return Err(PayloadError::AmbiguousFramedBody);
    }
    let payload = format!("tl{type_id}{body}");
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(PayloadError::PayloadTooLong);
    }
    Ok(payload)
}

const fn flag(value: bool) -> char {
    if value {
        '1'
    } else {
        '0'
    }
}

/// Lower-case base-36 digits, matching JavaScript `Number.prototype.toString(36)`.
fn to_base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return String::from("0");
    }
    let mut digits = Vec::new();
    while value > 0 {
        digits.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    digits.iter().rev().map(|&d| char::from(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base36_matches_javascript_radix_conversion() {
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
        assert_eq!(to_base36(u64::from(u32::MAX)), "1z141z3");
        assert_eq!(to_base36(u64::MAX), "3w5e11264sgsf");
    }

    #[test]
    fn amounts_reject_floats_negatives_and_overflow() {
        assert_eq!(parse_atomic_amount("0"), Ok(0));
        assert_eq!(parse_atomic_amount("007"), Ok(7));
        assert_eq!(parse_atomic_amount(&u64::MAX.to_string()), Ok(u64::MAX));
        assert_eq!(
            parse_atomic_amount("1.5"),
            Err(PayloadError::NonIntegerAmount)
        );
        assert_eq!(
            parse_atomic_amount("1e8"),
            Err(PayloadError::NonIntegerAmount)
        );
        assert_eq!(parse_atomic_amount("-1"), Err(PayloadError::NegativeAmount));
        assert_eq!(parse_atomic_amount("+1"), Err(PayloadError::InvalidAmount));
        assert_eq!(parse_atomic_amount(""), Err(PayloadError::InvalidAmount));
        assert_eq!(
            parse_atomic_amount("18446744073709551616"),
            Err(PayloadError::AmountOverflow)
        );
    }

    #[test]
    fn legacy_precision_boundary_is_exact() {
        assert!(build_simple_send("alice", 1, MAX_LEGACY_SAFE_ATOMS).is_ok());
        assert_eq!(
            build_simple_send("alice", 1, MAX_LEGACY_SAFE_ATOMS + 1),
            Err(PayloadError::AmountExceedsLegacyPrecision)
        );
        assert_eq!(
            build_simple_send("alice", 1, u64::MAX),
            Err(PayloadError::AmountExceedsLegacyPrecision)
        );
    }

    fn inline_commit(address: &str) -> CommitToChannel<'_> {
        CommitToChannel {
            property_id: 7,
            amount_atoms: ATOMS_PER_TOKEN,
            channel: ChannelLocation::Address(address),
            pay_enabled: false,
            clear_lists: ClearLists::Absent,
            colored_output: false,
            commit_clearlist_id: None,
        }
    }

    #[test]
    fn channel_reference_rules_are_explicit() {
        let long = "C".repeat(MAX_INLINE_CHANNEL_BYTES + 1);
        assert_eq!(
            build_commit_to_channel(&inline_commit(&long)),
            Err(PayloadError::ChannelAddressRequiresReference)
        );
        assert_eq!(
            build_commit_to_channel(&inline_commit("ref:9")),
            Err(PayloadError::ReservedReferencePrefix)
        );
        assert_eq!(
            build_commit_to_channel(&inline_commit("")),
            Err(PayloadError::EmptyField("channel"))
        );
        let referenced = CommitToChannel {
            channel: ChannelLocation::Reference(0),
            ..inline_commit("")
        };
        assert_eq!(
            build_commit_to_channel(&referenced),
            Ok("tl47,1njchs,ref:0,0,,0".to_owned())
        );
    }

    #[test]
    fn framed_commit_body_is_refused() {
        let commit = CommitToChannel {
            property_id: 1065,
            amount_atoms: ATOMS_PER_TOKEN,
            channel: ChannelLocation::Address("chan"),
            pay_enabled: false,
            clear_lists: ClearLists::Single(7),
            colored_output: false,
            commit_clearlist_id: None,
        };
        assert_eq!(
            build_commit_to_channel(&commit),
            Err(PayloadError::AmbiguousFramedBody)
        );
    }

    #[test]
    fn attestation_metadata_rules_are_explicit() {
        assert_eq!(
            build_attestation(false, 7, "alice", "a,b"),
            Err(PayloadError::ForbiddenByte {
                field: "metadata",
                byte: b','
            })
        );
        assert_eq!(
            build_attestation(false, 7, "alice", "ok"),
            Err(PayloadError::MetadataNotUppercaseStable)
        );
        assert_eq!(
            build_attestation(true, 7, "alice", "OK"),
            Ok("tl91,7,alice,OK".to_owned())
        );
    }
}
