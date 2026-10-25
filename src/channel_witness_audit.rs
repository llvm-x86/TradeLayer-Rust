//! Stage 25H: BOUNDED, READ-ONLY structural 2-of-2 P2WSH UTXO audit.
//!
//! Historical source: tradelayer.js@dca0888b, txUtils.js:1419–1490
//! (createMultisig chooses p2wsh or p2sh), channels.js:754–792
//! (derive 2-of-2 channel address), validity.js:473–653 (type-4 commit).
//!
//! Given serialized funding and spending transactions, this module checks
//! - the true SHA256d funding TXID and an outpoint reference in spend bytes;
//! - a native P2WSH funding scriptPubKey's SHA256 witness-script commitment;
//! - exactly two DISTINCT expected compressed pubkeys in a canonical 2-of-2
//!   CHECKMULTISIG witness script, in either order;
//! - a native SegWit witness with empty CHECKMULTISIG dummy and two
//!   BIP66-shaped DER + SIGHASH_ALL byte sequences.
//!
//! NONE OF THIS VERIFIES ECDSA, script execution, chain inclusion, best-work
//! ancestry, a channel update's state/revocation history, type-4 whitelist
//! eligibility, a real TradeLayer commitment, or economic settlement.
//! Syntactically valid signatures can be fake! The returned opaque status
//! is ALWAYS Unverified and its authorization method ALWAYS DENIES.
//! Unsupported P2SH (HASH160, which is not audited in this crate), nested
//! P2SH-P2WSH, noncanonical scripts and Dogecoin are explicitly rejected.
//!
//! No new crates or build scripts. The existing internal SHA-256 routine has
//! vectors but is not an independently audited cryptographic TCB.

use crate::hash256::sha256;
use crate::network_profile::TradeLayerMainnet;
use crate::read_only::{inspect_transaction, ScanError, MAX_IO_COUNT, MAX_RAW_TRANSACTION};

pub const CANONICAL_P2WSH_OUTPUT_SIZE: usize = 34;
pub const CANONICAL_COMPRESSED_2_OF_2_SCRIPT_SIZE: usize = 71;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralChannelError {
    RawTransaction(ScanError),
    UnsupportedNetwork,
    InvalidOutputIndex,
    InvalidInputIndex,
    InvalidOutputValue,
    UnsupportedFundingScript,
    InvalidCompressedPubkeys,
    SameSignerPubkey,
    WrongPrevout,
    NonNativeWitness,
    NonemptySpendingScriptSig,
    InvalidWitnessShape,
    InvalidWitnessScript,
    MismatchedWitnessPubkeys,
    WitnessCommitmentMismatch,
    MalformedSignatureEncoding,
    UnsupportedSignatureHashType,
    SignaturesNotVerified,
}

