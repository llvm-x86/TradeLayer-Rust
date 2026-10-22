//! Inspection of zkTL Cairo batch *claims*, never a proof verifier.
//!
//! zkTL's batch-binding STWO example binds checkpoint metadata, rather than
//! proving that every receipt describes an authorized state transition.
//! Parsed claims cannot authorize balances, keys or ChainState operations.

use crate::hash256::sha256;
use crate::read_only::decode_hex;
use crate::TransactionType;
use std::collections::BTreeSet;
use std::fmt;

pub const MAX_BATCH_BYTES: usize = 2_000_000;
pub const MAX_RECEIPTS: usize = 10_000;
pub const PROTOCOL: &str = "tlzk_aggregated_batch_checkpoint_v1";
pub const RECEIPT_PROTOCOL: &str = "tlzk_trace_transition_receipt_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    pub fn parse(value: &str) -> Result<Self, ClaimError> {
        if value.len() != 64 || !value.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(ClaimError::Invalid("32-byte lowercase digest expected"));
        }
        if value.bytes().any(|c| c.is_ascii_uppercase()) {
            return Err(ClaimError::Invalid("digest must use lowercase hex"));
        }
        let bytes = decode_hex(value).map_err(|_| ClaimError::Invalid("digest hex"))?;
        let array: [u8; 32] = bytes
            .try_into()
            .map_err(|_| ClaimError::Invalid("digest size"))?;
        Ok(Self(array))
    }

    pub fn hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Fixed-width canonical digest bytes for read-only root reconciliation.
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    fn hash(bytes: &[u8]) -> Self {
        Self(sha256(bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    TooLarge,
    Invalid(&'static str),
    Mismatch(&'static str),
    AuditOnly,
    ConflictingTx34,
}

impl fmt::Display for ClaimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ClaimError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportLevel {
    StateKernel,
    AuditReplay,
}

impl SupportLevel {
    fn parse(value: &str) -> Result<Self, ClaimError> {
        match value {
            "state_kernel" => Ok(Self::StateKernel),
            "audit_replay" => Ok(Self::AuditReplay),
            _ => Err(ClaimError::Invalid("unknown supportLevel")),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::StateKernel => "state_kernel",
            Self::AuditReplay => "audit_replay",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub receipt_id: Digest,
    pub txid: Digest,
    pub tx_type: TransactionType,
    pub height: u64,
    pub old_state_root: Digest,
    pub new_state_root: Digest,
    pub payload_hash: Digest,
    pub witness_root: Digest,
    pub support: SupportLevel,
    pub economic_mutation: bool,
    pub valid: bool,
}

impl Receipt {
    /// Byte-for-byte compatible with zkTL `hashCanonical(receiptCore)` for
    /// canonical `tlzk_trace_transition_receipt_v1` records with empty reason.
    /// All displayed fields are ASCII or fixed-width hex, so JSON quoting is
    /// unambiguous. Nonempty failure reasons are rejected by the bridge.
    pub fn calculate_receipt_id(&self) -> Digest {
        let canonical = format!(
            concat!(
                "{{\"block\":{},\"economicMutation\":{},\"newStateRoot\":\"{}\",",
                "\"oldStateRoot\":\"{}\",\"payloadHash\":\"{}\",",
                "\"protocol\":\"{}\",\"reason\":\"\",",
                "\"supportLevel\":\"{}\",\"txType\":{},\"txid\":\"{}\",",
                "\"valid\":{},\"witnessRoot\":\"{}\"}}"
            ),
            self.height,
            self.economic_mutation,
            self.new_state_root.hex(),
            self.old_state_root.hex(),
            self.payload_hash.hex(),
            RECEIPT_PROTOCOL,
            self.support.as_str(),
            self.tx_type.as_u8(),
            self.txid.hex(),
            self.valid,
            self.witness_root.hex(),
        );
        Digest::hash(canonical.as_bytes())
    }
}

/// Inspectable metadata containing zkTL commitments. This is *not* a STWO
/// proof result, a Cairo execution witness, or Bitcoin block inclusion proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditBatch {
    pub batch_id: Digest,
    pub transition_root: Digest,
    pub first_state_root: Digest,
    pub final_state_root: Digest,
    pub start_height: u64,
    pub end_height: u64,
    pub receipts: Vec<Receipt>,
}

impl AuditBatch {
    pub fn calculate_transition_root(&self) -> Digest {
        let ids = digest_json_array(self.receipts.iter().map(|r| r.receipt_id));
        Digest::hash(ids.as_bytes())
    }

    pub fn calculate_batch_id(&self) -> Digest {
        let ids = digest_json_array(self.receipts.iter().map(|r| r.receipt_id));
        let canonical = format!(
            concat!(
                "{{\"endHeight\":{},\"finalStateRoot\":\"{}\",",
                "\"firstStateRoot\":\"{}\",\"protocol\":\"{}\",",
                "\"receiptIds\":{},\"startHeight\":{},\"transitionRoot\":\"{}\"}}"
            ),
            self.end_height,
            self.final_state_root.hex(),
            self.first_state_root.hex(),
            PROTOCOL,
            ids,
            self.start_height,
            self.transition_root.hex()
        );
        Digest::hash(canonical.as_bytes())
    }

    /// Pure metadata consistency check. Passing means that a zkTL JSON claim
    /// is internally consistent. The STWO proof and economic transitions are
    /// **unverified**, and this value must never authorize a state mutation.
    pub fn inspect_strict(&self) -> Result<(), ClaimError> {
        if self.start_height > self.end_height || self.receipts.is_empty() {
            return Err(ClaimError::Invalid("empty or reversed block interval"));
        }
        if self.receipts.len() > MAX_RECEIPTS {
            return Err(ClaimError::TooLarge);
        }
        let mut seen = BTreeSet::new();
        let mut prior_root = self.first_state_root;
        let mut prior_height = self.start_height;
        for receipt in &self.receipts {
            if receipt.height < prior_height || receipt.height > self.end_height {
                return Err(ClaimError::Invalid("receipt block ordering"));
            }
            if !seen.insert(receipt.txid) {
                return Err(ClaimError::Invalid("duplicate transaction ID"));
            }
            if receipt.old_state_root != prior_root {
                return Err(ClaimError::Mismatch("root chain"));
            }
            if !receipt.valid {
                return Err(ClaimError::Invalid("invalid receipt"));
            }
            if receipt.support != SupportLevel::StateKernel {
                return Err(ClaimError::AuditOnly);
            }
            // zkTL maps its ZK batch movement to 34, while TradeLayer.js
            // assigns 34 to Cross-Layer Bridge. This conflict is unresolved.
            if receipt.tx_type.as_u8() == 34 {
                return Err(ClaimError::ConflictingTx34);
            }
            if receipt.calculate_receipt_id() != receipt.receipt_id {
                return Err(ClaimError::Mismatch("receiptId"));
            }
            prior_root = receipt.new_state_root;
            prior_height = receipt.height;
        }
        if prior_root != self.final_state_root {
            return Err(ClaimError::Mismatch("final state root"));
        }
        if self.calculate_transition_root() != self.transition_root {
            return Err(ClaimError::Mismatch("transitionRoot"));
        }
        if self.calculate_batch_id() != self.batch_id {
            return Err(ClaimError::Mismatch("batchId"));
        }
        Ok(())
    }
}

fn digest_json_array(digests: impl IntoIterator<Item = Digest>) -> String {
    let mut encoded = String::from("[");
    for (i, digest) in digests.into_iter().enumerate() {
        if i != 0 {
            encoded.push(',');
        }
        encoded.push('"');
        encoded.push_str(&digest.hex());
        encoded.push('"');
    }
    encoded.push(']');
    encoded
}

fn parse_u64(value: &str) -> Result<u64, ClaimError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|c| c.is_ascii_digit())
    {
        return Err(ClaimError::Invalid("unsigned integer"));
    }
    value
        .parse()
        .map_err(|_| ClaimError::Invalid("integer overflow"))
}

fn parse_bool(value: &str) -> Result<bool, ClaimError> {
    match value {
        "1" => Ok(true),
        "0" => Ok(false),
        _ => Err(ClaimError::Invalid("boolean")),
    }
}

/// Bridge transport from `scripts/export_tlzk_batch.py`, using *exactly* one
/// header plus ordered receipt rows. This transport is not a signed format.
/// Header: TLZK1 batchId start end transitionRoot firstRoot finalRoot
/// Receipt: R id txid type height oldRoot newRoot payloadHash witnessRoot
///            supportLevel economicMutation valid (12 tab-separated fields).
pub fn parse_bridge_tsv(input: &str) -> Result<AuditBatch, ClaimError> {
    if input.len() > MAX_BATCH_BYTES || !input.is_ascii() {
        return Err(ClaimError::TooLarge);
    }
    let mut lines = input.lines();
    let line = lines.next().ok_or(ClaimError::Invalid("missing header"))?;
    let hdr: Vec<_> = line.split('\t').collect();
    if hdr.len() != 7 || hdr[0] != "TLZK1" {
        return Err(ClaimError::Invalid("header"));
    }
    let mut batch = AuditBatch {
        batch_id: Digest::parse(hdr[1])?,
        start_height: parse_u64(hdr[2])?,
        end_height: parse_u64(hdr[3])?,
        transition_root: Digest::parse(hdr[4])?,
        first_state_root: Digest::parse(hdr[5])?,
        final_state_root: Digest::parse(hdr[6])?,
        receipts: Vec::new(),
    };
    for line in lines {
        if batch.receipts.len() == MAX_RECEIPTS {
            return Err(ClaimError::TooLarge);
        }
        let f: Vec<_> = line.split('\t').collect();
        if f.len() != 12 || f[0] != "R" {
            return Err(ClaimError::Invalid("receipt row"));
        }
        let ty = parse_u64(f[3])?;
        let type_byte = u8::try_from(ty).map_err(|_| ClaimError::Invalid("transaction type"))?;
        let tx_type = TransactionType::try_from(type_byte)
            .map_err(|_| ClaimError::Invalid("unknown transaction type"))?;
        batch.receipts.push(Receipt {
            receipt_id: Digest::parse(f[1])?,
            txid: Digest::parse(f[2])?,
            tx_type,
            height: parse_u64(f[4])?,
            old_state_root: Digest::parse(f[5])?,
            new_state_root: Digest::parse(f[6])?,
            payload_hash: Digest::parse(f[7])?,
            witness_root: Digest::parse(f[8])?,
            support: SupportLevel::parse(f[9])?,
            economic_mutation: parse_bool(f[10])?,
            valid: parse_bool(f[11])?,
        });
    }
    Ok(batch)
}
