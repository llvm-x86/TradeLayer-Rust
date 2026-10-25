//! Offline, **untrusted** Bitcoin-style transaction inspection.
//!
//! This module decodes Bitcoin transaction serialization, fingerprints txids,
//! and identifies a single canonical push after OP_RETURN. It cannot prove
//! inclusion, best-chain status, signature validity, input ownership, or TradeLayer
//! authorization. Its observations MUST NOT implement `TrustedChainAdapter`.
//! Historical Litecoin/Dogecoin features beyond this common serialization are
//! not covered by the fixture suite.

use crate::hash256::{double_sha256, double_sha256_parts};
use crate::{wire, wire_v1};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_RAW_TRANSACTION: usize = 1_000_000;
pub const MAX_IO_COUNT: u64 = 100_000;
pub const MAX_CORPUS_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_CORPUS_ROWS: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    TooLong,
    Truncated,
    Invalid(String),
    NonCanonicalCompactSize,
    InvalidHex,
    TrailingBytes,
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ScanError {}

struct Cursor<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, offset: 0 }
    }

    fn read(&mut self, length: usize) -> Result<&'a [u8], ScanError> {
        let end = self.offset.checked_add(length).ok_or(ScanError::TooLong)?;
        let bytes = self
            .input
            .get(self.offset..end)
            .ok_or(ScanError::Truncated)?;
        self.offset = end;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, ScanError> {
        Ok(self.read(1)?[0])
    }

    fn varint(&mut self) -> Result<u64, ScanError> {
        let prefix = self.byte()?;
        let number = match prefix {
            0..=252 => u64::from(prefix),
            253 => {
                let number =
                    u64::from(u16::from_le_bytes(self.read(2)?.try_into().expect("slice")));
                if number < 253 {
                    return Err(ScanError::NonCanonicalCompactSize);
                }
                number
            }
            254 => {
                let number =
                    u64::from(u32::from_le_bytes(self.read(4)?.try_into().expect("slice")));
                if number <= u16::MAX as u64 {
                    return Err(ScanError::NonCanonicalCompactSize);
                }
                number
            }
            255 => {
                let number = u64::from_le_bytes(self.read(8)?.try_into().expect("slice"));
                if number <= u32::MAX as u64 {
                    return Err(ScanError::NonCanonicalCompactSize);
                }
                number
            }
        };
        Ok(number)
    }

    fn limited_count(&mut self) -> Result<usize, ScanError> {
        let value = self.varint()?;
        if value == 0 || value > MAX_IO_COUNT {
            return Err(ScanError::Invalid(
                "zero/excessive input or output count".into(),
            ));
        }
        usize::try_from(value).map_err(|_| ScanError::TooLong)
    }

    fn variable_bytes(&mut self) -> Result<&'a [u8], ScanError> {
        let size = self.varint()?;
        if size > MAX_RAW_TRANSACTION as u64 {
            return Err(ScanError::TooLong);
        }
        self.read(usize::try_from(size).map_err(|_| ScanError::TooLong)?)
    }
}