impl From<ScanError> for StructuralChannelError {
    fn from(value: ScanError) -> Self {
        Self::RawTransaction(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimedTwoOfTwoPubkeys {
    pub participant_a: [u8; 33],
    pub participant_b: [u8; 33],
}

impl ClaimedTwoOfTwoPubkeys {
    fn check(self) -> Result<(), StructuralChannelError> {
        if ![self.participant_a, self.participant_b]
            .iter()
            .all(|pk| matches!(pk[0], 0x02 | 0x03) && pk[1..].iter().any(|b| *b != 0))
        {
            return Err(StructuralChannelError::InvalidCompressedPubkeys);
        }
        if self.participant_a == self.participant_b {
            return Err(StructuralChannelError::SameSignerPubkey);
        }
        Ok(())
    }
}

/// No public constructor for an apparently checked observation. This value
/// never provides a token or a trusted bridge into ledger execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedP2wshOutpointSpend {
    network: TradeLayerMainnet,
    funding_txid_display: [u8; 32],
    funding_vout: u32,
    spending_txid_display: [u8; 32],
    spending_vin: u32,
    funding_satoshis: u64,
    script_sha256: [u8; 32],
    script_pubkeys: ClaimedTwoOfTwoPubkeys,
}

impl UnverifiedP2wshOutpointSpend {
    pub const fn network(&self) -> TradeLayerMainnet {
        self.network
    }
    pub const fn funding_vout(&self) -> u32 {
        self.funding_vout
    }
    pub const fn spending_vin(&self) -> u32 {
        self.spending_vin
    }
    pub const fn funding_satoshis(&self) -> u64 {
        self.funding_satoshis
    }
    pub const fn funding_txid_display(&self) -> [u8; 32] {
        self.funding_txid_display
    }
    pub const fn spending_txid_display(&self) -> [u8; 32] {
        self.spending_txid_display
    }
    pub const fn script_sha256(&self) -> [u8; 32] {
        self.script_sha256
    }
    pub const fn claimed_pubkeys(&self) -> ClaimedTwoOfTwoPubkeys {
        self.script_pubkeys
    }
    pub const fn require_verified_channel_signatures(&self) -> Result<(), StructuralChannelError> {
        Err(StructuralChannelError::SignaturesNotVerified)
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ScanError> {
        let end = self.position.checked_add(n).ok_or(ScanError::TooLong)?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or(ScanError::Truncated)?;
        self.position = end;
        Ok(slice)
    }

    fn compact(&mut self) -> Result<u64, ScanError> {
        let code = self.take(1)?[0];
        let value = match code {
            0..=252 => u64::from(code),
            253 => {
                let n = u16::from_le_bytes(self.take(2)?.try_into().expect("two bytes"));
                if n < 253 {
                    return Err(ScanError::NonCanonicalCompactSize);
                }
                u64::from(n)
            }
            254 => {
                let n = u32::from_le_bytes(self.take(4)?.try_into().expect("four bytes"));
                if n <= u16::MAX as u32 {
                    return Err(ScanError::NonCanonicalCompactSize);
                }
                u64::from(n)
            }
            255 => {
                let n = u64::from_le_bytes(self.take(8)?.try_into().expect("eight bytes"));
                if n <= u32::MAX as u64 {
                    return Err(ScanError::NonCanonicalCompactSize);
                }
                n
            }
        };
        Ok(value)
    }

    fn count(&mut self) -> Result<usize, ScanError> {
        let count = self.compact()?;
        if count == 0 || count > MAX_IO_COUNT {
            return Err(ScanError::Invalid("invalid input/output count".into()));
        }
        usize::try_from(count).map_err(|_| ScanError::TooLong)
    }

    fn data(&mut self) -> Result<&'a [u8], ScanError> {
        let len = self.compact()?;
        if len > MAX_RAW_TRANSACTION as u64 {
            return Err(ScanError::TooLong);
        }
        self.take(usize::try_from(len).map_err(|_| ScanError::TooLong)?)
    }
}

#[derive(Debug)]
struct SelectedInput<'a> {
    parent_txid_raw: [u8; 32],
    parent_vout: u32,
    script_sig: &'a [u8],
    witness: Vec<&'a [u8]>,
}

#[derive(Debug)]
struct SelectedOutput<'a> {
    value_sats: u64,
    script_pubkey: &'a [u8],
}

