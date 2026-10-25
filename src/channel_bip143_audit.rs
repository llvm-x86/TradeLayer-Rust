//! Stage 25I: pure BIP143 SegWit-v0 SIGHASH_ALL commitment digest for the
//! native P2WSH Stage 25H *unverified* 2-of-2 research path.
//!
//! BIP143 spec: https://bitcoin.org/bip/143/
//!   dSHA256(nVersion || hashPrevouts || hashSequence || outpoint ||
//!           compactSize(scriptCode) || scriptCode || amount || nSequence ||
//!           hashOutputs || nLockTime || sighashType(4 bytes))
//!
//! This handles ONLY SIGHASH_ALL (0x01), with NO ANYONECANPAY, NONE, SINGLE,
//! OP_CODESEPARATOR, nested P2SH, Taproot, Dogecoin or script interpreter.
//! The canonical 71-byte OP_2 PUSH33 PUSH33 OP_2 CHECKMULTISIG script has
//! no CODESEPARATOR, so scriptCode is the entire actual witness script.
//!
//! Critically, computing the exact digest does NOT validate either ECDSA
//! signature. In particular, the fake DER signatures used in Stage 25H's
//! structural regression still produce a perfectly defined BIP143 digest.
//! No signer authorization, accepted-chain, revocation, type4/19 whitelist,
//! contract, margin, liquidation or trusted replay path is implemented.

use crate::channel_witness_audit::{
    audit_native_p2wsh_outpoint_unverified, ClaimedTwoOfTwoPubkeys, Cursor,
    StructuralChannelError, UnverifiedP2wshOutpointSpend,
};
use crate::hash256::double_sha256;
use crate::network_profile::TradeLayerMainnet;
use crate::read_only::{inspect_transaction, ScanError, MAX_IO_COUNT, MAX_RAW_TRANSACTION};

pub const BIP143_SIGHASH_ALL: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedP2wshBip143Digest {
    structural: UnverifiedP2wshOutpointSpend,
    hash_prevouts: [u8; 32],
    hash_sequence: [u8; 32],
    hash_outputs: [u8; 32],
    digest_raw: [u8; 32],
}

impl UnverifiedP2wshBip143Digest {
    pub fn structural(&self) -> &UnverifiedP2wshOutpointSpend {
        &self.structural
    }
    /// BIP143 digest in raw SHA256d byte order, not explorer display order.
    pub const fn digest_raw(&self) -> [u8; 32] {
        self.digest_raw
    }
    pub const fn hash_prevouts_raw(&self) -> [u8; 32] {
        self.hash_prevouts
    }
    pub const fn hash_sequence_raw(&self) -> [u8; 32] {
        self.hash_sequence
    }
    pub const fn hash_outputs_raw(&self) -> [u8; 32] {
        self.hash_outputs
    }
    /// The public read-only digest is NEVER a signature verification result.
    pub const fn require_verified_channel_signatures(&self) -> Result<(), StructuralChannelError> {
        Err(StructuralChannelError::SignaturesNotVerified)
    }
}

struct Commitments {
    prevouts: [u8; 32],
    sequence: [u8; 32],
    outputs: [u8; 32],
    digest: [u8; 32],
}

