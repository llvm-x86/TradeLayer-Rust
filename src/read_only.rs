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

/// Read a canonical Bitcoin CompactSize prefix from a borrowed byte stream.
/// Internal block-ingestion boundary: this is syntax, not consensus authority.
pub(crate) fn compact_size_prefix(raw: &[u8]) -> Result<(u64, usize), ScanError> {
    let mut cursor = Cursor::new(raw);
    let count = cursor.varint()?;
    Ok((count, cursor.offset))
}

/// A single canonical parse over raw TX syntax, storing only the structural
/// offsets needed for later hashing and source-carrier classification.
/// Ordinary non-TL transactions allocate no per-TX payload vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TransactionLayout {
    total_bytes: usize,
    stripped_bytes: usize,
    input_start: usize,
    outputs_end: usize,
    has_witness: bool,
    input_count: u32,
    output_count: u32,
    op_return_count: u32,
    payloads: Vec<(u32, std::ops::Range<usize>)>,
}
impl TransactionLayout {
    pub(crate) const fn total_bytes(&self) -> usize { self.total_bytes }
    pub(crate) const fn stripped_bytes(&self) -> usize { self.stripped_bytes }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RawOutpoint {
    pub(crate) txid_raw: [u8; 32],
    pub(crate) vout: u32,
}

/// Extract first-input parent references only when an offline dependency
/// report is requested. This does not prove that external UTXOs exist.
pub(crate) fn source_prevouts_from_layout(
    raw: &[u8], layout: &TransactionLayout,
) -> Result<Vec<RawOutpoint>, ScanError> {
    if raw.len() != layout.total_bytes { return Err(ScanError::TrailingBytes); }
    let mut cursor = Cursor::new(raw);
    cursor.offset = layout.input_start;
    let mut out = Vec::with_capacity(layout.input_count as usize);
    for _ in 0..layout.input_count {
        let txid_raw = cursor.read(32)?.try_into().expect("read 32-byte parent");
        let vout = u32::from_le_bytes(cursor.read(4)?.try_into().expect("read u32"));
        out.push(RawOutpoint { txid_raw, vout });
        cursor.variable_bytes()?;
        cursor.read(4)?;
    }
    Ok(out)
}

/// Locate exactly one transaction at the start of a possibly larger block
/// suffix. This strictly parses BIP144 marker, vin/vout, witness and locktime
/// without computing a TXID or copying serialized bytes.
pub(crate) fn prepare_transaction_prefix(raw: &[u8]) -> Result<TransactionLayout, ScanError> {
    let mut reader = Cursor::new(raw);
    reader.read(4)?; // version
    let has_witness = raw.get(4) == Some(&0);
    if has_witness {
        let marker = reader.byte()?;
        let flag = reader.byte()?;
        if marker != 0 || flag != 1 {
            return Err(ScanError::Invalid("unsupported SegWit marker/flag".into()));
        }
    }
    let input_start = reader.offset;
    let input_count = reader.limited_count()?;
    for _ in 0..input_count {
        reader.read(36)?;
        reader.variable_bytes()?;
        reader.read(4)?;
    }
    let output_count = reader.limited_count()?;
    let mut op_return_count = 0u32;
    let mut payloads = Vec::new();
    for n in 0..output_count {
        reader.read(8)?;
        let script = reader.variable_bytes()?;
        if script.first() == Some(&0x6a) {
            op_return_count += 1;
            if let Some(payload) = one_op_return_push(script) {
                if payload.starts_with(b"tl") || payload.starts_with(b"TLR") {
                    let script_start = reader.offset - script.len();
                    let start = script_start + script.len() - payload.len();
                    payloads.push((n as u32, start..reader.offset));
                }
            }
        }
    }
    let outputs_end = reader.offset;
    if has_witness {
        let mut nonempty = false;
        for _ in 0..input_count {
            let count = reader.varint()?;
            if count > MAX_IO_COUNT { return Err(ScanError::TooLong); }
            for _ in 0..count {
                nonempty |= !reader.variable_bytes()?.is_empty();
            }
        }
        if !nonempty {
            return Err(ScanError::Invalid("superfluous SegWit witness marker".into()));
        }
    }
    reader.read(4)?;
    let total_bytes = reader.offset;
    if total_bytes > MAX_RAW_TRANSACTION { return Err(ScanError::TooLong); }
    let stripped_bytes = if has_witness { 8 + outputs_end - input_start } else { total_bytes };
    Ok(TransactionLayout {
        total_bytes, stripped_bytes, input_start, outputs_end, has_witness,
        input_count: input_count as u32, output_count: output_count as u32,
        op_return_count, payloads,
    })
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

/// Hash and classify carriers using a previously validated structural scan.
/// No second walk through every vin, vout, scriptSig or witness stack.
pub(crate) fn inspect_prepared_transaction(
    raw: &[u8], layout: &TransactionLayout,
) -> Result<TransactionObservation, ScanError> {
    if raw.len() > MAX_RAW_TRANSACTION { return Err(ScanError::TooLong); }
    if raw.len() != layout.total_bytes { return Err(ScanError::TrailingBytes); }
    let wtxid = double_sha256(raw);
    let txid = if layout.has_witness {
        double_sha256_parts(&[
            &raw[..4], &raw[layout.input_start..layout.outputs_end],
            &raw[raw.len() - 4..],
        ])
    } else {
        wtxid
    };
    let carriers = layout.payloads.iter()
        .filter_map(|(vout, range)| classify(*vout, &raw[range.clone()]))
        .collect();
    Ok(TransactionObservation {
        txid, wtxid, has_witness: layout.has_witness,
        input_count: layout.input_count, output_count: layout.output_count,
        op_return_count: layout.op_return_count, carriers,
    })
}

/// Standalone TX inspection checks exact length; block preparse uses the
/// same structural layout against a larger borrowed suffix.
pub fn inspect_transaction(raw: &[u8]) -> Result<TransactionObservation, ScanError> {
    if raw.len() > MAX_RAW_TRANSACTION { return Err(ScanError::TooLong); }
    let layout = prepare_transaction_prefix(raw)?;
    inspect_prepared_transaction(raw, &layout)
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
