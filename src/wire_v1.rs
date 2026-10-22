//! Proposed version-1 TradeLayer binary wire envelope and basic sends.
//!
//! A distinct uppercase magic prevents accidental interpretation by the
//! legacy JavaScript scanner, which looks for the lowercase ASCII `tl` marker.
//! This is a proposed format only: no activation height, consensus transition,
//! chain parameter, signature, or address verification is implemented here.
//!
//! All integers are binary and big-endian. In particular, transaction types
//! are encoded as one unsigned byte, never as base-36 text.

use crate::TransactionType;
use core::fmt;

const MAGIC: &[u8; 3] = b"TLR";
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 5;
pub const MAX_FRAME_BYTES: usize = 4_096;
pub const MAX_ADDRESS_BYTES: usize = 90;
pub const SIMPLE_SEND_TYPE: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireV1Error {
    Truncated,
    TooLong,
    InvalidMagic,
    UnsupportedVersion(u8),
    InvalidTransactionType(u8),
    WrongTransactionType(u8),
    InvalidSendVariant(u8),
    InvalidPropertyId,
    InvalidAmount,
    InvalidRecipient,
    TrailingBytes,
}

impl fmt::Display for WireV1Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated version-1 wire message"),
            Self::TooLong => write!(f, "version-1 wire message exceeds parser limit"),
            Self::InvalidMagic => write!(f, "invalid version-1 wire marker"),
            Self::UnsupportedVersion(value) => write!(f, "unsupported wire version {value}"),
            Self::InvalidTransactionType(value) => write!(f, "unknown transaction type {value}"),
            Self::WrongTransactionType(value) => write!(f, "expected send type 2, got {value}"),
            Self::InvalidSendVariant(value) => write!(f, "unknown send variant {value}"),
            Self::InvalidPropertyId => write!(f, "property ID must be nonzero"),
            Self::InvalidAmount => write!(f, "send amount must be positive atomic units"),
            Self::InvalidRecipient => write!(f, "invalid ASCII recipient identifier"),
            Self::TrailingBytes => write!(f, "unexpected trailing bytes in send body"),
        }
    }
}

impl std::error::Error for WireV1Error {}

/// A syntactically decoded v1 envelope. Its body remains unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    pub tx_type: TransactionType,
    pub body: &'a [u8],
}

/// Decode a binary envelope: `TLR` + version (1) + one numeric type byte + body.
/// This function never falls back to legacy base-36 decoding.
pub fn parse_frame(bytes: &[u8]) -> Result<Frame<'_>, WireV1Error> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(WireV1Error::TooLong);
    }
    if bytes.len() < HEADER_LEN {
        return Err(WireV1Error::Truncated);
    }
    if &bytes[..3] != MAGIC {
        return Err(WireV1Error::InvalidMagic);
    }
    if bytes[3] != VERSION {
        return Err(WireV1Error::UnsupportedVersion(bytes[3]));
    }
    let tx_type = TransactionType::try_from(bytes[4])
        .map_err(|_| WireV1Error::InvalidTransactionType(bytes[4]))?;
    Ok(Frame {
        tx_type,
        body: &bytes[HEADER_LEN..],
    })
}

/// Construct exactly one binary envelope for any known transaction type.
/// Encoding here does not imply the body is valid for that transaction type.
pub fn encode_frame(
    tx_type: TransactionType,
    body: &[u8],
) -> Result<Vec<u8>, WireV1Error> {
    if body.len() > MAX_FRAME_BYTES - HEADER_LEN {
        return Err(WireV1Error::TooLong);
    }
    let mut bytes = Vec::with_capacity(HEADER_LEN + body.len());
    bytes.extend_from_slice(MAGIC);
    bytes.push(VERSION);
    bytes.push(tx_type.as_u8());
    bytes.extend_from_slice(body);
    Ok(bytes)
}

/// Transaction type 2 in the proposed v1 format.
///
/// Direct send body: `0x00` + u32 property ID + u64 atomic amount +
/// u8 recipient length + printable non-space ASCII recipient bytes.
/// Send-all body: `0x01` + u8 recipient length + recipient bytes.
/// This layer checks syntax and amount ranges only; wallet or consensus
/// authorization must be performed separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Send<'a> {
    Direct {
        property_id: u32,
        amount_atoms: u64,
        recipient: &'a str,
    },
    All {
        recipient: &'a str,
    },
}