fn compute_bip143_sighash_all_unverified(
    raw_spending_transaction: &[u8],
    vin: u32,
    spent_amount_sats: u64,
) -> Result<Commitments, StructuralChannelError> {
    // Stage25H checks input, P2WSH script, two apparent SIGHASH_ALL
    // signatures and funding outpoint. This separate byte walk calculates
    // a digest over ORIGINAL serialized transaction bytes.
    if raw_spending_transaction.len() > MAX_RAW_TRANSACTION {
        return Err(ScanError::TooLong.into());
    }
    let inspected = inspect_transaction(raw_spending_transaction)?;
    if !inspected.has_witness {
        return Err(StructuralChannelError::NonNativeWitness);
    }
    let mut cursor = Cursor::new(raw_spending_transaction);
    let version = cursor.take(4)?;
    if cursor.take(2)? != [0, 1] {
        return Err(StructuralChannelError::NonNativeWitness);
    }
    let n_inputs = cursor.count()?;
    let selected = usize::try_from(vin).map_err(|_| ScanError::TooLong)?;
    if selected >= n_inputs {
        return Err(StructuralChannelError::InvalidInputIndex);
    }
    let mut prevouts = Vec::with_capacity(n_inputs * 36);
    let mut sequences = Vec::with_capacity(n_inputs * 4);
    let mut selected_outpoint = [0u8; 36];
    let mut selected_sequence = [0u8; 4];
    for i in 0..n_inputs {
        let outpoint = cursor.take(36)?;
        prevouts.extend_from_slice(outpoint);
        cursor.data()?; // scriptSig is NOT BIP143 scriptCode
        let sequence = cursor.take(4)?;
        sequences.extend_from_slice(sequence);
        if i == selected {
            selected_outpoint.copy_from_slice(outpoint);
            selected_sequence.copy_from_slice(sequence);
        }
    }
    let n_outputs = cursor.count()?;
    let mut outputs = Vec::new();
    for _ in 0..n_outputs {
        let start = cursor.offset();
        cursor.take(8)?; // output base-chain satoshis
        cursor.data()?; // CompactSize+scriptPubKey preserved byte-exact
        outputs.extend_from_slice(&raw_spending_transaction[start..cursor.offset()]);
    }
    let mut witness_script = None;
    for i in 0..n_inputs {
        let count = cursor.compact()?;
        if count > MAX_IO_COUNT {
            return Err(ScanError::TooLong.into());
        }
        for j in 0..count {
            let item = cursor.data()?;
            if i == selected && j + 1 == count {
                witness_script = Some(item);
            }
        }
    }
    let locktime = cursor.take(4)?;
    if cursor.offset() != raw_spending_transaction.len() {
        return Err(ScanError::TrailingBytes.into());
    }
    let script = witness_script.ok_or(StructuralChannelError::InvalidWitnessShape)?;
    // The Stage25H parser requires this fixed 2-of-2 script with no
    // CODESEPARATOR. Refuse future changes that alter scriptCode semantics.
    // A 0xab byte INSIDE a pushed compressed public key is data, not an
    // executed OP_CODESEPARATOR. Stage25H has already validated every
    // opcode of the fixed 2-of-2 script template.
    if script.len() != crate::channel_witness_audit::CANONICAL_COMPRESSED_2_OF_2_SCRIPT_SIZE {
        return Err(StructuralChannelError::InvalidWitnessScript);
    }
    let script_len = u8::try_from(script.len())
        .map_err(|_| StructuralChannelError::InvalidWitnessScript)?;
    let h_prevouts = double_sha256(&prevouts);
    let h_sequence = double_sha256(&sequences);
    let h_outputs = double_sha256(&outputs);

    // All preimage fields use canonical little-endian / raw hash order.
    // In this 2-of-2 subset, scriptCode CompactSize is one byte (71).
    let mut preimage = Vec::with_capacity(4 + 32 + 32 + 36 + 1 + 71 + 8 + 4 + 32 + 4 + 4);
    preimage.extend_from_slice(version);
    preimage.extend_from_slice(&h_prevouts);
    preimage.extend_from_slice(&h_sequence);
    preimage.extend_from_slice(&selected_outpoint);
    preimage.push(script_len);
    preimage.extend_from_slice(script);
    preimage.extend_from_slice(&spent_amount_sats.to_le_bytes());
    preimage.extend_from_slice(&selected_sequence);
    preimage.extend_from_slice(&h_outputs);
    preimage.extend_from_slice(locktime);
    preimage.extend_from_slice(&BIP143_SIGHASH_ALL.to_le_bytes());
    Ok(Commitments {
        prevouts: h_prevouts,
        sequence: h_sequence,
        outputs: h_outputs,
        digest: double_sha256(&preimage),
    })
}

/// Derive BIP143 SIGHASH_ALL only after Stage25H funding/spending
/// outpoint+witness structure succeeds. The spent amount is read from
/// the raw funding output and CANNOT be specified independently by caller.
///
/// The digest is UNVERIFIED: no ECC or script/chain validity is inferred.
pub fn audit_native_p2wsh_bip143_sighash_all_unverified(
    network: TradeLayerMainnet,
    raw_funding_transaction: &[u8],
    funding_vout: u32,
    raw_spending_transaction: &[u8],
    spending_vin: u32,
    claimed_pubkeys: ClaimedTwoOfTwoPubkeys,
) -> Result<UnverifiedP2wshBip143Digest, StructuralChannelError> {
    let structural = audit_native_p2wsh_outpoint_unverified(
        network,
        raw_funding_transaction,
        funding_vout,
        raw_spending_transaction,
        spending_vin,
        claimed_pubkeys,
    )?;
    let calculated = compute_bip143_sighash_all_unverified(
        raw_spending_transaction,
        spending_vin,
        structural.funding_satoshis(),
    )?;
    Ok(UnverifiedP2wshBip143Digest {
        structural,
        hash_prevouts: calculated.prevouts,
        hash_sequence: calculated.sequence,
        hash_outputs: calculated.outputs,
        digest_raw: calculated.digest,
    })
}
