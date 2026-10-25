//! Stage25O: first-input P2WPKH *candidate* origin for a raw type-9 transaction.
//!
//! Source: pinned tradelayer.js@dca0888b
//! src/txUtils.js:170-203 derives sender from vin[0]'s previous output;
//! src/txUtils.js:912-955 builds type9 with a SINGLE encoder prefix;
//! src/txEncoder.js:228-237 emits four-field "tl9" payload;
//! src/types.js:27-61 could DOUBLE-prefix type9 in its generic builder.
//!
//! Offline Bitcoin-only relative evidence: actual raw parent TXID/vout,
//! P2WPKH SHA256->RIPEMD160 commitment to the witness compressed pubkey,
//! strict bc1 Bech32-v0 claimed address, and exact BIP143 SIGHASH_ALL
//! digest of the spending transaction, plus two txid Merkle paths in one
//! *caller-anchored*, locally checked header segment. The raw type9 payload
//! must independently pass Stage25N's direct-only strict candidate parser.
//!
//! Neither ECDSA mathematical validity nor script execution is certified by
//! this Rust module; the public-only synthetic fixture is verified by system
//! OpenSSL in an independent Python CI test. Validity of the parent UTXO,
//! chainwork, full block, BIP141 witness commitment, first-input ownership,
//! historical sender/list-admin/activation, data origin and channel revision
//! remain external obligations. No authorization or ledger mutation API.

use crate::bitcoin_inclusion::{
    CheckedHeaderSegment, IncludedTransaction, InclusionError, MerklePath,
};
use crate::channel_type9_inclusion_audit::{
    inspect_raw_type9_candidate_unverified, Type9OriginError, UnverifiedType9Payload,
};
use crate::channel_witness_audit::{check_der_all, Cursor, StructuralChannelError};
use crate::hash160::hash160;
use crate::hash256::double_sha256;
use crate::network_profile::TradeLayerMainnet;
use crate::read_only::{inspect_transaction, ScanError, MAX_IO_COUNT, MAX_RAW_TRANSACTION};