/// Parse strictly complete bytes, selecting only requested input/output.
/// Does not import or mutate the Ledger, and never constructs a chain proof.
fn selected_transaction<'a>(
    raw: &'a [u8],
    requested_vin: u32,
    requested_vout: u32,
) -> Result<(SelectedInput<'a>, SelectedOutput<'a>, bool), StructuralChannelError> {
    if raw.len() > MAX_RAW_TRANSACTION {
        return Err(ScanError::TooLong.into());
    }
    // Reuse the existing offline tx/wtxid parser so syntactic agreement is
    // checked independently of these selected input/output offsets.
    let observation = inspect_transaction(raw)?;
    let mut cursor = Cursor::new(raw);
    cursor.take(4)?; // version
    let has_witness = raw.get(4) == Some(&0);
    if has_witness {
        if cursor.take(2)? != [0, 1] {
            return Err(ScanError::Invalid("invalid witness marker".into()).into());
        }
    }
    let n_inputs = cursor.count()?;
    let vin = usize::try_from(requested_vin).map_err(|_| ScanError::TooLong)?;
    if vin >= n_inputs {
        return Err(StructuralChannelError::InvalidInputIndex);
    }
    let mut selected_input = None;
    for i in 0..n_inputs {
        let parent_txid_raw = cursor.take(32)?.try_into().expect("32-byte TXID");
        let parent_vout = u32::from_le_bytes(cursor.take(4)?.try_into().expect("vout"));
        let script_sig = cursor.data()?;
        cursor.take(4)?; // sequence
        if i == vin {
            selected_input = Some(SelectedInput {
                parent_txid_raw,
                parent_vout,
                script_sig,
                witness: Vec::new(),
            });
        }
    }
    let n_outputs = cursor.count()?;
    let vout = usize::try_from(requested_vout).map_err(|_| ScanError::TooLong)?;
    if vout >= n_outputs {
        return Err(StructuralChannelError::InvalidOutputIndex);
    }
    let mut selected_output = None;
    for i in 0..n_outputs {
        let value_sats = u64::from_le_bytes(cursor.take(8)?.try_into().expect("satoshis"));
        let script_pubkey = cursor.data()?;
        if i == vout {
            selected_output = Some(SelectedOutput {
                value_sats,
                script_pubkey,
            });
        }
    }
    if has_witness {
        for i in 0..n_inputs {
            let stack_count = cursor.compact()?;
            if stack_count > MAX_IO_COUNT {
                return Err(ScanError::TooLong.into());
            }
            for _ in 0..stack_count {
                let bytes = cursor.data()?;
                if i == vin {
                    selected_input
                        .as_mut()
                        .expect("selected input index in bound")
                        .witness
                        .push(bytes);
                }
            }
        }
    }
    cursor.take(4)?; // locktime
    if cursor.position != raw.len()
        || observation.input_count != u32::try_from(n_inputs).expect("bounded")
        || observation.output_count != u32::try_from(n_outputs).expect("bounded")
        || observation.has_witness != has_witness
    {
        return Err(ScanError::TrailingBytes.into());
    }
    Ok((
        selected_input.expect("selected input index in bound"),
        selected_output.expect("selected output index in bound"),
        has_witness,
    ))
}

fn check_der_all(sig: &[u8]) -> Result<(), StructuralChannelError> {
    // BIP66 DER, with the sighash byte included in the item but EXCLUDED
    // from the DER sequence length. Positive minimally encoded R/S.
    if !(9..=73).contains(&sig.len())
        || sig[0] != 0x30
        || usize::from(sig[1]) != sig.len() - 3
        || sig[2] != 0x02
    {
        return Err(StructuralChannelError::MalformedSignatureEncoding);
    }
    let r_len = usize::from(sig[3]);
    if r_len == 0 || 5 + r_len >= sig.len() {
        return Err(StructuralChannelError::MalformedSignatureEncoding);
    }
    let s_tag = 4 + r_len;
    let s_len_index = 5 + r_len;
    if sig[s_tag] != 0x02 {
        return Err(StructuralChannelError::MalformedSignatureEncoding);
    }
    let s_len = usize::from(sig[s_len_index]);
    if s_len == 0 || r_len + s_len + 7 != sig.len() {
        return Err(StructuralChannelError::MalformedSignatureEncoding);
    }
    let r_start = 4;
    let s_start = 6 + r_len;
    if sig[r_start] & 0x80 != 0
        || (r_len > 1 && sig[r_start] == 0 && sig[r_start + 1] & 0x80 == 0)
        || sig[s_start] & 0x80 != 0
        || (s_len > 1 && sig[s_start] == 0 && sig[s_start + 1] & 0x80 == 0)
    {
        return Err(StructuralChannelError::MalformedSignatureEncoding);
    }
    if sig[sig.len() - 1] != 0x01 {
        return Err(StructuralChannelError::UnsupportedSignatureHashType);
    }
    Ok(())
}