fn recipient_bytes(recipient: &str) -> Result<&[u8], WireV1Error> {
    let bytes = recipient.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_ADDRESS_BYTES
        || !bytes.iter().all(|&b| (0x21..=0x7e).contains(&b))
    {
        return Err(WireV1Error::InvalidRecipient);
    }
    Ok(bytes)
}

fn decode_recipient(body: &[u8], length_index: usize) -> Result<&str, WireV1Error> {
    let declared = *body.get(length_index).ok_or(WireV1Error::Truncated)? as usize;
    if declared == 0 || declared > MAX_ADDRESS_BYTES {
        return Err(WireV1Error::InvalidRecipient);
    }
    let start = length_index + 1;
    let end = start + declared;
    let remaining = body.get(start..end).ok_or(WireV1Error::Truncated)?;
    if body.len() > end {
        return Err(WireV1Error::TrailingBytes);
    }
    let recipient = core::str::from_utf8(remaining).map_err(|_| WireV1Error::InvalidRecipient)?;
    recipient_bytes(recipient)?;
    Ok(recipient)
}

/// Encode a type-2 body and wrap it in the canonical v1 header.
pub fn encode_send(send: Send<'_>) -> Result<Vec<u8>, WireV1Error> {
    let mut body = Vec::new();
    match send {
        Send::Direct {
            property_id,
            amount_atoms,
            recipient,
        } => {
            if property_id == 0 {
                return Err(WireV1Error::InvalidPropertyId);
            }
            if amount_atoms == 0 {
                return Err(WireV1Error::InvalidAmount);
            }
            let to = recipient_bytes(recipient)?;
            body.reserve(14 + to.len());
            body.push(0);
            body.extend_from_slice(&property_id.to_be_bytes());
            body.extend_from_slice(&amount_atoms.to_be_bytes());
            body.push(u8::try_from(to.len()).map_err(|_| WireV1Error::InvalidRecipient)?);
            body.extend_from_slice(to);
        }
        Send::All { recipient } => {
            let to = recipient_bytes(recipient)?;
            body.reserve(2 + to.len());
            body.push(1);
            body.push(u8::try_from(to.len()).map_err(|_| WireV1Error::InvalidRecipient)?);
            body.extend_from_slice(to);
        }
    }
    let send_type = TransactionType::try_from(SIMPLE_SEND_TYPE)
        .map_err(|_| WireV1Error::InvalidTransactionType(SIMPLE_SEND_TYPE))?;
    encode_frame(send_type, &body)
}

/// Parse a type-2 body after its v1 envelope has been checked.
pub fn parse_send_body(body: &[u8]) -> Result<Send<'_>, WireV1Error> {
    let discriminator = *body.first().ok_or(WireV1Error::Truncated)?;
    match discriminator {
        0 => {
            let pid_bytes: [u8; 4] = body
                .get(1..5)
                .ok_or(WireV1Error::Truncated)?
                .try_into()
                .map_err(|_| WireV1Error::Truncated)?;
            let amount_bytes: [u8; 8] = body
                .get(5..13)
                .ok_or(WireV1Error::Truncated)?
                .try_into()
                .map_err(|_| WireV1Error::Truncated)?;
            let property_id = u32::from_be_bytes(pid_bytes);
            let amount_atoms = u64::from_be_bytes(amount_bytes);
            if property_id == 0 {
                return Err(WireV1Error::InvalidPropertyId);
            }
            if amount_atoms == 0 {
                return Err(WireV1Error::InvalidAmount);
            }
            let recipient = decode_recipient(body, 13)?;
            Ok(Send::Direct {
                property_id,
                amount_atoms,
                recipient,
            })
        }
        1 => Ok(Send::All {
            recipient: decode_recipient(body, 1)?,
        }),
        value => Err(WireV1Error::InvalidSendVariant(value)),
    }
}

