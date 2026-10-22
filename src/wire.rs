//! Defensive decoding of the legacy TradeLayer ASCII payload format.
//!
//! This module identifies the envelope and decodes a limited set of historical
//! field layouts. It does not validate transaction authority, activation,
//! address ownership, balances, or consensus state.

use crate::TransactionType;
use core::fmt;

/// Atomic units per fungible token, as used by the legacy send decoder.
pub const ATOMS_PER_TOKEN: u128 = 100_000_000;

/// Defensive ceiling for standalone ASCII payload parsing.
/// This is an application limit, not a claim about an on-chain OP_RETURN limit.
pub const MAX_PAYLOAD_BYTES: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    InvalidMarker,
    InvalidType,
    NonPrintableAscii,
    TooLong,
    InvalidField(&'static str),
    NumberOverflow,
    NestedEnvelope,
    UnsupportedType(u8),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMarker => write!(f, "payload must start with 'tl'"),
            Self::InvalidType => write!(f, "missing or invalid transaction type"),
            Self::NonPrintableAscii => write!(f, "payload contains non-printable ASCII or Unicode"),
            Self::TooLong => write!(f, "payload exceeds defensive size limit"),
            Self::InvalidField(name) => write!(f, "invalid field: {name}"),
            Self::NumberOverflow => write!(f, "base-36 number exceeds supported integer range"),
            Self::NestedEnvelope => write!(f, "ambiguous nested tl/type prefix"),
            Self::UnsupportedType(id) => write!(f, "transaction type {id} has no typed parser"),
        }
    }
}

impl std::error::Error for WireError {}

/// Borrowed view of the ASCII envelope. The payload has not been validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    pub tx_type: TransactionType,
    pub body: &'a str,
}

/// Parses `tl` + a single base-36 transaction type character + opaque body.
///
/// The JavaScript block processor uses the first character after `tl` as
/// the transaction selector. Both upper and lower-case letters are accepted
/// here to match JavaScript's `parseInt(..., 36)` behavior.
pub fn parse_frame(raw: &str) -> Result<Frame<'_>, WireError> {
    if raw.len() > MAX_PAYLOAD_BYTES {
        return Err(WireError::TooLong);
    }
    if !raw.is_ascii() || !raw.bytes().all(|c| (0x20..=0x7e).contains(&c)) {
        return Err(WireError::NonPrintableAscii);
    }
    if !raw.starts_with("tl") {
        return Err(WireError::InvalidMarker);
    }
    let selector = raw.as_bytes().get(2).ok_or(WireError::InvalidType)?;
    let number = base36_digit(*selector).ok_or(WireError::InvalidType)?;
    let tx_type = TransactionType::try_from(number).map_err(|_| WireError::InvalidType)?;
    Ok(Frame {
        tx_type,
        body: &raw[3..],
    })
}

fn base36_digit(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'z' => Some(ch - b'a' + 10),
        b'A'..=b'Z' => Some(ch - b'A' + 10),
        _ => None,
    }
}

/// Strict, whole-field base-36 integer parsing with checked arithmetic.
/// JavaScript `parseInt` is more permissive and loses precision for large values.
pub fn parse_base36(raw: &str) -> Result<u128, WireError> {
    if raw.is_empty() {
        return Err(WireError::InvalidField("empty base-36 integer"));
    }
    let mut value = 0u128;
    for ch in raw.bytes() {
        let digit = u128::from(base36_digit(ch).ok_or(WireError::InvalidField("base-36 integer"))?);
        value = value
            .checked_mul(36)
            .and_then(|v| v.checked_add(digit))
            .ok_or(WireError::NumberOverflow)?;
    }
    Ok(value)
}

fn property_id(raw: &str) -> Result<u32, WireError> {
    u32::try_from(parse_base36(raw)?).map_err(|_| WireError::NumberOverflow)
}

fn boolean(raw: &str) -> Result<bool, WireError> {
    match raw {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(WireError::InvalidField("boolean flag")),
    }
}