fn check_script(
    script: &[u8],
    claimed: ClaimedTwoOfTwoPubkeys,
) -> Result<(), StructuralChannelError> {
    claimed.check()?;
    if script.len() != CANONICAL_COMPRESSED_2_OF_2_SCRIPT_SIZE
        || script[0] != 0x52
        || script[1] != 0x21
        || script[35] != 0x21
        || script[69] != 0x52
        || script[70] != 0xae
    {
        return Err(StructuralChannelError::InvalidWitnessScript);
    }
    let first: [u8; 33] = script[2..35].try_into().expect("pubkey");
    let second: [u8; 33] = script[36..69].try_into().expect("pubkey");
    if first == second {
        return Err(StructuralChannelError::SameSignerPubkey);
    }
    if !((first == claimed.participant_a && second == claimed.participant_b)
        || (first == claimed.participant_b && second == claimed.participant_a))
    {
        return Err(StructuralChannelError::MismatchedWitnessPubkeys);
    }
    Ok(())
}

/// Source-appropriate P2WSH subset. Byte structure and script hashes are
/// checked against RAW serialized transactions, not caller-supplied pubkey
/// flags. The funding TXID in the spending outpoint is independently
/// calculated from the raw non-witness funding transaction.
///
/// ECDSA and sighash digests are NOT verified, nor is the signed spend
/// authenticated as mined or included. A syntactically plausible unsigned
/// spend with two *fake* DER sequences can pass: the result is UNVERIFIED.
pub fn audit_native_p2wsh_outpoint_unverified(
    network: TradeLayerMainnet,
    raw_funding_transaction: &[u8],
    funding_vout: u32,
    raw_spending_transaction: &[u8],
    spending_vin: u32,
    claimed_pubkeys: ClaimedTwoOfTwoPubkeys,
) -> Result<UnverifiedP2wshOutpointSpend, StructuralChannelError> {
    if !matches!(
        network,
        TradeLayerMainnet::Bitcoin | TradeLayerMainnet::Litecoin
    ) {
        return Err(StructuralChannelError::UnsupportedNetwork);
    }
    claimed_pubkeys.check()?;
    let funding_observation = inspect_transaction(raw_funding_transaction)?;
    let (_, funding_output, _) = selected_transaction(raw_funding_transaction, 0, funding_vout)?;
    if funding_output.value_sats == 0 {
        return Err(StructuralChannelError::InvalidOutputValue);
    }
    let spk = funding_output.script_pubkey;
    if spk.len() != CANONICAL_P2WSH_OUTPUT_SIZE || spk[0] != 0x00 || spk[1] != 0x20 {
        return Err(StructuralChannelError::UnsupportedFundingScript);
    }
    let spending_observation = inspect_transaction(raw_spending_transaction)?;
    let (spend, _, witness) = selected_transaction(raw_spending_transaction, spending_vin, 0)?;
    if spend.parent_txid_raw != funding_observation.txid || spend.parent_vout != funding_vout {
        return Err(StructuralChannelError::WrongPrevout);
    }
    if !witness {
        return Err(StructuralChannelError::NonNativeWitness);
    }
    if !spend.script_sig.is_empty() {
        return Err(StructuralChannelError::NonemptySpendingScriptSig);
    }
    if spend.witness.len() != 4 || !spend.witness[0].is_empty() {
        return Err(StructuralChannelError::InvalidWitnessShape);
    }
    let script = spend.witness[3];
    check_script(script, claimed_pubkeys)?;
    if sha256(script) != spk[2..34] {
        return Err(StructuralChannelError::WitnessCommitmentMismatch);
    }
    check_der_all(spend.witness[1])?;
    check_der_all(spend.witness[2])?;
    if spend.witness[1] == spend.witness[2] {
        return Err(StructuralChannelError::InvalidWitnessShape);
    }
    let mut funding_txid_display = funding_observation.txid;
    funding_txid_display.reverse();
    let mut spending_txid_display = spending_observation.txid;
    spending_txid_display.reverse();
    Ok(UnverifiedP2wshOutpointSpend {
        network,
        funding_txid_display,
        funding_vout,
        spending_txid_display,
        spending_vin,
        funding_satoshis: funding_output.value_sats,
        script_sha256: sha256(script),
        script_pubkeys: claimed_pubkeys,
    })
}
