//! Source-expression audit of pinned legacy txIndex.js, file 8/49.
//!
//! Source: tradelayer.js@dca0888b, src/txIndex.js:161-196, 241-315.
//! The source's OP_RETURN scan is NOT the strict Rust one. It searches
//! literal lowercase HEX "746c" at nibble positions <=6 of the FIRST
//! nulldata output, then decodes Buffer.from(hex) with truncation of odd or
//! malformed hex and enforces printable ASCII. It does not require an OP_
//! RETURN canonical push, and an empty payload still matches its ASCII rule.
//! This source audit preserves those differences as UNVERIFIED observations.
//!
//! Source TxIndex.processTransaction also discards output vout positions when
//! mapping references 0/1 and calls Types.decodePayload(txId, marker, payload)
//! with THREE arguments, while the callee expects NINE. A returned JS Error
//! object does not magically become a valid transaction.
//!
//! The performance scanner in txindex_parallel.rs instead parses complete
//! Bitcoin-style serialized TXs strictly and parallelizes independent TXID
//! hashes, while preserving original transaction position for serial
//! stateful verification. Neither module authenticates consensus or funds.

use crate::legacy_types_audit::{
    source_inspect_decode_invocation, SourceDecoderInvocation, UnverifiedTypesDispatch,
};

pub const MAX_SOURCE_INDEX_HEX: usize = 8192;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyIndexError {
    MalformedSourceOutput,
    OversizedSourceHex,
    NonPrintableCarrier,
    EmptySourceTxid,
    UnverifiedSourceOrdering,
    UnverifiedFirstInputAndReferences,
    NoAcceptedConsensusOrSignedRevision,
}
#[derive(Debug, Clone, Copy)]
pub struct ClaimedOutput<'a> {
    pub vout: u32,
    pub is_nulldata: bool,
    pub script_hex: &'a str,
    pub address: &'a str,
    pub satoshis: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedMarkerSource {
    payload: String,
    selected_vout: u32,
    marker_nibble_offset: usize,
}
impl UnverifiedMarkerSource {
    pub fn payload_after_marker(&self) -> &str {
        &self.payload
    }
    pub const fn selected_vout(&self) -> u32 {
        self.selected_vout
    }
    pub const fn source_nibble_position(&self) -> usize {
        self.marker_nibble_offset
    }
    pub fn legacy_decoder_entry(&self) -> UnverifiedTypesDispatch {
        source_inspect_decode_invocation(SourceDecoderInvocation::TxIndexThreeArguments {
            payload_after_marker: &self.payload,
        })
    }
    pub const fn require_historical_transaction_acceptance(&self) -> Result<(), LegacyIndexError> {
        Err(LegacyIndexError::NoAcceptedConsensusOrSignedRevision)
    }
}
fn nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
/// Node Buffer.from(text,'hex') accepts pairs until the FIRST invalid pair
/// and ignores an unmatched final nibble. This is NOT a canonical consensus
/// hex decoder. Use it only to demonstrate pinned JS source output.
fn legacy_loose_hex(hex: &str) -> Vec<u8> {
    let mut decoded = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks_exact(2) {
        let (Some(a), Some(b)) = (nibble(pair[0]), nibble(pair[1])) else {
            break;
        };
        decoded.push((a << 4) | b);
    }
    decoded
}
/// Source: first nulldata OUTPUT wins even if its marker is missing.
pub fn source_extract_first_nulldata_unverified(
    outputs: &[ClaimedOutput<'_>],
) -> Result<Option<UnverifiedMarkerSource>, LegacyIndexError> {
    let Some(first) = outputs.iter().find(|o| o.is_nulldata) else {
        return Ok(None);
    };
    if first.script_hex.len() > MAX_SOURCE_INDEX_HEX {
        return Err(LegacyIndexError::OversizedSourceHex);
    }
    if !first.script_hex.is_ascii() {
        return Err(LegacyIndexError::MalformedSourceOutput);
    }
    let Some(offset) = first.script_hex.find("746c") else {
        return Ok(None);
    };
    if offset > 6 {
        return Ok(None);
    }
    let start = if first.script_hex.get(4..8) == Some("746c") {
        8
    } else if first.script_hex.get(5..9) == Some("746c") {
        9
    } else if first.script_hex.get(6..10) == Some("746c") {
        10
    } else {
        offset + 4
    };
    let after = first.script_hex.get(start..).unwrap_or("");
    let bytes = legacy_loose_hex(after);
    if !bytes.iter().all(|ch| (0x20..=0x7e).contains(ch)) {
        return Err(LegacyIndexError::NonPrintableCarrier);
    }
    let payload = String::from_utf8(bytes).expect("printable ASCII");
    Ok(Some(UnverifiedMarkerSource {
        payload,
        selected_vout: first.vout,
        marker_nibble_offset: offset,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedReference {
    pub address: String,
    pub satoshis: u64,
    // Legacy TxIndex threw away the original vout number. Intentionally
    // missing, never invent a mapping from array position to vout index.
}
pub fn source_project_refs_0_and_1_unverified(
    outputs: &[ClaimedOutput<'_>],
) -> Vec<UnverifiedReference> {
    outputs
        .iter()
        .filter(|o| o.vout == 0 || o.vout == 1)
        .map(|o| UnverifiedReference {
            address: o.address.into(),
            satoshis: o.satoshis,
        })
        .collect()
}
/// Pinned NeDB row ID drops block hash and network namespace; same
/// height+txid across forks aliases the same key. It is NOT reorg safe.
pub fn source_index_row_key_unverified(
    height: u64,
    txid: &str,
) -> Result<String, LegacyIndexError> {
    if txid.is_empty() {
        return Err(LegacyIndexError::EmptySourceTxid);
    }
    if !txid.is_ascii() || !txid.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(LegacyIndexError::MalformedSourceOutput);
    }
    Ok(format!("tx-{height}-{txid}"))
}

/// Source processBlockData updates MaxHeight INSIDE the detected-TL loop
/// and once at end-of-extractBlockData WITHOUT awaiting the Promise. This
/// is a source observation, not a crash-safe exact checkpoint scheme.
pub const fn source_updates_max_height_in_loop_on_tl_only() -> bool {
    true
}
pub const fn require_historical_index_consensus() -> Result<(), LegacyIndexError> {
    Err(LegacyIndexError::UnverifiedSourceOrdering)
}
pub const fn require_authenticated_reference_output() -> Result<(), LegacyIndexError> {
    Err(LegacyIndexError::UnverifiedFirstInputAndReferences)
}
