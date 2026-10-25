//! Stage25K: *structural* legacy P2SH 2-of-2 script/outpoint audit.
//!
//! Historical source: tradelayer.js@dca0888b txUtils.js:1419–1490:
//! BTC uses bitcore-lib P2SH, LTC uses bitcore-lib-ltc P2SH, in addition
//! to the previously characterized native P2WSH branch.
//! BIP16 specifies OP_HASH160 PUSH20 hash OP_EQUAL and push-only scriptSig:
//! https://bitcoin.org/bip/16/
//!
//! Accepts only:
//!   funding scriptPubKey: OP_HASH160 PUSH20 HASH160(redeem) OP_EQUAL
//!   spending scriptSig:  OP_0 PUSH(sigA) PUSH(sigB) PUSH71(redeem)
//!   redeem: OP_2 PUSH33(pubA) PUSH33(pubB) OP_2 CHECKMULTISIG
//!
//! SHA256 and RIPEMD160 are local, research-tested primitives, NOT audited
//! production consensus crypto. DER shape is NOT ECDSA validity. No legacy
//! SIGHASH_ALL digest/signature verification, script interpreter, chain,
//! UTXO set, channel revocation, type4/19 eligibility or liquidation power.
//! The output always refuses real channel authorization.

use crate::channel_witness_audit::{
    check_der_all, check_script, ClaimedTwoOfTwoPubkeys, Cursor, StructuralChannelError,
    CANONICAL_COMPRESSED_2_OF_2_SCRIPT_SIZE,
};
use crate::hash160::hash160;
use crate::network_profile::TradeLayerMainnet;
use crate::read_only::{inspect_transaction, ScanError, MAX_RAW_TRANSACTION};

pub const P2SH_SCRIPT_PUBKEY_LENGTH: usize = 23;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P2shAuditError {
    RawTransaction(ScanError),
    UnsupportedNetwork,
    FundingVoutOutOfRange,
    SpendingVinOutOfRange,
    InvalidOutputAmount,
    NotP2shFunding,
    WrongPrevout,
    UnsupportedSegwitSpend,
    InvalidScriptSig,
    InvalidDummy,
    NonCanonicalPush,
    Signature(StructuralChannelError),
    RedeemScript(StructuralChannelError),
    Hash160Mismatch,
    DuplicateSignatures,
    SignaturesNotVerified,
}
impl From<ScanError> for P2shAuditError {
    fn from(value: ScanError) -> Self {
        Self::RawTransaction(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedP2shTwoOfTwoSpend {
    network: TradeLayerMainnet,
    funding_txid_display: [u8; 32],
    spending_txid_display: [u8; 32],
    funding_vout: u32,
    spending_vin: u32,
    funding_satoshis: u64,
    script_hash160: [u8; 20],
    signer_pubkeys: ClaimedTwoOfTwoPubkeys,
}

impl UnverifiedP2shTwoOfTwoSpend {
    pub const fn network(&self) -> TradeLayerMainnet { self.network }
    pub const fn funding_txid_display(&self) -> [u8; 32] { self.funding_txid_display }
    pub const fn spending_txid_display(&self) -> [u8; 32] { self.spending_txid_display }
    pub const fn funding_vout(&self) -> u32 { self.funding_vout }
    pub const fn spending_vin(&self) -> u32 { self.spending_vin }
    pub const fn funding_satoshis(&self) -> u64 { self.funding_satoshis }
    pub const fn script_hash160(&self) -> [u8; 20] { self.script_hash160 }
    pub const fn claimed_pubkeys(&self) -> ClaimedTwoOfTwoPubkeys { self.signer_pubkeys }
    pub const fn require_verified_channel_signatures(&self) -> Result<(), P2shAuditError> {
        Err(P2shAuditError::SignaturesNotVerified)
    }
}

struct FundingOutput<'a> {
    value: u64,
    script: &'a [u8],
}
struct SpendingInput<'a> {
    prevout: [u8; 36],
    script_sig: &'a [u8],
}

fn selected_funding_vout<'a>(tx: &'a [u8], target: u32) -> Result<FundingOutput<'a>, P2shAuditError> {
    if tx.len() > MAX_RAW_TRANSACTION { return Err(ScanError::TooLong.into()); }
    let inspected = inspect_transaction(tx)?;
    let mut cursor = Cursor::new(tx);
    cursor.take(4)?; // version
    if inspected.has_witness && cursor.take(2)? != [0, 1] {
        return Err(ScanError::Invalid("invalid witness marker".into()).into());
    }
    let n_inputs = cursor.count()?;
    for _ in 0..n_inputs {
        cursor.take(36)?;
        cursor.data()?;
        cursor.take(4)?;
    }
    let n_outputs = cursor.count()?;
    let vout = usize::try_from(target).map_err(|_| ScanError::TooLong)?;
    if vout >= n_outputs { return Err(P2shAuditError::FundingVoutOutOfRange); }
    let mut chosen = None;
    for i in 0..n_outputs {
        let value = u64::from_le_bytes(cursor.take(8)?.try_into().expect("8-byte sats"));
        let script = cursor.data()?;
        if i == vout { chosen = Some(FundingOutput { value, script }); }
    }
    Ok(chosen.expect("vout within checked output count"))
}