pub const BIP143_P2WPKH_SIGHASH_ALL: u32 = 1;
const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P2wpkhSenderError {
    UnsupportedNetwork,
    NoncausalTransactionOrder,
    WrongFirstInput,
    OutOfRangeFundingVout,
    MissingWitness,
    NonemptyFirstScriptSig,
    InvalidWitnessStack,
    InvalidCompressedPubkey,
    WrongSignerCommitment,
    UnsupportedFundingScript,
    InvalidFundingAmount,
    WrongFundingPrevout,
    InvalidBech32Address,
    SenderAddressMismatch,
    InvalidBip66Signature(StructuralChannelError),
    HeaderOrMerkle(InclusionError),
    Transaction(ScanError),
    Type9Payload(Type9OriginError),
    SenderSignatureAndScriptNotVerified,
    HistoricalListAdministratorNotVerified,
    CanonicalStateRevisionUnspecified,
}
impl From<ScanError> for P2wpkhSenderError {
    fn from(value: ScanError) -> Self {
        Self::Transaction(value)
    }
}
impl From<StructuralChannelError> for P2wpkhSenderError {
    fn from(value: StructuralChannelError) -> Self {
        Self::InvalidBip66Signature(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedP2wpkhType9FirstInput {
    funding: IncludedTransaction,
    type9_spending: IncludedTransaction,
    pubkey: [u8; 33],
    sender_hash160: [u8; 20],
    expected_sender_address: String,
    digest_raw: [u8; 32],
    funding_satoshis: u64,
    payload: UnverifiedType9Payload,
}
impl UnverifiedP2wpkhType9FirstInput {
    pub const fn funding(&self) -> IncludedTransaction {
        self.funding
    }
    pub const fn type9_spending(&self) -> IncludedTransaction {
        self.type9_spending
    }
    pub const fn public_key(&self) -> [u8; 33] {
        self.pubkey
    }
    pub const fn address_hash160(&self) -> [u8; 20] {
        self.sender_hash160
    }
    pub fn claimed_sender_address(&self) -> &str {
        &self.expected_sender_address
    }
    pub const fn digest_raw(&self) -> [u8; 32] {
        self.digest_raw
    }
    pub const fn funding_satoshis(&self) -> u64 {
        self.funding_satoshis
    }
    pub fn type9_payload(&self) -> &UnverifiedType9Payload {
        &self.payload
    }
    pub const fn require_authenticated_sender(&self) -> Result<(), P2wpkhSenderError> {
        Err(P2wpkhSenderError::SenderSignatureAndScriptNotVerified)
    }
    pub const fn require_historical_admin_permission(&self) -> Result<(), P2wpkhSenderError> {
        Err(P2wpkhSenderError::HistoricalListAdministratorNotVerified)
    }
    pub const fn require_canonical_channel_revision(&self) -> Result<(), P2wpkhSenderError> {
        Err(P2wpkhSenderError::CanonicalStateRevisionUnspecified)
    }
}

fn bech32_polymod(data: &[u8]) -> u32 {
    let mut checksum: u32 = 1;
    for value in data {
        let top = checksum >> 25;
        checksum = ((checksum & 0x01ff_ffff) << 5) ^ u32::from(*value);
        for (i, generator) in [
            0x3b6a57b2u32,
            0x26508e6d,
            0x1ea119fa,
            0x3d4233dd,
            0x2a1462b3,
        ]
        .iter()
        .enumerate()
        {
            if (top >> i) & 1 == 1 {
                checksum ^= generator;
            }
        }
    }
    checksum
}

/// Strict Bitcoin MAINNET lowercase native v0 Bech32 P2WPKH subset.
/// BIP173 checksum, witness version and convertbits padding are checked.
/// It refuses P2SH, Litecoin HRPs, mixed/uppercase, Taproot/Bech32m.
fn decode_mainnet_p2wpkh(address: &str) -> Result<[u8; 20], P2wpkhSenderError> {
    if address.len() != 42 || !address.starts_with("bc1") {
        return Err(P2wpkhSenderError::InvalidBech32Address);
    }
    let mut values = Vec::with_capacity(3 + 1 + 39);
    // HRP expand "bc" per BIP173, then the witness data and checksum.
    values.extend_from_slice(&[3, 3, 0, 2, 3]);
    for ch in address[3..].bytes() {
        let digit = CHARSET
            .iter()
            .position(|value| *value == ch)
            .ok_or(P2wpkhSenderError::InvalidBech32Address)?;
        values.push(digit as u8);
    }
    if bech32_polymod(&values) != 1 {
        return Err(P2wpkhSenderError::InvalidBech32Address);
    }
    let data = &values[5..values.len() - 6];
    if data.len() != 33 || data[0] != 0 {
        return Err(P2wpkhSenderError::InvalidBech32Address);
    }
    let mut out = Vec::with_capacity(20);
    let mut acc: u32 = 0;
    let mut bits: usize = 0;
    for digit in &data[1..] {
        acc = ((acc << 5) | u32::from(*digit)) & 0x0fff;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 255) as u8);
        }
    }
    if out.len() != 20 || bits >= 5 || ((acc << (8 - bits)) & 255) != 0 {
        return Err(P2wpkhSenderError::InvalidBech32Address);
    }
    out.try_into()
        .map_err(|_| P2wpkhSenderError::InvalidBech32Address)
}

fn selected_funding_output(
    raw_funding: &[u8],
    vout: u32,
) -> Result<(u64, [u8; 20]), P2wpkhSenderError> {
    if raw_funding.len() > MAX_RAW_TRANSACTION {
        return Err(ScanError::TooLong.into());
    }
    let observation = inspect_transaction(raw_funding)?;
    let mut cur = Cursor::new(raw_funding);
    cur.take(4)?;
    if observation.has_witness {
        if cur.take(2)? != [0, 1] {
            return Err(ScanError::Invalid("invalid funding witness flag".into()).into());
        }
    }
    let inputs = cur.count()?;
    for _ in 0..inputs {
        cur.take(36)?;
        cur.data()?;
        cur.take(4)?;
    }
    let outputs = cur.count()?;
    let target = usize::try_from(vout).map_err(|_| ScanError::TooLong)?;
    if target >= outputs {
        return Err(P2wpkhSenderError::OutOfRangeFundingVout);
    }
    let mut selected = None;
    for index in 0..outputs {
        let value = u64::from_le_bytes(cur.take(8)?.try_into().expect("funding sat value"));
        let script = cur.data()?;
        if index == target {
            if value == 0 {
                return Err(P2wpkhSenderError::InvalidFundingAmount);
            }
            if script.len() != 22 || script[..2] != [0x00, 0x14] {
                return Err(P2wpkhSenderError::UnsupportedFundingScript);
            }
            let program: [u8; 20] = script[2..].try_into().expect("P2WPKH hash length");
            selected = Some((value, program));
        }
    }
    Ok(selected.expect("selected vout index already checked"))
}