/// Hash in display order, the reverse of SHA256d digest bytes.
pub fn id_hex(digest: &[u8; 32]) -> String {
    digest
        .iter()
        .rev()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Encode raw bytes for fixture and log inspection.
pub fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn decode_hex(hex: &str) -> Result<Vec<u8>, ScanError> {
    fn nibble(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    if !hex.len().is_multiple_of(2) || hex.len() > 2 * MAX_RAW_TRANSACTION {
        return Err(ScanError::InvalidHex);
    }
    let mut result = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks_exact(2) {
        let high = nibble(pair[0]).ok_or(ScanError::InvalidHex)?;
        let low = nibble(pair[1]).ok_or(ScanError::InvalidHex)?;
        result.push((high << 4) | low);
    }
    Ok(result)
}

/// A single `OP_RETURN <canonical push>` carrying one of the TradeLayer markers.
/// The marker is only a syntactic observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarrierFormat {
    LegacyAscii,
    ProposedBinaryV1,
}
impl CarrierFormat {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LegacyAscii => "legacy-tl",
            Self::ProposedBinaryV1 => "proposed-TLR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carrier {
    pub vout: u32,
    pub format: CarrierFormat,
    pub payload: Vec<u8>,
    pub transaction_type: Option<u8>,
    pub status: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionObservation {
    /// Raw digest bytes in hashing order (not explorer display order).
    pub txid: [u8; 32],
    pub wtxid: [u8; 32],
    pub has_witness: bool,
    pub input_count: u32,
    pub output_count: u32,
    pub op_return_count: u32,
    pub carriers: Vec<Carrier>,
}

/// Strictly extract one push following OP_RETURN. Multiple pushes, trailing
/// opcodes, and malformed/nonminimal encodings are ignored as TL candidates.
fn one_op_return_push(script: &[u8]) -> Option<&[u8]> {
    if script.first().copied() != Some(0x6a) {
        return None;
    }
    let opcode = *script.get(1)?;
    let (length, start): (usize, usize) = match opcode {
        1..=75 => (usize::from(opcode), 2),
        0x4c => {
            let size = usize::from(*script.get(2)?);
            if size <= 75 {
                return None;
            }
            (size, 3)
        }
        0x4d => {
            let bytes: [u8; 2] = script.get(2..4)?.try_into().ok()?;
            let size = usize::from(u16::from_le_bytes(bytes));
            if size <= u8::MAX as usize {
                return None;
            }
            (size, 4)
        }
        0x4e => {
            let bytes: [u8; 4] = script.get(2..6)?.try_into().ok()?;
            let size = usize::try_from(u32::from_le_bytes(bytes)).ok()?;
            if size <= u16::MAX as usize {
                return None;
            }
            (size, 6)
        }
        _ => return None,
    };
    let end = start.checked_add(length)?;
    if end != script.len() {
        return None;
    }
    script.get(start..end)
}

fn classify(vout: u32, payload: &[u8]) -> Option<Carrier> {
    let (format, transaction_type, status) = if payload.starts_with(b"tl") {
        let value = match std::str::from_utf8(payload) {
            Ok(value) => value,
            Err(_) => {
                return Some(Carrier {
                    vout,
                    format: CarrierFormat::LegacyAscii,
                    payload: payload.to_vec(),
                    transaction_type: None,
                    status: "invalid-ascii",
                })
            }
        };
        match wire::parse_frame(value) {
            Ok(frame) => {
                let status = match wire::parse_typed(value) {
                    Ok(_) => "typed-syntax",
                    Err(wire::WireError::UnsupportedType(_)) => "opaque-type",
                    Err(_) => "invalid-body",
                };
                (
                    CarrierFormat::LegacyAscii,
                    Some(frame.tx_type.as_u8()),
                    status,
                )
            }
            Err(_) => (CarrierFormat::LegacyAscii, None, "invalid-frame"),
        }
    } else if payload.starts_with(b"TLR") {
        match wire_v1::parse_frame(payload) {
            Ok(frame) => {
                let status = if frame.tx_type.as_u8() == 2 {
                    if wire_v1::parse_send(payload).is_ok() {
                        "typed-syntax"
                    } else {
                        "invalid-body"
                    }
                } else {
                    "opaque-type"
                };
                (
                    CarrierFormat::ProposedBinaryV1,
                    Some(frame.tx_type.as_u8()),
                    status,
                )
            }
            Err(_) => (CarrierFormat::ProposedBinaryV1, None, "invalid-frame"),
        }
    } else {
        return None;
    };
    Some(Carrier {
        vout,
        format,
        payload: payload.to_vec(),
        transaction_type,
        status,
    })
}

/// Parses a complete Bitcoin-style serialized transaction and identifies TL
/// OP_RETURN candidates without assigning origins or mutating ledger state.
pub fn inspect_transaction(raw: &[u8]) -> Result<TransactionObservation, ScanError> {
    if raw.len() > MAX_RAW_TRANSACTION {
        return Err(ScanError::TooLong);
    }
    let mut reader = Cursor::new(raw);
    let version = reader.read(4)?;
    let has_witness = raw.get(4).copied() == Some(0);
    if has_witness {
        let marker = reader.byte()?;
        let flag = reader.byte()?;
        if marker != 0 || flag != 1 {
            return Err(ScanError::Invalid("unsupported SegWit marker/flag".into()));
        }
    }
    let inputs_start = reader.offset;
    let inputs = reader.limited_count()?;
    for _ in 0..inputs {
        reader.read(36)?; // outpoint
        reader.variable_bytes()?; // scriptSig
        reader.read(4)?; // sequence
    }
    let outputs = reader.limited_count()?;
    let mut carriers = Vec::new();
    let mut op_return_count = 0u32;
    for n in 0..outputs {
        reader.read(8)?; // value
        let script = reader.variable_bytes()?;
        if script.first() == Some(&0x6a) {
            op_return_count += 1;
            if let Some(payload) = one_op_return_push(script) {
                if let Some(carrier) = classify(n as u32, payload) {
                    carriers.push(carrier);
                }
            }
        }
    }
    let outputs_end = reader.offset;
    if has_witness {
        let mut nonempty_witness = false;
        for _ in 0..inputs {
            let n = reader.varint()?;
            if n > MAX_IO_COUNT {
                return Err(ScanError::TooLong);
            }
            for _ in 0..n {
                let data = reader.variable_bytes()?;
                nonempty_witness |= !data.is_empty();
            }
        }
        if !nonempty_witness {
            return Err(ScanError::Invalid(
                "superfluous SegWit witness marker".into(),
            ));
        }
    }
    let locktime = reader.read(4)?;
    if reader.offset != raw.len() {
        return Err(ScanError::TrailingBytes);
    }
    // For non-SegWit serialization, BIP141 defines txid == wtxid.
    // Hash exactly once. The previous version hashed the same raw bytes
    // TWICE for each legacy transaction, wasting a hot CPU preindex pass.
    let wtxid = double_sha256(raw);
    let txid = if has_witness {
        // SegWit txid excludes marker, flag and all witness stacks. Hash
        // the three stripped segments in place, with no temporary Vec.
        double_sha256_parts(&[version, &raw[inputs_start..outputs_end], locktime])
    } else {
        wtxid
    };
    Ok(TransactionObservation {
        txid,
        wtxid,
        has_witness,
        input_count: u32::try_from(inputs).map_err(|_| ScanError::TooLong)?,
        output_count: u32::try_from(outputs).map_err(|_| ScanError::TooLong)?,
        op_return_count,
        carriers,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayItem {
    pub label: String,
    pub network_label: String,
    pub reported_height: Option<u64>,
    pub reported_block_hash: Option<String>,
    pub observation: TransactionObservation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayError {
    pub line: usize,
    pub reason: String,
}
impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}
impl std::error::Error for ReplayError {}

/// Scan a TSV corpus in file order. Each line has exactly six columns:
/// label, network, reported_height_or_dash, reported_blockhash_or_dash,
/// expected_txid_display_hex, raw_transaction_hex.
///
/// The reported chain metadata is UNVERIFIED. Matching txids proves only that
/// the supplied bytes match their expected fingerprint; it does not prove
/// block inclusion, input signatures, ownership, or protocol acceptance.
pub fn replay_tsv(tsv: &str) -> Result<Vec<ReplayItem>, ReplayError> {
    if tsv.len() > MAX_CORPUS_BYTES {
        return Err(ReplayError {
            line: 0,
            reason: "fixture corpus too large".into(),
        });
    }
    let mut result = Vec::new();
    let mut observed: BTreeSet<(String, String)> = BTreeSet::new();
    let mut heights: BTreeMap<String, u64> = BTreeMap::new();
    for (index, line) in tsv.lines().enumerate() {
        let line_number = index + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if result.len() == MAX_CORPUS_ROWS {
            return Err(ReplayError {
                line: line_number,
                reason: "too many rows".into(),
            });
        }
        let problem = |message: String| ReplayError {
            line: line_number,
            reason: message,
        };
        let columns: Vec<_> = line.split('\t').collect();
        let [label, network, height, blockhash, txid, raw_hex] = <[&str; 6]>::try_from(columns)
            .map_err(|_| problem("expected six TSV columns".into()))?;
        if label.is_empty()
            || network.is_empty()
            || !label.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
            || !network
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(problem("bad fixture label/network".into()));
        }
        let reported_height = if height == "-" {
            None
        } else {
            Some(
                height
                    .parse::<u64>()
                    .map_err(|_| problem("invalid height".into()))?,
            )
        };
        if let Some(height) = reported_height {
            if heights.get(network).is_some_and(|last| *last > height) {
                return Err(problem("height order regressed".into()));
            }
            heights.insert(network.to_owned(), height);
        }
        let reported_block_hash = if blockhash == "-" {
            None
        } else {
            if blockhash.len() != 64 || decode_hex(blockhash).is_err() {
                return Err(problem("invalid reported block hash".into()));
            }
            Some((*blockhash).to_owned())
        };
        if txid.len() != 64 || decode_hex(txid).is_err() {
            return Err(problem("bad expected txid".into()));
        }
        let raw = decode_hex(raw_hex).map_err(|e| problem(e.to_string()))?;
        let observation = inspect_transaction(&raw).map_err(|e| problem(e.to_string()))?;
        let derived = id_hex(&observation.txid);
        if !derived.eq_ignore_ascii_case(txid) {
            return Err(problem(format!(
                "txid mismatch: expected {txid}, derived {derived}"
            )));
        }
        if !observed.insert((network.to_owned(), derived)) {
            return Err(problem(
                "duplicate transaction on same named network".into(),
            ));
        }
        result.push(ReplayItem {
            label: (*label).to_owned(),
            network_label: network.to_owned(),
            reported_height,
            reported_block_hash,
            observation,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_push_extraction() {
        assert_eq!(one_op_return_push(b"\x6a\x02tl"), Some(&b"tl"[..]));
        assert_eq!(one_op_return_push(b"\x6a\x4c\x4c"), None); // truncated
        assert_eq!(one_op_return_push(b"\x6a\x4c\x02tl"), None); // nonminimal
        assert_eq!(one_op_return_push(b"\x6a\x02tl\x51"), None); // trailing opcode
        assert_eq!(one_op_return_push(b"\x00\x6a\x02tl"), None); // nonstandard prefix
    }

    #[test]
    fn hex_checks() {
        assert_eq!(decode_hex("AbC0"), Ok(vec![171, 192]));
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("0z").is_err());
        assert!(decode_hex(&"ff".repeat(MAX_RAW_TRANSACTION + 1)).is_err());
    }

    #[test]
    fn malformed_transactions_fail_closed() {
        assert_eq!(inspect_transaction(b""), Err(ScanError::Truncated));
        let malformed = decode_hex("0100000000ff").unwrap();
        assert!(inspect_transaction(&malformed).is_err());
        let noncanonical = decode_hex("01000000fdfc00").unwrap();
        assert_eq!(
            inspect_transaction(&noncanonical),
            Err(ScanError::NonCanonicalCompactSize)
        );
    }
}