/// Decode the `src/txDecoder.js::decodeAmount` bimodal send convention:
/// whole token units use base-36 without suffix; atomic units end with `~`.
/// The historical encoder sometimes omits `~` on fractional inputs.
pub fn parse_send_amount(raw: &str) -> Result<u128, WireError> {
    match raw.strip_suffix('~') {
        Some(atoms) => parse_base36(atoms),
        None => parse_base36(raw)?
            .checked_mul(ATOMS_PER_TOKEN)
            .ok_or(WireError::NumberOverflow),
    }
}

fn fields(raw: &str, delimiter: char, count: usize) -> Result<Vec<&str>, WireError> {
    let parts: Vec<_> = raw.split(delimiter).collect();
    if parts.len() != count {
        return Err(WireError::InvalidField("field count"));
    }
    Ok(parts)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedTransaction {
    Activate {
        types: Vec<TransactionType>,
        code_hash_base36: String,
    },
    Issue {
        initial_tokens: u128,
        ticker: String,
        whitelist_raw: String,
        managed: bool,
        backup_address: String,
        nft: bool,
        colored_coin_hybrid: bool,
        procedural_type: Option<u32>,
    },
    Send {
        recipient: String,
        property_id: u32,
        amount_atoms: u128,
    },
    SendAll {
        recipient: String,
    },
    Grant {
        property_id: u32,
        amount_atoms: u128,
        recipient: String,
        legacy_hash: String,
        dlc_template_id: String,
        dlc_contract_id: String,
        settlement_state: String,
        dlc_hash: String,
    },
    Redeem {
        property_id: u32,
        amount_atoms: u128,
        dlc_template_id: String,
        dlc_contract_id: String,
        settlement_state: String,
    },
}

/// Parse a small, explicitly selected subset of the legacy wire format.
///
/// The result contains syntactically decoded fields only. Consensus checks,
/// integer-range compatibility with JavaScript, and application semantics
/// belong to later migration stages.
pub fn parse_typed(raw: &str) -> Result<ParsedTransaction, WireError> {
    let frame = parse_frame(raw)?;
    parse_body(frame.tx_type, frame.body)
}

/// Parse a body after its marker and transaction type were separated.
pub fn parse_body(tx_type: TransactionType, body: &str) -> Result<ParsedTransaction, WireError> {
    if body.len() > MAX_PAYLOAD_BYTES {
        return Err(WireError::TooLong);
    }
    if !body.is_ascii() || !body.bytes().all(|c| (0x20..=0x7e).contains(&c)) {
        return Err(WireError::NonPrintableAscii);
    }

    // Some JS callers append an already-framed result to `tl<type>`.
    // This creates a genuine ambiguity with long base-36 numeric fields.
    // Defer those payloads for explicit protocol review.
    if let Some(tail) = body.strip_prefix("tl") {
        if let Some(selector) = tail.bytes().next() {
            if base36_digit(selector) == Some(tx_type.as_u8()) {
                return Err(WireError::NestedEnvelope);
            }
        }
    }

    match tx_type.as_u8() {
        0 => {
            let f = fields(body, ',', 2)?;
            if f[1].is_empty() || f[1].bytes().any(|c| base36_digit(c).is_none()) {
                return Err(WireError::InvalidField("activation code hash"));
            }
            let types = f[0]
                .split(';')
                .map(|raw| {
                    let value = raw
                        .parse::<u8>()
                        .map_err(|_| WireError::InvalidField("decimal activation type"))?;
                    TransactionType::try_from(value)
                        .map_err(|_| WireError::InvalidField("activation type out of range"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ParsedTransaction::Activate {
                types,
                code_hash_base36: f[1].to_owned(),
            })
        }
        1 => {
            let f = fields(body, ',', 8)?;
            Ok(ParsedTransaction::Issue {
                initial_tokens: parse_base36(f[0])?,
                ticker: f[1].to_owned(),
                whitelist_raw: f[2].to_owned(),
                managed: boolean(f[3])?,
                backup_address: f[4].to_owned(),
                nft: boolean(f[5])?,
                colored_coin_hybrid: boolean(f[6])?,
                procedural_type: if f[7].is_empty() {
                    None
                } else {
                    Some(property_id(f[7])?)
                },
            })
        }
        2 => {
            let f: Vec<_> = body.split(';').collect();
            match f.as_slice() {
                ["1", recipient] => Ok(ParsedTransaction::SendAll {
                    recipient: (*recipient).to_owned(),
                }),
                ["0", recipient, id, amount] => Ok(ParsedTransaction::Send {
                    recipient: (*recipient).to_owned(),
                    property_id: property_id(id)?,
                    amount_atoms: parse_send_amount(amount)?,
                }),
                _ => Err(WireError::InvalidField("send field layout")),
            }
        }
        11 => {
            let f = fields(body, ',', 8)?;
            Ok(ParsedTransaction::Grant {
                property_id: property_id(f[0])?,
                amount_atoms: parse_base36(f[1])?,
                recipient: f[2].to_owned(),
                legacy_hash: f[3].to_owned(),
                dlc_template_id: f[4].to_owned(),
                dlc_contract_id: f[5].to_owned(),
                settlement_state: f[6].to_owned(),
                dlc_hash: f[7].to_owned(),
            })
        }
        12 => {
            let f = fields(body, ',', 5)?;
            Ok(ParsedTransaction::Redeem {
                property_id: property_id(f[0])?,
                amount_atoms: parse_base36(f[1])?,
                dlc_template_id: f[2].to_owned(),
                dlc_contract_id: f[3].to_owned(),
                settlement_state: f[4].to_owned(),
            })
        }
        other => Err(WireError::UnsupportedType(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_base36_and_amounts() {
        assert_eq!(parse_base36("z"), Ok(35));
        assert_eq!(parse_base36("10"), Ok(36));
        assert_eq!(parse_base36("ABC"), Ok(13_368));
        assert_eq!(parse_send_amount("2"), Ok(200_000_000));
        assert_eq!(parse_send_amount("1~"), Ok(1));
        assert!(parse_base36("-1").is_err());
        assert!(parse_base36("1z$").is_err());
        assert_eq!(
            parse_base36("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
            Err(WireError::NumberOverflow)
        );
        assert_eq!(
            parse_send_amount("~"),
            Err(WireError::InvalidField("empty base-36 integer"))
        );
    }

    #[test]
    fn reject_bad_envelopes() {
        for input in [
            "",
            "tl",
            "om2a",
            "tl?x",
            "tl\n2x",
            "tl\u{1f642}",
            "tL20;x;1;1",
        ] {
            assert!(parse_frame(input).is_err(), "{input:?}");
        }
        assert_eq!(
            parse_frame(&"x".repeat(MAX_PAYLOAD_BYTES + 1)),
            Err(WireError::TooLong)
        );
        assert_eq!(parse_frame("tlAfoo").unwrap().tx_type.as_u8(), 10);
    }

    #[test]
    fn a_single_atomic_unit_is_exact() {
        let tx = parse_typed("tl20;alice;5;1~").unwrap();
        assert_eq!(
            tx,
            ParsedTransaction::Send {
                recipient: "alice".into(),
                property_id: 5,
                amount_atoms: 1,
            }
        );
    }

    #[test]
    fn fractional_encoder_omission_changes_semantics() {
        let tx = parse_typed("tl20;alice;5;1").unwrap();
        assert_eq!(
            tx,
            ParsedTransaction::Send {
                recipient: "alice".into(),
                property_id: 5,
                amount_atoms: ATOMS_PER_TOKEN,
            }
        );
    }

    #[test]
    fn activation_hash_supports_256_bit_values() {
        let base36_hash = "z".repeat(50);
        let wire = format!("tl00,{base36_hash}");
        let tx = parse_typed(&wire).unwrap();
        match tx {
            ParsedTransaction::Activate { code_hash_base36, .. } => {
                assert_eq!(code_hash_base36, base36_hash);
            }
            other => panic!("expected activation, got {other:?}"),
        }
    }

    #[test]
    fn reject_ambiguous_wrapper() {
        assert_eq!(
            parse_typed("tl2tl20;alice;5;2"),
            Err(WireError::NestedEnvelope)
        );
        assert_eq!(
            parse_typed("tl1tl1a,FOO,,1,,0,0,"),
            Err(WireError::NestedEnvelope)
        );
    }
}