fn compute_first_input_digest(
    raw_spend: &[u8],
    funding_txid_raw: [u8; 32],
    funding_vout: u32,
    amount_sats: u64,
    committed_pubkey_hash: [u8; 20],
) -> Result<([u8; 32], [u8; 33]), P2wpkhSenderError> {
    if raw_spend.len() > MAX_RAW_TRANSACTION {
        return Err(ScanError::TooLong.into());
    }
    let inspected = inspect_transaction(raw_spend)?;
    if !inspected.has_witness {
        return Err(P2wpkhSenderError::MissingWitness);
    }
    let mut cursor = Cursor::new(raw_spend);
    let version = cursor.take(4)?;
    if cursor.take(2)? != [0, 1] {
        return Err(P2wpkhSenderError::MissingWitness);
    }
    let inputs = cursor.count()?;
    let mut previous = Vec::with_capacity(inputs * 36);
    let mut sequences = Vec::with_capacity(inputs * 4);
    let mut first_prevout = [0u8; 36];
    let mut first_sequence = [0u8; 4];
    for index in 0..inputs {
        let prevout = cursor.take(36)?;
        let script_sig = cursor.data()?;
        let sequence = cursor.take(4)?;
        if index == 0 {
            if !script_sig.is_empty() {
                return Err(P2wpkhSenderError::NonemptyFirstScriptSig);
            }
            first_prevout.copy_from_slice(prevout);
            first_sequence.copy_from_slice(sequence);
        }
        previous.extend_from_slice(prevout);
        sequences.extend_from_slice(sequence);
    }
    let mut expected_prevout = [0u8; 36];
    expected_prevout[..32].copy_from_slice(&funding_txid_raw);
    expected_prevout[32..].copy_from_slice(&funding_vout.to_le_bytes());
    if first_prevout != expected_prevout {
        return Err(P2wpkhSenderError::WrongFundingPrevout);
    }
    let outputs = cursor.count()?;
    let mut raw_outputs = Vec::new();
    for _ in 0..outputs {
        let begin = cursor.offset();
        cursor.take(8)?;
        cursor.data()?;
        raw_outputs.extend_from_slice(&raw_spend[begin..cursor.offset()]);
    }
    let mut signer_key = None;
    for index in 0..inputs {
        let n = cursor.compact()?;
        if n > MAX_IO_COUNT {
            return Err(ScanError::TooLong.into());
        }
        if index == 0 && n != 2 {
            return Err(P2wpkhSenderError::InvalidWitnessStack);
        }
        for item in 0..n {
            let data = cursor.data()?;
            if index == 0 {
                if item == 0 {
                    check_der_all(data)?;
                } else {
                    if data.len() != 33 || !matches!(data[0], 0x02 | 0x03) {
                        return Err(P2wpkhSenderError::InvalidCompressedPubkey);
                    }
                    let key: [u8; 33] = data.try_into().expect("compressed key");
                    if hash160(&key) != committed_pubkey_hash {
                        return Err(P2wpkhSenderError::WrongSignerCommitment);
                    }
                    signer_key = Some(key);
                }
            }
        }
    }
    let locktime = cursor.take(4)?;
    if cursor.offset() != raw_spend.len() {
        return Err(ScanError::TrailingBytes.into());
    }
    // BIP143 P2WPKH scriptCode is a canonical 25-byte P2PKH script.
    let mut script = [0u8; 25];
    script[..3].copy_from_slice(&[0x76, 0xa9, 0x14]);
    script[3..23].copy_from_slice(&committed_pubkey_hash);
    script[23..].copy_from_slice(&[0x88, 0xac]);

    let h_prevouts = double_sha256(&previous);
    let h_sequences = double_sha256(&sequences);
    let h_outputs = double_sha256(&raw_outputs);
    let mut preimage = Vec::with_capacity(4 + 32 + 32 + 36 + 1 + 25 + 8 + 4 + 32 + 4 + 4);
    preimage.extend_from_slice(version);
    preimage.extend_from_slice(&h_prevouts);
    preimage.extend_from_slice(&h_sequences);
    preimage.extend_from_slice(&first_prevout);
    preimage.push(script.len() as u8);
    preimage.extend_from_slice(&script);
    preimage.extend_from_slice(&amount_sats.to_le_bytes());
    preimage.extend_from_slice(&first_sequence);
    preimage.extend_from_slice(&h_outputs);
    preimage.extend_from_slice(locktime);
    preimage.extend_from_slice(&BIP143_P2WPKH_SIGHASH_ALL.to_le_bytes());
    Ok((
        double_sha256(&preimage),
        signer_key.ok_or(P2wpkhSenderError::InvalidWitnessStack)?,
    ))
}

