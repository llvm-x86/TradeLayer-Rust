//! Offline Bitcoin SHA256d block-header and txid-Merkle inclusion checks.
//!
//! Checks SHA256d header hashing, compact PoW targets, parent links, canonical
//! raw non-witness txid and exact Bitcoin-style Merkle branches. IMPORTANT:
//! - A caller-selected anchor is NOT authenticated by these checks.
//! - No difficulty/retarget, timestamps/MTP, cumulative work, best-chain,
//!   block validity, UTXO ownership, signatures or TradeLayer consensus.
//! - Litecoin/Dogecoin Scrypt PoW is NOT supported by this module.
//! - The internal SHA256 implementation is not an independently audited
//!   production cryptographic primitive.
//! - These proofs MUST NOT implement TrustedProtocolAdapter directly.
//! A self-consistent synthetic regtest-like header is NOT a Bitcoin block.

use crate::chain::{BlockId, Tip};
use crate::hash256::double_sha256;
use crate::read_only::{inspect_transaction, CarrierFormat, ScanError};
use crate::TransactionType;
use std::cmp::Ordering;

pub const MAX_HEADER_SEGMENT: usize = 500;
pub const MAX_PROOF_TRANSACTIONS: u32 = 1_000_000;
pub const MAX_PROOF_SIBLINGS: usize = 20;

/// Only Bitcoin SHA256d proof-of-work is handled. The profile sets a
/// difficulty *ceiling*, not the actual required per-height difficulty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitcoinPowProfile {
    MainnetLike,
    RegtestLike,
}