fn selected_spending_vin<'a>(tx: &'a [u8], target: u32) -> Result<SpendingInput<'a>, P2shAuditError> {
    if tx.len() > MAX_RAW_TRANSACTION { return Err(ScanError::TooLong.into()); }
    let inspected = inspect_transaction(tx)?;
    if inspected.has_witness { return Err(P2shAuditError::UnsupportedSegwitSpend); }
    let mut cursor = Cursor::new(tx);
    cursor.take(4)?; // version
    let n_inputs = cursor.count()?;
    let vin = usize::try_from(target).map_err(|_| ScanError::TooLong)?;
    if vin >= n_inputs { return Err(P2shAuditError::SpendingVinOutOfRange); }
    let mut chosen = None;
    for i in 0..n_inputs {
        let prevout = cursor.take(36)?.try_into().expect("36-byte outpoint");
        let script_sig = cursor.data()?;
        cursor.take(4)?; // sequence
        if i == vin { chosen = Some(SpendingInput { prevout, script_sig }); }
    }
    Ok(chosen.expect("vin within checked input count"))
}

fn parse_two_signatures_and_redeem(
    script_sig: &[u8],
) -> Result<(&[u8], &[u8], &[u8]), P2shAuditError> {
    // A canonical compressed two-key script is 71 bytes: direct OP_PUSH71,
    // not OP_PUSHDATA1/2/4. DER signatures also use direct minimal pushes.
    let mut reader = Cursor::new(script_sig);
    if reader.take(1)? != [0] {
        return Err(P2shAuditError::InvalidDummy);
    }
    let mut sigs = [None, None];
    for slot in &mut sigs {
        let opcode = reader.take(1)?[0];
        if !(9..=73).contains(&opcode) {
            return Err(P2shAuditError::NonCanonicalPush);
        }
        let sig = reader.take(usize::from(opcode))?;
        check_der_all(sig).map_err(P2shAuditError::Signature)?;
        *slot = Some(sig);
    }
    if reader.take(1)? != [CANONICAL_COMPRESSED_2_OF_2_SCRIPT_SIZE as u8] {
        return Err(P2shAuditError::NonCanonicalPush);
    }
    let redeem = reader.take(CANONICAL_COMPRESSED_2_OF_2_SCRIPT_SIZE)?;
    if reader.offset() != script_sig.len() {
        return Err(P2shAuditError::InvalidScriptSig);
    }
    let first = sigs[0].ok_or(P2shAuditError::InvalidScriptSig)?;
    let second = sigs[1].ok_or(P2shAuditError::InvalidScriptSig)?;
    if first == second {
        return Err(P2shAuditError::DuplicateSignatures);
    }
    Ok((first, second, redeem))
}

/// Research-only historical P2SH source path. This checks the real
/// serialized funding TXID and selected spending prevout, HASH160 of the
/// pushed redeemScript and strict minimal two-signature scriptSig shape.
/// It does NOT calculate a legacy signature hash or verify signatures.
pub fn audit_legacy_p2sh_two_of_two_unverified(
    network: TradeLayerMainnet,
    funding_tx: &[u8],
    funding_vout: u32,
    spending_tx: &[u8],
    spending_vin: u32,
    claimed_pubkeys: ClaimedTwoOfTwoPubkeys,
) -> Result<UnverifiedP2shTwoOfTwoSpend, P2shAuditError> {
    if !matches!(network, TradeLayerMainnet::Bitcoin | TradeLayerMainnet::Litecoin) {
        return Err(P2shAuditError::UnsupportedNetwork);
    }
    let fund = inspect_transaction(funding_tx)?;
    let spend = inspect_transaction(spending_tx)?;
    let selected_fund = selected_funding_vout(funding_tx, funding_vout)?;
    if selected_fund.value == 0 {
        return Err(P2shAuditError::InvalidOutputAmount);
    }
    let spk = selected_fund.script;
    if spk.len() != P2SH_SCRIPT_PUBKEY_LENGTH
        || spk[0] != 0xa9
        || spk[1] != 0x14
        || spk[22] != 0x87
    {
        return Err(P2shAuditError::NotP2shFunding);
    }
    let selected_spend = selected_spending_vin(spending_tx, spending_vin)?;
    let mut expected = [0u8; 36];
    expected[..32].copy_from_slice(&fund.txid);
    expected[32..].copy_from_slice(&funding_vout.to_le_bytes());
    if selected_spend.prevout != expected {
        return Err(P2shAuditError::WrongPrevout);
    }
    let (_sig_a, _sig_b, redeem) =
        parse_two_signatures_and_redeem(selected_spend.script_sig)?;
    check_script(redeem, claimed_pubkeys).map_err(P2shAuditError::RedeemScript)?;
    let digest = hash160(redeem);
    if digest != spk[2..22] {
        return Err(P2shAuditError::Hash160Mismatch);
    }
    let mut funding_txid_display = fund.txid;
    let mut spending_txid_display = spend.txid;
    funding_txid_display.reverse();
    spending_txid_display.reverse();
    Ok(UnverifiedP2shTwoOfTwoSpend {
        network,
        funding_txid_display,
        spending_txid_display,
        funding_vout,
        spending_vin,
        funding_satoshis: selected_fund.value,
        script_hash160: digest,
        signer_pubkeys: claimed_pubkeys,
    })
}