/// Parse a full type-2 v1 message; no implicit legacy compatibility path exists.
pub fn parse_send(bytes: &[u8]) -> Result<Send<'_>, WireV1Error> {
    let frame = parse_frame(bytes)?;
    if frame.tx_type.as_u8() != SIMPLE_SEND_TYPE {
        return Err(WireV1Error::WrongTransactionType(frame.tx_type.as_u8()));
    }
    parse_send_body(frame.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_type_is_a_byte_and_never_base36() {
        for value in 0u8..=35 {
            let tx_type = TransactionType::try_from(value).unwrap();
            let bytes = encode_frame(tx_type, b"sample").unwrap();
            assert_eq!(&bytes[..4], b"TLR\x01");
            assert_eq!(bytes[4], value);
            let frame = parse_frame(&bytes).unwrap();
            assert_eq!(frame.tx_type, tx_type);
            assert_eq!(frame.body, b"sample");
        }
        assert_eq!(
            parse_frame(b"TLR\x01\x24"),
            Err(WireV1Error::InvalidTransactionType(36))
        );
    }

    #[test]
    fn legacy_ascii_cannot_be_a_v1_message() {
        assert_eq!(
            parse_frame(b"tl20;alice;5;2"),
            Err(WireV1Error::InvalidMagic)
        );
        assert_eq!(
            parse_frame(b"tl2tl20;alice;5;2"),
            Err(WireV1Error::InvalidMagic)
        );
        assert_eq!(
            parse_frame(b"TLR\x02\x02"),
            Err(WireV1Error::UnsupportedVersion(2))
        );
    }

    #[test]
    fn direct_send_is_exact_at_single_atom() {
        let input = Send::Direct {
            property_id: 5,
            amount_atoms: 1,
            recipient: "alice",
        };
        let bytes = encode_send(input).unwrap();
        assert_eq!(parse_send(&bytes), Ok(input));
        assert_eq!(
            bytes.as_slice(),
            &b"TLR\x01\x02\x00\x00\x00\x00\x05\x00\x00\x00\x00\x00\x00\x00\x01\x05alice"[..]
        );
    }

    #[test]
    fn send_all_is_distinct_and_round_trips() {
        let input = Send::All { recipient: "bob" };
        let bytes = encode_send(input).unwrap();
        assert_eq!(parse_send(&bytes), Ok(input));
        assert_eq!(bytes.as_slice(), &b"TLR\x01\x02\x01\x03bob"[..]);
    }

    #[test]
    fn reject_zero_amount_and_property() {
        assert_eq!(
            encode_send(Send::Direct {
                property_id: 0,
                amount_atoms: 2,
                recipient: "alice",
            }),
            Err(WireV1Error::InvalidPropertyId)
        );
        assert_eq!(
            encode_send(Send::Direct {
                property_id: 5,
                amount_atoms: 0,
                recipient: "alice",
            }),
            Err(WireV1Error::InvalidAmount)
        );
    }

    #[test]
    fn never_ignore_trailing_bytes() {
        let mut encoded = encode_send(Send::All { recipient: "bob" }).unwrap();
        encoded.push(0);
        assert_eq!(parse_send(&encoded), Err(WireV1Error::TrailingBytes));
    }

    #[test]
    fn reject_invalid_recipients() {
        assert_eq!(
            encode_send(Send::All { recipient: "" }),
            Err(WireV1Error::InvalidRecipient)
        );
        assert_eq!(
            encode_send(Send::All { recipient: "a b" }),
            Err(WireV1Error::InvalidRecipient)
        );
        assert_eq!(
            encode_send(Send::All {
                recipient: &"x".repeat(MAX_ADDRESS_BYTES + 1)
            }),
            Err(WireV1Error::InvalidRecipient)
        );
    }

    #[test]
    fn reject_other_type_and_truncations() {
        let wrong = encode_frame(TransactionType::try_from(1).unwrap(), b"hi").unwrap();
        assert_eq!(parse_send(&wrong), Err(WireV1Error::WrongTransactionType(1)));
        assert_eq!(parse_send(b"TLR\x01\x02\x00"), Err(WireV1Error::Truncated));
        assert_eq!(parse_send(b"TLR\x01\x02\x7f"), Err(WireV1Error::InvalidSendVariant(127)));
        assert_eq!(parse_send(b"TLR\x01\x02\x01\x03bo"), Err(WireV1Error::Truncated));
    }

    #[test]
    fn frame_boundary_is_enforced() {
        assert_eq!(
            parse_frame(&vec![0; MAX_FRAME_BYTES + 1]),
            Err(WireV1Error::TooLong)
        );
        let t = TransactionType::try_from(15).unwrap();
        assert_eq!(
            encode_frame(t, &vec![0; MAX_FRAME_BYTES - HEADER_LEN + 1]),
            Err(WireV1Error::TooLong)
        );
    }
}