/// Narrow first-input sender-candidate binding under caller-anchored
/// Bitcoin-style relative transaction inclusion. Both TXIDs must occur
/// within the same locally checked segment, with funding before spend.
///
/// Even independently valid synthetic ECDSA does not authenticate a
/// historical list administrator or canonical off-chain channel update.
#[allow(clippy::too_many_arguments)]
pub fn audit_bitcoin_type9_p2wpkh_first_input_unverified(
    network: TradeLayerMainnet,
    segment: &CheckedHeaderSegment,
    raw_funding: &[u8],
    funding_height: u64,
    funding_vout: u32,
    funding_path: &MerklePath,
    raw_spending: &[u8],
    spending_height: u64,
    spending_path: &MerklePath,
    claimed_sender_address: &str,
) -> Result<UnverifiedP2wpkhType9FirstInput, P2wpkhSenderError> {
    if network != TradeLayerMainnet::Bitcoin {
        return Err(P2wpkhSenderError::UnsupportedNetwork);
    }
    if spending_height < funding_height
        || (spending_height == funding_height && spending_path.tx_index <= funding_path.tx_index)
    {
        return Err(P2wpkhSenderError::NoncausalTransactionOrder);
    }
    let funding = segment
        .check_transaction_inclusion(funding_height, raw_funding, funding_path)
        .map_err(P2wpkhSenderError::HeaderOrMerkle)?;
    let spending = segment
        .check_transaction_inclusion(spending_height, raw_spending, spending_path)
        .map_err(P2wpkhSenderError::HeaderOrMerkle)?;
    // Source getSender() takes vin[0], irrespective of other inputs.
    if inspect_transaction(raw_spending)?.input_count == 0 {
        return Err(P2wpkhSenderError::WrongFirstInput);
    }
    let payload = inspect_raw_type9_candidate_unverified(raw_spending)
        .map_err(P2wpkhSenderError::Type9Payload)?;
    let (amount_sats, committed_hash) = selected_funding_output(raw_funding, funding_vout)?;
    let funding_txid = inspect_transaction(raw_funding)?.txid;
    let (digest, key) = compute_first_input_digest(
        raw_spending,
        funding_txid,
        funding_vout,
        amount_sats,
        committed_hash,
    )?;
    let address_hash = decode_mainnet_p2wpkh(claimed_sender_address)?;
    if address_hash != committed_hash {
        return Err(P2wpkhSenderError::SenderAddressMismatch);
    }
    Ok(UnverifiedP2wpkhType9FirstInput {
        funding,
        type9_spending: spending,
        pubkey: key,
        sender_hash160: committed_hash,
        expected_sender_address: claimed_sender_address.to_owned(),
        digest_raw: digest,
        funding_satoshis: amount_sats,
        payload,
    })
}