impl BitcoinPowProfile {
    fn pow_limit(self) -> [u8; 32] {
        match self {
            Self::MainnetLike => compact_target_unbounded(0x1d00ffff).expect("mainnet pow limit"),
            Self::RegtestLike => compact_target_unbounded(0x207fffff).expect("regtest pow limit"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InclusionError {
    HeaderSize,
    HeaderTooEasy,
    InvalidCompactTarget,
    InvalidProof,
    MutatedMerkleBranch,
    MerkleRootMismatch,
    UnsupportedProtocolCarrier,
    InvalidHeaderChain,
    TooManyHeaders,
    Transaction(ScanError),
}
impl core::fmt::Display for InclusionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for InclusionError {}
impl From<ScanError> for InclusionError {
    fn from(value: ScanError) -> Self {
        Self::Transaction(value)
    }
}

/// Hash-order bytes are *little endian* for the numeric PoW target.
fn exceeds(left: &[u8; 32], right: &[u8; 32]) -> bool {
    for i in (0..32).rev() {
        match left[i].cmp(&right[i]) {
            Ordering::Greater => return true,
            Ordering::Less => return false,
            Ordering::Equal => {}
        }
    }
    false
}

/// Strict canonical Bitcoin compact target, limited to representable 256-bit
/// positive targets. No consensus transition difficulty schedule is checked.
fn compact_target_unbounded(bits: u32) -> Result<[u8; 32], InclusionError> {
    let size = (bits >> 24) as usize;
    let mantissa = bits & 0x007fffff;
    if bits & 0x00800000 != 0 || mantissa == 0 || size == 0 || size > 32 {
        return Err(InclusionError::InvalidCompactTarget);
    }
    let mut target = [0u8; 32];
    if size <= 3 {
        let value = mantissa >> (8 * (3 - size));
        target[..4].copy_from_slice(&value.to_le_bytes());
    } else {
        for i in 0..3 {
            target[size - 3 + i] = ((mantissa >> (8 * i)) & 0xff) as u8;
        }
    }
    if target.iter().all(|byte| *byte == 0) {
        return Err(InclusionError::InvalidCompactTarget);
    }
    // Avoid alternate encodings of the exact same target. Bitcoin Core
    // SetCompact permits some nonminimal forms, but this offline subset
    // intentionally rejects ambiguous representations.
    let highest = target.iter().rposition(|byte| *byte != 0).expect("nonzero target");
    let mut encoded_size = highest + 1;
    let mut encoded_mantissa = if encoded_size <= 3 {
        let value = u32::from_le_bytes(target[..4].try_into().expect("4-byte target prefix"));
        value << (8 * (3 - encoded_size))
    } else {
        u32::from(target[encoded_size - 3])
            | (u32::from(target[encoded_size - 2]) << 8)
            | (u32::from(target[encoded_size - 1]) << 16)
    };
    if encoded_mantissa & 0x00800000 != 0 {
        encoded_mantissa >>= 8;
        encoded_size += 1;
    }
    if ((encoded_size as u32) << 24) | encoded_mantissa != bits {
        return Err(InclusionError::InvalidCompactTarget);
    }
    Ok(target)
}

fn reverse_hash(bytes: &[u8]) -> [u8; 32] {
    let mut hash: [u8; 32] = bytes.try_into().expect("32-byte hash");
    hash.reverse();
    hash
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckedHeader {
    height: u64,
    id: BlockId,
    parent: BlockId,
    merkle_root: [u8; 32],
    bits: u32,
}

impl CheckedHeader {
    /// Reads an 80-byte Bitcoin header, validates its *own* PoW at the
    /// profile's target ceiling, and derives its hashes independently.
    /// Height and Bitcoin network membership are still caller claims.
    pub fn check_header_at_claimed_height(
        height: u64,
        raw_header: &[u8],
        profile: BitcoinPowProfile,
    ) -> Result<Self, InclusionError> {
        if raw_header.len() != 80 {
            return Err(InclusionError::HeaderSize);
        }
        let bits = u32::from_le_bytes(raw_header[72..76].try_into().expect("bits slice"));
        let target = compact_target_unbounded(bits)?;
        if exceeds(&target, &profile.pow_limit()) {
            return Err(InclusionError::HeaderTooEasy);
        }
        let hash_bytes = double_sha256(raw_header);
        if exceeds(&hash_bytes, &target) {
            return Err(InclusionError::HeaderTooEasy);
        }
        Ok(Self {
            height,
            id: BlockId(reverse_hash(&hash_bytes)),
            parent: BlockId(reverse_hash(&raw_header[4..36])),
            merkle_root: raw_header[36..68].try_into().expect("Merkle bytes"),
            bits,
        })
    }

    /// One *known* Bitcoin-mainnet anchor: the mined genesis header. This
    /// can establish the published block hash but not independently prove
    /// membership of future headers in the active best-work chain.
    pub fn check_bitcoin_mainnet_genesis(raw_header: &[u8]) -> Result<Self, InclusionError> {
        let header = Self::check_header_at_claimed_height(
            0, raw_header, BitcoinPowProfile::MainnetLike,
        )?;
        if header.parent != BlockId([0; 32])
            || header.id.0 != bitcoin_mainnet_genesis_hash()
        {
            return Err(InclusionError::InvalidHeaderChain);
        }
        Ok(header)
    }

    pub const fn tip(self) -> Tip {
        Tip { height: self.height, id: self.id }
    }
    pub const fn parent(self) -> BlockId {
        self.parent
    }
    pub const fn merkle_root_raw(self) -> [u8; 32] {
        self.merkle_root
    }
    pub const fn n_bits(self) -> u32 {
        self.bits
    }
}

fn bitcoin_mainnet_genesis_hash() -> [u8; 32] {
    // Bitcoin Core's published mainnet genesis hash, in display order.
    [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x19, 0xd6, 0x68,
        0x9c, 0x08, 0x5a, 0xe1, 0x65, 0x83, 0x1e, 0x93,
        0x4f, 0xf7, 0x63, 0xae, 0x46, 0xa2, 0xa6, 0xc1,
        0x72, 0xb3, 0xf1, 0xb6, 0x0a, 0x8c, 0xe2, 0x6f,
    ]
}

/// Header validation against an EXTERNALLY CONFIGURED anchor. Even if
/// every SHA256d PoW/link check succeeds, the supplied anchor might be fake
/// and the segment might not be on the active chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedHeaderSegment {
    anchor: Tip,
    headers: Vec<CheckedHeader>,
}

impl CheckedHeaderSegment {
    pub fn check_against_claimed_anchor(
        anchor: Tip,
        header_bytes: &[[u8; 80]],
        profile: BitcoinPowProfile,
    ) -> Result<Self, InclusionError> {
        if header_bytes.is_empty() || header_bytes.len() > MAX_HEADER_SEGMENT {
            return Err(InclusionError::TooManyHeaders);
        }
        let mut expected_parent = anchor;
        let mut headers = Vec::with_capacity(header_bytes.len());
        for raw in header_bytes {
            let height = expected_parent.height.checked_add(1)
                .ok_or(InclusionError::InvalidHeaderChain)?;
            let checked = CheckedHeader::check_header_at_claimed_height(height, raw, profile)?;
            if checked.parent() != expected_parent.id {
                return Err(InclusionError::InvalidHeaderChain);
            }
            expected_parent = checked.tip();
            headers.push(checked);
        }
        Ok(Self { anchor, headers })
    }

    pub const fn anchor(&self) -> Tip {
        self.anchor
    }
    pub fn headers(&self) -> &[CheckedHeader] {
        &self.headers
    }
    pub fn tip(&self) -> Tip {
        self.headers.last().expect("nonempty segment").tip()
    }
    /// Prove relative inclusion against the locally checked header Merkle
    /// commitment. Signatures, scripts, economic semantics remain unchecked.
    pub fn check_transaction_inclusion(
        &self,
        height: u64,
        raw_tx: &[u8],
        proof: &MerklePath,
    ) -> Result<IncludedTransaction, InclusionError> {
        let offset = height.checked_sub(self.anchor.height)
            .and_then(|difference| difference.checked_sub(1))
            .and_then(|index| usize::try_from(index).ok())
            .ok_or(InclusionError::InvalidHeaderChain)?;
        let header = self.headers.get(offset).ok_or(InclusionError::InvalidHeaderChain)?;
        header.check_transaction_inclusion(raw_tx, proof)
    }
}

/// Bitcoin's txid Merkle siblings in *SHA256d raw internal byte order*.
/// For odd-size levels, supply the exact last-leaf self-duplicate, but that
/// synthetic padding is not an actual duplicate transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerklePath {
    pub tx_index: u32,
    pub tx_count: u32,
    pub siblings_raw: Vec<[u8; 32]>,
}

/// Inclusion proved *relative to a locally checked header*.
/// The type is opaque outside this module to prevent caller construction.
/// It is still not a full consensus or economic authorization verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IncludedTransaction {
    height: u64,
    block: BlockId,
    txid_display: [u8; 32],
    tx_index: u32,
    tx_count: u32,
    /// Identified from a single legacy TL carrier with validated *syntax*.
    /// Parsing is NOT consensus-level sender/activation authorization.
    syntactic_type: Option<TransactionType>,
}

impl IncludedTransaction {
    pub const fn height(&self) -> u64 {
        self.height
    }
    pub const fn block_id(&self) -> BlockId {
        self.block
    }
    pub const fn txid_display(&self) -> [u8; 32] {
        self.txid_display
    }
    pub const fn tx_index(&self) -> u32 {
        self.tx_index
    }
    pub const fn tx_count(&self) -> u32 {
        self.tx_count
    }
    pub const fn legacy_type_hint(&self) -> Option<TransactionType> {
        self.syntactic_type
    }
}

impl CheckedHeader {
    pub fn check_transaction_inclusion(
        &self,
        raw_tx: &[u8],
        proof: &MerklePath,
    ) -> Result<IncludedTransaction, InclusionError> {
        if proof.tx_count == 0 || proof.tx_count > MAX_PROOF_TRANSACTIONS
            || proof.tx_index >= proof.tx_count
            || proof.siblings_raw.len() > MAX_PROOF_SIBLINGS
        {
            return Err(InclusionError::InvalidProof);
        }
        let inspected = inspect_transaction(raw_tx)?;
        let mut hash = inspected.txid; // SHA256d internal byte order; NOT wtxid.
        let mut index = proof.tx_index;
        let mut count = proof.tx_count;
        let mut level = 0;
        while count > 1 {
            let sibling = *proof.siblings_raw.get(level).ok_or(InclusionError::InvalidProof)?;
            let actual_sibling_exists = index % 2 == 1 || index + 1 < count;
            if !actual_sibling_exists {
                if sibling != hash {
                    return Err(InclusionError::InvalidProof);
                }
            } else if sibling == hash {
                // Equal real sibling roots imply duplicate-transaction
                // ambiguity as opposed to an odd-level padding duplicate.
                return Err(InclusionError::MutatedMerkleBranch);
            }
            let mut pair = [0u8; 64];
            if index % 2 == 0 {
                pair[..32].copy_from_slice(&hash);
                pair[32..].copy_from_slice(&sibling);
            } else {
                pair[..32].copy_from_slice(&sibling);
                pair[32..].copy_from_slice(&hash);
            }
            hash = double_sha256(&pair);
            index /= 2;
            count = (count + 1) / 2;
            level += 1;
        }
        if level != proof.siblings_raw.len() || hash != self.merkle_root {
            return Err(InclusionError::MerkleRootMismatch);
        }
        let syntactic_type = match inspected.carriers.as_slice() {
            [carrier]
                if carrier.format == CarrierFormat::LegacyAscii
                    && carrier.status == "typed-syntax" =>
            {
                carrier.transaction_type.and_then(|ty| TransactionType::try_from(ty).ok())
            }
            _ => None,
        };
        let mut txid_display = inspected.txid;
        txid_display.reverse();
        Ok(IncludedTransaction {
            height: self.height,
            block: self.id,
            txid_display,
            tx_index: proof.tx_index,
            tx_count: proof.tx_count,
            syntactic_type,
        })
    }
}
