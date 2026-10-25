//! Stage25L: exact legacy P2SH SIGHASH_ALL digest, research-only.
//!
//! Historical Bitcoin/Litecoin P2SH multisig: double-SHA256 of the legacy
//! non-witness transaction, replacing only the selected input scriptSig with
//! its actual 71-byte redeemScript and emptying every other input scriptSig,
//! then appending little-endian SIGHASH_ALL (1).
//!
//! Structural P2SH/HASH160/outpoint/BIP66 checks run first. This digest
//! deliberately does NOT verify secp256k1 signatures, execute Script, prove
//! chain inclusion, UTXO existence, state revision, revocation or any
//! TradeLayer economic permission. Even a real synthetic ECDSA fixture
//! always returns SignaturesNotVerified to callers.

use crate::channel_p2sh_audit::{
    audit_legacy_p2sh_two_of_two_unverified, parse_two_signatures_and_redeem, P2shAuditError,
    UnverifiedP2shTwoOfTwoSpend,
};
use crate::channel_witness_audit::{ClaimedTwoOfTwoPubkeys, Cursor};
use crate::hash256::double_sha256;
use crate::network_profile::TradeLayerMainnet;
use crate::read_only::{inspect_transaction, ScanError, MAX_RAW_TRANSACTION};

pub const LEGACY_P2SH_SIGHASH_ALL: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedP2shLegacySighashDigest {
    structural: UnverifiedP2shTwoOfTwoSpend,
    digest_raw: [u8; 32],
}

impl UnverifiedP2shLegacySighashDigest {
    pub fn structural(&self) -> &UnverifiedP2shTwoOfTwoSpend {
        &self.structural
    }

    /// Raw SHA256d digest bytes, never reversed explorer display order.
    pub const fn digest_raw(&self) -> [u8; 32] {
        self.digest_raw
    }

    /// Hash reconstruction alone cannot grant signature or channel authority.
    pub const fn require_verified_channel_signatures(&self) -> Result<(), P2shAuditError> {
        Err(P2shAuditError::SignaturesNotVerified)
    }
}

fn legacy_sighash_all_unverified(
    raw_spending_transaction: &[u8],
    spending_vin: u32,
) -> Result<[u8; 32], P2shAuditError> {
    if raw_spending_transaction.len() > MAX_RAW_TRANSACTION {
        return Err(ScanError::TooLong.into());
    }
    if inspect_transaction(raw_spending_transaction)?.has_witness {
        return Err(P2shAuditError::UnsupportedSegwitSpend);
    }
    let mut cursor = Cursor::new(raw_spending_transaction);
    let version = cursor.take(4)?;
    let count_start = cursor.offset();
    let count = cursor.count()?;
    let encoded_count = &raw_spending_transaction[count_start..cursor.offset()];
    let selected = usize::try_from(spending_vin).map_err(|_| ScanError::TooLong)?;
    if selected >= count {
        return Err(P2shAuditError::SpendingVinOutOfRange);
    }

    let mut inputs = Vec::with_capacity(count);
    let mut selected_script = None;
    for index in 0..count {
        let outpoint = cursor.take(36)?;
        let script = cursor.data()?;
        let sequence = cursor.take(4)?;
        if index == selected {
            selected_script = Some(script);
        }
        inputs.push((outpoint, sequence));
    }
    let output_start = cursor.offset();
    let outputs = cursor.count()?;
    for _ in 0..outputs {
        cursor.take(8)?;
        cursor.data()?;
    }
    let output_end = cursor.offset();
    let locktime = cursor.take(4)?;
    if cursor.offset() != raw_spending_transaction.len() {
        return Err(ScanError::TrailingBytes.into());
    }

    // Stage25K structurally validated this exact scriptSig. Reuse its strict
    // parser to obtain the actual redeemScript; never trust caller scriptCode.
    let redeem =
        parse_two_signatures_and_redeem(selected_script.ok_or(P2shAuditError::InvalidScriptSig)?)?;
    // Fixed OP_2/PUSH33/PUSH33/OP_2/CHECKMULTISIG has no executable
    // OP_CODESEPARATOR or signature push to remove from legacy scriptCode.
    if redeem.len() != 71 {
        return Err(P2shAuditError::InvalidScriptSig);
    }
    let mut preimage = Vec::with_capacity(raw_spending_transaction.len().saturating_add(4));
    preimage.extend_from_slice(version);
    preimage.extend_from_slice(encoded_count);
    for (index, (outpoint, sequence)) in inputs.into_iter().enumerate() {
        preimage.extend_from_slice(outpoint);
        if index == selected {
            preimage.push(71);
            preimage.extend_from_slice(redeem);
        } else {
            preimage.push(0);
        }
        preimage.extend_from_slice(sequence);
    }
    preimage.extend_from_slice(&raw_spending_transaction[output_start..output_end]);
    preimage.extend_from_slice(locktime);
    preimage.extend_from_slice(&LEGACY_P2SH_SIGHASH_ALL.to_le_bytes());
    Ok(double_sha256(&preimage))
}

/// Read-only, legacy non-SegWit P2SH SIGHASH_ALL digest after Stage25K checks.
/// No caller-supplied funding amount enters this legacy digest: the referenced
/// funding TXID commits to the actual spent output indirectly.
pub fn audit_legacy_p2sh_sighash_all_unverified(
    network: TradeLayerMainnet,
    raw_funding_transaction: &[u8],
    funding_vout: u32,
    raw_spending_transaction: &[u8],
    spending_vin: u32,
    claimed_pubkeys: ClaimedTwoOfTwoPubkeys,
) -> Result<UnverifiedP2shLegacySighashDigest, P2shAuditError> {
    let structural = audit_legacy_p2sh_two_of_two_unverified(
        network,
        raw_funding_transaction,
        funding_vout,
        raw_spending_transaction,
        spending_vin,
        claimed_pubkeys,
    )?;
    let digest_raw = legacy_sighash_all_unverified(raw_spending_transaction, spending_vin)?;
    Ok(UnverifiedP2shLegacySighashDigest {
        structural,
        digest_raw,
    })
}
