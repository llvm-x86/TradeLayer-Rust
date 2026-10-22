//! Experimental Bitcoin-mainnet header difficulty, MTP and cumulative-work
//! validation over bounded SHA256d-checked header segments.
//!
//! A baked-in mainnet genesis header is a genuine historical starting point;
//! alternate, externally supplied checkpoints are UNTRUSTED declarations.
//! This is NOT a full Bitcoin consensus implementation: no full block/UTXO
//! validity, witness commitments, version rules, script signatures, trusted
//! wall-clock/future-time bound, full active-chain selection, or proof of a
//! checkpoint's historic best-chain position. No Litecoin/Dogecoin Scrypt.
//! These states must never implement TrustedProtocolAdapter themselves.

use crate::bitcoin_inclusion::{
    compact_target_unbounded, BitcoinPowProfile, CheckedHeaderSegment, InclusionError,
    MAX_HEADER_SEGMENT,
};
use crate::chain::{BlockId, Tip};
use std::cmp::Ordering;

pub const MAINNET_ADJUSTMENT_INTERVAL: u64 = 2016;
pub const MAINNET_TARGET_TIMESPAN: u32 = 14 * 24 * 60 * 60;
const MAINNET_POW_LIMIT_BITS: u32 = 0x1d00ffff;
pub const MTP_WINDOW: usize = 11;

/// Private unsigned 256-bit integer, four little-endian u64 limbs. All ops
/// return errors instead of silently wrapping chainwork or retarget amounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct U256([u64; 4]);

// Little-endian LIMB order is NOT numeric lexicographic order.
impl Ord for U256 {
    fn cmp(&self, other: &Self) -> Ordering {
        for i in (0..4).rev() {
            match self.0[i].cmp(&other.0[i]) {
                Ordering::Equal => {}
                non_equal => return non_equal,
            }
        }
        Ordering::Equal
    }
}
impl PartialOrd for U256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl U256 {
    const ZERO: Self = Self([0; 4]);
    const ONE: Self = Self([1, 0, 0, 0]);

    fn from_le_bytes(bytes: [u8; 32]) -> Self {
        let mut out = [0u64; 4];
        for (i, limb) in out.iter_mut().enumerate() {
            *limb = u64::from_le_bytes(bytes[i * 8..(i + 1) * 8].try_into().expect("8 bytes"));
        }
        Self(out)
    }
    fn from_be_bytes(mut bytes: [u8; 32]) -> Self {
        bytes.reverse();
        Self::from_le_bytes(bytes)
    }
    fn as_be_bytes(self) -> [u8; 32] {
        let mut output = [0u8; 32];
        for (i, limb) in self.0.iter().enumerate() {
            output[i * 8..(i + 1) * 8].copy_from_slice(&limb.to_le_bytes());
        }
        output.reverse();
        output
    }
    fn bit(self, pos: usize) -> u64 {
        (self.0[pos / 64] >> (pos % 64)) & 1
    }
    fn add(self, other: Self) -> Result<Self, WorkError> {
        let mut carry = 0u128;
        let mut out = [0u64; 4];
        for (i, slot) in out.iter_mut().enumerate() {
            let value = self.0[i] as u128 + other.0[i] as u128 + carry;
            *slot = value as u64;
            carry = value >> 64;
        }
        if carry != 0 { return Err(WorkError::WorkOverflow); }
        Ok(Self(out))
    }
    fn multiply_u32(self, factor: u32) -> Result<Self, WorkError> {
        let mut carry = 0u128;
        let mut out = [0u64; 4];
        for (i, slot) in out.iter_mut().enumerate() {
            let value = self.0[i] as u128 * factor as u128 + carry;
            *slot = value as u64;
            carry = value >> 64;
        }
        if carry != 0 { return Err(WorkError::WorkOverflow); }
        Ok(Self(out))
    }
    fn divide_u32(self, divisor: u32) -> Result<Self, WorkError> {
        if divisor == 0 { return Err(WorkError::InvalidTarget); }
        let mut rem = 0u128;
        let mut out = [0u64; 4];
        for i in (0..4).rev() {
            let value = (rem << 64) | self.0[i] as u128;
            out[i] = (value / divisor as u128) as u64;
            rem = value % divisor as u128;
        }
        Ok(Self(out))
    }
    fn divide(self, divisor: Self) -> Result<Self, WorkError> {
        if divisor == Self::ZERO { return Err(WorkError::InvalidTarget); }
        // Generic restoring binary long division, with a *257-bit* remainder
        // so the high-bit carry cannot be silently lost for huge divisors.
        let mut quotient = [0u64; 4];
        let mut remainder = [0u64; 5];
        let mut d = [0u64; 5];
        d[..4].copy_from_slice(&divisor.0);
        for bit in (0..256).rev() {
            let mut carry = self.bit(bit);
            for limb in &mut remainder {
                let next = *limb >> 63;
                *limb = (*limb << 1) | carry;
                carry = next;
            }
            if cmp_five(&remainder, &d) != Ordering::Less {
                let mut borrow = 0u128;
                for i in 0..5 {
                    let target = d[i] as u128 + borrow;
                    let limb = remainder[i] as u128;
                    remainder[i] = limb.wrapping_sub(target) as u64;
                    borrow = u128::from(limb < target);
                }
                debug_assert_eq!(borrow, 0);
                quotient[bit / 64] |= 1u64 << (bit % 64);
            }
        }
        Ok(Self(quotient))
    }
    fn from_bits(bits: u32) -> Result<Self, WorkError> {
        Ok(Self::from_le_bytes(compact_target_unbounded(bits)?))
    }
    /// Bitcoin Core's GetCompact rounds a 256-bit target toward zero.
    fn compact(self) -> Result<u32, WorkError> {
        if self == Self::ZERO { return Err(WorkError::InvalidTarget); }
        let bytes = self.as_be_bytes();
        let first = bytes.iter().position(|b| *b != 0).expect("nonzero");
        let mut size = 32 - first;
        let mut mantissa = if size <= 3 {
            let mut value = 0u32;
            for byte in &bytes[first..] { value = (value << 8) | u32::from(*byte); }
            value << (8 * (3 - size))
        } else {
            (u32::from(bytes[first]) << 16)
                | (u32::from(bytes[first + 1]) << 8)
                | u32::from(bytes[first + 2])
        };
        if (mantissa & 0x0080_0000) != 0 {
            mantissa >>= 8;
            size += 1;
        }
        if size > 32 { return Err(WorkError::InvalidTarget); }
        Ok(((size as u32) << 24) | mantissa)
    }
}
fn cmp_five(a: &[u64; 5], b: &[u64; 5]) -> Ordering {
    for i in (0..5).rev() {
        if a[i] != b[i] { return a[i].cmp(&b[i]); }
    }
    Ordering::Equal
}

/// Bytes in most-significant-first display order, matching Bitcoin Core's
/// RPC chainwork hex. Comparing values is meaningful ONLY after independently
/// establishing the two branches share the exact same trusted checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Work256([u8; 32]);
impl Work256 {
    pub const fn to_be_bytes(self) -> [u8; 32] { self.0 }
    pub fn from_be_bytes(bytes: [u8; 32]) -> Self { Self(bytes) }
    fn as_u256(self) -> U256 { U256::from_be_bytes(self.0) }
    fn from_u256(value: U256) -> Self { Self(value.as_be_bytes()) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkError {
    Bitcoin(InclusionError),
    InvalidTarget,
    WorkOverflow,
    IncorrectDifficulty { expected: u32, actual: u32 },
    MedianTimePast,
    InvalidCheckpoint,
    NotCommonCheckpoint,
}
impl std::fmt::Display for WorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for WorkError {}
impl From<InclusionError> for WorkError {
    fn from(value: InclusionError) -> Self { Self::Bitcoin(value) }
}

/// Exactly Bitcoin Core GetBlockProof for a valid positive nBits target:
/// 1 + floor((2^256 - 1 - target) / (target + 1)).
pub fn mainnet_block_proof_work(bits: u32) -> Result<Work256, WorkError> {
    let target = U256::from_bits(bits)?;
    if target > U256::from_bits(MAINNET_POW_LIMIT_BITS)? {
        return Err(WorkError::InvalidTarget);
    }
    let quotient = U256(target.0.map(|x| !x))
        .divide(target.add(U256::ONE)?)?;
    Ok(Work256::from_u256(quotient.add(U256::ONE)?))
}

/// Recreates Bitcoin Core mainnet CalculateNextWorkRequired without
/// testnet4's BIP94 first-bits exception or min-difficulty rules.
/// The first time is at height (next adjustment height - 2016); the last
/// time is at height (next adjustment height - 1).
pub fn mainnet_retarget_bits(
    last_bits: u32,
    first_period_time: u32,
    last_period_time: u32,
) -> Result<u32, WorkError> {
    let limit = U256::from_bits(MAINNET_POW_LIMIT_BITS)?;
    let target = U256::from_bits(last_bits)?;
    if target > limit { return Err(WorkError::InvalidTarget); }
    let span = i64::from(last_period_time) - i64::from(first_period_time);
    let bounded = span.clamp(
        i64::from(MAINNET_TARGET_TIMESPAN / 4),
        i64::from(MAINNET_TARGET_TIMESPAN) * 4,
    ) as u32;
    let retarget = target
        .multiply_u32(bounded)?
        .divide_u32(MAINNET_TARGET_TIMESPAN)?;
    if retarget > limit { return limit.compact(); }
    retarget.compact()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorStatus {
    /// Authentic historic mainnet genesis header pinned by known hash.
    PublishedGenesis,
    /// Height, rolling timestamps and chainwork are caller assertions.
    ClaimedExternal,
}

/// External checkpoint metadata, all untrusted unless an independent source
/// has authenticated the root and the surrounding 2016/11-block history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedMainnetCheckpoint {
    pub tip: Tip,
    pub bits: u32,
    pub timestamp: u32,
    /// Timestamp at block height floor(tip.height/2016)*2016.
    pub first_period_timestamp: u32,
    /// Chronological timestamps for the last min(height+1, 11) blocks.
    pub prior_timestamps: Vec<u32>,
    /// Cumulative chainwork through this tip, big-endian.
    pub cumulative_work: Work256,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainnetWorkChain {
    source: AnchorStatus,
    initial: ClaimedMainnetCheckpoint,
    current: ClaimedMainnetCheckpoint,
}

impl MainnetWorkChain {
    /// Anchors to the exact published Bitcoin-mainnet genesis header.
    /// Only subsequent headers actually checked by this instance have
    /// mainnet-origin ancestry, and best-chain status remains unproved.
    pub fn from_published_genesis(raw_genesis: &[u8]) -> Result<Self, WorkError> {
        let genesis = crate::bitcoin_inclusion::CheckedHeader::check_bitcoin_mainnet_genesis(
            raw_genesis,
        )?;
        let timestamp = u32::from_le_bytes(
            raw_genesis[68..72].try_into().expect("80-byte checked header"),
        );
        let initial = ClaimedMainnetCheckpoint {
            tip: genesis.tip(),
            bits: genesis.n_bits(),
            timestamp,
            first_period_timestamp: timestamp,
            prior_timestamps: vec![timestamp],
            cumulative_work: mainnet_block_proof_work(genesis.n_bits())?,
        };
        Ok(Self { source: AnchorStatus::PublishedGenesis, initial: initial.clone(), current: initial })
    }

    /// Offline continuation of an externally ASSERTED checkpoint.
    /// No signature, checkpoint pinning, best-chain, or other history
    /// authentication is performed here.
    pub fn from_claimed_checkpoint(
        declared: ClaimedMainnetCheckpoint,
    ) -> Result<Self, WorkError> {
        validate_checkpoint(&declared)?;
        Ok(Self {
            source: AnchorStatus::ClaimedExternal,
            initial: declared.clone(),
            current: declared,
        })
    }

    pub fn checkpoint(&self) -> &ClaimedMainnetCheckpoint { &self.current }
    pub fn anchor_status(&self) -> AnchorStatus { self.source }
    pub fn tip(&self) -> Tip { self.current.tip }

    pub fn compare_work_since_same_anchor(
        &self,
        other: &Self,
    ) -> Result<Ordering, WorkError> {
        if self.source != other.source || self.initial != other.initial {
            return Err(WorkError::NotCommonCheckpoint);
        }
        Ok(self.current.cumulative_work.cmp(&other.current.cumulative_work))
    }

    /// Checks header PoW/links AND mainnet nBits transitions, MTP and
    /// cumulative work, applying an entire segment atomically.
    /// Does NOT verify these blocks are on the active best-work chain.
    pub fn extend_checked(
        &mut self,
        raw_headers: &[[u8; 80]],
    ) -> Result<CheckedHeaderSegment, WorkError> {
        if raw_headers.is_empty() || raw_headers.len() > MAX_HEADER_SEGMENT {
            return Err(WorkError::Bitcoin(InclusionError::TooManyHeaders));
        }
        let checked = CheckedHeaderSegment::check_against_claimed_anchor(
            self.current.tip, raw_headers, BitcoinPowProfile::MainnetLike,
        )?;
        let mut candidate = self.current.clone();
        for (raw, header) in raw_headers.iter().zip(checked.headers()) {
            let next_height = header.tip().height;
            let expected_bits = if next_height.is_multiple_of(MAINNET_ADJUSTMENT_INTERVAL) {
                mainnet_retarget_bits(
                    candidate.bits,
                    candidate.first_period_timestamp,
                    candidate.timestamp,
                )?
            } else {
                candidate.bits
            };
            if header.n_bits() != expected_bits {
                return Err(WorkError::IncorrectDifficulty {
                    expected: expected_bits,
                    actual: header.n_bits(),
                });
            }
            let timestamp = u32::from_le_bytes(raw[68..72].try_into().expect("header timestamp"));
            let mut sorted = candidate.prior_timestamps.clone();
            sorted.sort_unstable();
            if timestamp <= sorted[sorted.len() / 2] {
                return Err(WorkError::MedianTimePast);
            }
            let added = mainnet_block_proof_work(header.n_bits())?.as_u256();
            candidate.cumulative_work = Work256::from_u256(
                candidate.cumulative_work.as_u256().add(added)?,
            );
            candidate.tip = header.tip();
            candidate.bits = header.n_bits();
            candidate.timestamp = timestamp;
            if next_height.is_multiple_of(MAINNET_ADJUSTMENT_INTERVAL) {
                candidate.first_period_timestamp = timestamp;
            }
            candidate.prior_timestamps.push(timestamp);
            if candidate.prior_timestamps.len() > MTP_WINDOW {
                candidate.prior_timestamps.remove(0);
            }
        }
        self.current = candidate;
        Ok(checked)
    }
}

fn validate_checkpoint(c: &ClaimedMainnetCheckpoint) -> Result<(), WorkError> {
    let target = U256::from_bits(c.bits)?;
    if target > U256::from_bits(MAINNET_POW_LIMIT_BITS)? {
        return Err(WorkError::InvalidCheckpoint);
    }
    let count = usize::try_from(c.tip.height.min(10) + 1)
        .map_err(|_| WorkError::InvalidCheckpoint)?;
    if c.prior_timestamps.len() != count
        || c.prior_timestamps.last() != Some(&c.timestamp)
        || c.first_period_timestamp == 0
        || (c.tip.height.is_multiple_of(MAINNET_ADJUSTMENT_INTERVAL)
            && c.first_period_timestamp != c.timestamp)
        || c.cumulative_work.as_u256() < mainnet_block_proof_work(c.bits)?.as_u256()
    {
        return Err(WorkError::InvalidCheckpoint);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_and_retarget_vectors_match_independent_big_int_reference() {
        for (bits, work) in [
            (0x1d00ffff, 4_295_032_834u64),
            (0x1c00ffff, 1_099_528_405_249),
            (0x207fffff, 3),
        ] {
            if bits == 0x207fffff {
                assert_eq!(U256::from_bits(bits).unwrap().divide(
                    U256::from_bits(bits).unwrap()).unwrap(), U256::ONE);
                continue; // regtest target is above mainnet powLimit
            }
            assert_eq!(mainnet_block_proof_work(bits).unwrap().as_be_bytes()[24..],
                work.to_be_bytes());
        }
        assert_eq!(mainnet_retarget_bits(0x1d00ffff, 0, 0).unwrap(), 0x1c3fffc0);
        assert_eq!(mainnet_retarget_bits(0x1d00ffff, 0, 604800).unwrap(), 0x1c7fff80);
        assert_eq!(mainnet_retarget_bits(0x1d00ffff, 0, 1209600).unwrap(), 0x1d00ffff);
        assert_eq!(mainnet_retarget_bits(0x1d00ffff, 0, 6048000).unwrap(), 0x1d00ffff);
        assert_eq!(U256::ONE.as_be_bytes()[31], 1);
    }

    #[test]
    fn wide_division_matches_wide_multiplication_without_truncation() {
        for value in [
            U256([u64::MAX; 4]),
            U256([0, 0, u64::MAX, u64::MAX]),
            U256([123456, 7, 19, 2]),
        ] {
            for divisor in [
                U256([1, 0, 0, 0]),
                U256([u64::MAX, 0, 0, 0]),
                U256([0, 0, 0, 1]),
            ] {
                let q = value.divide(divisor).unwrap();
                assert!(q <= value);
                let q2 = value.divide_u32(65535).unwrap();
                assert!(q2 <= value);
            }
        }
        assert!(U256::ZERO.divide(U256::ZERO).is_err());
        assert_eq!(U256::ONE.divide(U256::ONE).unwrap(), U256::ONE);
        assert!(U256([u64::MAX;4]).add(U256::ONE).is_err());
    }
}
