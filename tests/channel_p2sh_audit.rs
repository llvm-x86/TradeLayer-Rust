//! Stage25K: P2SH historical channel structural audit (NO ECDSA check).
use tradelayer_core::channel_p2sh_audit::{
    audit_legacy_p2sh_two_of_two_unverified, P2shAuditError,
};
use tradelayer_core::channel_witness_audit::{ClaimedTwoOfTwoPubkeys, StructuralChannelError};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::read_only::{decode_hex, inspect_transaction, ScanError};

fn keys() -> ClaimedTwoOfTwoPubkeys {
    let x: [u8; 32] =
        decode_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
            .unwrap()
            .try_into()
            .unwrap();
    let mut a = [0u8; 33];
    a[0] = 2;
    a[1..].copy_from_slice(&x);
    let mut b = [0u8; 33];
    b[0] = 3;
    b[1..].copy_from_slice(&x);
    ClaimedTwoOfTwoPubkeys {
        participant_a: a,
        participant_b: b,
    }
}

fn redeem(reverse: bool) -> Vec<u8> {
    let keys = keys();
    let (first, second) = if reverse {
        (keys.participant_b, keys.participant_a)
    } else {
        (keys.participant_a, keys.participant_b)
    };
    let mut script = vec![0x52, 0x21];
    script.extend_from_slice(&first);
    script.push(0x21);
    script.extend_from_slice(&second);
    script.extend_from_slice(&[0x52, 0xae]);
    assert_eq!(script.len(), 71);
    script
}

fn hash160(reverse: bool) -> Vec<u8> {
    decode_hex(if reverse {
        "62c5a9dce49968f5ebf6691fb6fa43e1ebede59d"
    } else {
        "3fb5b5fbb96ad2b002a5ab6f3e05da931641c0f4"
    })
    .unwrap()
}

fn p2sh(reverse: bool) -> Vec<u8> {
    let mut script = vec![0xa9, 0x14];
    script.extend_from_slice(&hash160(reverse));
    script.push(0x87);
    script
}

fn direct(out: &mut Vec<u8>, item: &[u8]) {
    assert!(item.len() <= 75);
    out.push(item.len() as u8);
    out.extend_from_slice(item);
}

fn fake_der(r: u8, s: u8) -> Vec<u8> {
    // DER shape only, not a real signature to this spending tx.
    vec![0x30, 0x06, 0x02, 0x01, r, 0x02, 0x01, s, 0x01]
}

fn script_sig(reverse: bool) -> Vec<u8> {
    let mut script = vec![0x00]; // CHECKMULTISIG's mandatory OP_0
    direct(&mut script, &fake_der(1, 2));
    direct(&mut script, &fake_der(3, 4));
    direct(&mut script, &redeem(reverse));
    script
}

fn funding(script: &[u8], value: u64) -> Vec<u8> {
    let mut tx = 2u32.to_le_bytes().to_vec();
    tx.push(1); // one input
    tx.extend_from_slice(&[0x22; 32]); // synthetic dummy parent
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx.push(0); // scriptSig empty
    tx.extend_from_slice(&u32::MAX.to_le_bytes()); // sequence
    tx.push(1); // one output
    tx.extend_from_slice(&value.to_le_bytes());
    direct(&mut tx, script);
    tx.extend_from_slice(&0u32.to_le_bytes()); // locktime
    tx
}

fn spending(prevout_txid_raw: [u8; 32], vout: u32, sig_script: &[u8]) -> Vec<u8> {
    let mut tx = 2u32.to_le_bytes().to_vec();
    tx.push(2); // include unrelated input 0
    tx.extend_from_slice(&[0x33; 32]);
    tx.extend_from_slice(&8u32.to_le_bytes());
    tx.push(0);
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.extend_from_slice(&prevout_txid_raw);
    tx.extend_from_slice(&vout.to_le_bytes());
    // scriptSig length is ~90 bytes, requires canonical CompactSize with
    // one length byte, but script P2SH redeem is itself push71.
    assert!(sig_script.len() < 253);
    tx.push(sig_script.len() as u8);
    tx.extend_from_slice(sig_script);
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.push(1);
    tx.extend_from_slice(&5_000u64.to_le_bytes());
    direct(&mut tx, &[0x51]); // unrelated OP_TRUE output
    tx.extend_from_slice(&0u32.to_le_bytes()); // locktime
    tx
}

fn pair(reverse: bool) -> (Vec<u8>, Vec<u8>) {
    let fund = funding(&p2sh(reverse), 15_000);
    let spend = spending(
        inspect_transaction(&fund).unwrap().txid,
        0,
        &script_sig(reverse),
    );
    (fund, spend)
}

fn check(
    fund: &[u8],
    spend: &[u8],
    claimed: ClaimedTwoOfTwoPubkeys,
) -> Result<tradelayer_core::channel_p2sh_audit::UnverifiedP2shTwoOfTwoSpend, P2shAuditError> {
    audit_legacy_p2sh_two_of_two_unverified(TradeLayerMainnet::Litecoin, fund, 0, spend, 1, claimed)
}

#[test]
fn independently_hashed_2of2_redeem_spends_correct_p2sh_outpoint_without_auth() {
    let (fund, spend) = pair(false);
    let result = check(&fund, &spend, keys()).unwrap();
    assert_eq!(result.funding_vout(), 0);
    assert_eq!(result.spending_vin(), 1);
    assert_eq!(result.funding_satoshis(), 15_000);
    assert_eq!(result.network(), TradeLayerMainnet::Litecoin);
    assert_eq!(result.script_hash160().as_slice(), hash160(false));
    let mut expected_fund = inspect_transaction(&fund).unwrap().txid;
    expected_fund.reverse();
    assert_eq!(result.funding_txid_display(), expected_fund);
    let mut expected_spend = inspect_transaction(&spend).unwrap().txid;
    expected_spend.reverse();
    assert_eq!(result.spending_txid_display(), expected_spend);
    assert_eq!(
        result.require_verified_channel_signatures(),
        Err(P2shAuditError::SignaturesNotVerified)
    );
}

#[test]
fn historical_reversed_pubkey_order_changes_hash_but_both_remain_supported() {
    let (fund, spend) = pair(true);
    let result = check(&fund, &spend, keys()).unwrap();
    assert_eq!(result.script_hash160().as_slice(), hash160(true));
    assert_ne!(hash160(true), hash160(false));
    assert_eq!(result.claimed_pubkeys(), keys());
    assert_eq!(
        result.require_verified_channel_signatures(),
        Err(P2shAuditError::SignaturesNotVerified)
    );
}

#[test]
fn wrong_funding_prevout_or_other_vout_and_zero_funds_reject() {
    let (fund, spend) = pair(false);
    let mut txid = inspect_transaction(&fund).unwrap().txid;
    txid[0] ^= 1;
    let wrong = spending(txid, 0, &script_sig(false));
    assert_eq!(
        check(&fund, &wrong, keys()),
        Err(P2shAuditError::WrongPrevout)
    );
    let wrong = spending(
        inspect_transaction(&fund).unwrap().txid,
        1,
        &script_sig(false),
    );
    assert_eq!(
        check(&fund, &wrong, keys()),
        Err(P2shAuditError::WrongPrevout)
    );
    assert_eq!(
        audit_legacy_p2sh_two_of_two_unverified(
            TradeLayerMainnet::Litecoin,
            &fund,
            1,
            &spend,
            1,
            keys()
        ),
        Err(P2shAuditError::FundingVoutOutOfRange)
    );
    assert_eq!(
        audit_legacy_p2sh_two_of_two_unverified(
            TradeLayerMainnet::Litecoin,
            &fund,
            0,
            &spend,
            2,
            keys()
        ),
        Err(P2shAuditError::SpendingVinOutOfRange)
    );
    let empty_fund = funding(&p2sh(false), 0);
    let zero_spend = spending(
        inspect_transaction(&empty_fund).unwrap().txid,
        0,
        &script_sig(false),
    );
    assert_eq!(
        check(&empty_fund, &zero_spend, keys()),
        Err(P2shAuditError::InvalidOutputAmount)
    );
}

#[test]
fn a_changed_hash160_redeem_or_a_non_p2sh_output_fails_closed() {
    let (fund, _) = pair(false);
    let mut bad = p2sh(false);
    bad[2] ^= 1;
    let changed_fund = funding(&bad, 15_000);
    let changed_spend = spending(
        inspect_transaction(&changed_fund).unwrap().txid,
        0,
        &script_sig(false),
    );
    assert_eq!(
        check(&changed_fund, &changed_spend, keys()),
        Err(P2shAuditError::Hash160Mismatch)
    );
    let native = funding(&[0x00, 0x20], 15_000);
    let native_spend = spending(
        inspect_transaction(&native).unwrap().txid,
        0,
        &script_sig(false),
    );
    assert_eq!(
        check(&native, &native_spend, keys()),
        Err(P2shAuditError::NotP2shFunding)
    );
    let mut sig = script_sig(false);
    let last = sig.len() - 1;
    sig[last] ^= 1; // OP_CHECKMULTISIG becomes unknown
    let changed_spend = spending(inspect_transaction(&fund).unwrap().txid, 0, &sig);
    assert_eq!(
        check(&fund, &changed_spend, keys()),
        Err(P2shAuditError::RedeemScript(
            StructuralChannelError::InvalidWitnessScript
        ))
    );
}

#[test]
fn malformed_or_nonminimal_script_sig_cannot_be_used_as_authority() {
    let (fund, _) = pair(false);
    let txid = inspect_transaction(&fund).unwrap().txid;
    let mut sig = script_sig(false);
    sig[0] = 0x51; // OP_1 rather than CHECKMULTISIG dummy
    let sp = spending(txid, 0, &sig);
    assert_eq!(check(&fund, &sp, keys()), Err(P2shAuditError::InvalidDummy));
    let mut sig = script_sig(false);
    sig[1] = 0x4c; // nonminimal PUSHDATA1 opcode instead of PUSH9
    let sp = spending(txid, 0, &sig);
    assert_eq!(
        check(&fund, &sp, keys()),
        Err(P2shAuditError::NonCanonicalPush)
    );
    let mut sig = script_sig(false);
    let push_redeem_offset = 1 + 1 + fake_der(1, 2).len() + 1 + fake_der(3, 4).len();
    sig[push_redeem_offset] = 0x4c; // strict direct PUSH71 only
    let sp = spending(txid, 0, &sig);
    assert_eq!(
        check(&fund, &sp, keys()),
        Err(P2shAuditError::NonCanonicalPush)
    );
    let mut sig = script_sig(false);
    sig.push(0);
    let sp = spending(txid, 0, &sig);
    assert_eq!(
        check(&fund, &sp, keys()),
        Err(P2shAuditError::InvalidScriptSig)
    );
    let mut sig = script_sig(false);
    sig[5] = 0x80; // make first R negative
    let sp = spending(txid, 0, &sig);
    assert!(matches!(
        check(&fund, &sp, keys()),
        Err(P2shAuditError::Signature(
            StructuralChannelError::MalformedSignatureEncoding
        ))
    ));
}

#[test]
fn wrong_duplicate_or_uncompressed_keys_and_reused_fake_signatures_fail() {
    let (fund, _) = pair(false);
    let id = inspect_transaction(&fund).unwrap().txid;
    let sp = spending(id, 0, &script_sig(false));
    let mut same = keys();
    same.participant_b = same.participant_a;
    assert_eq!(
        check(&fund, &sp, same),
        Err(P2shAuditError::RedeemScript(
            StructuralChannelError::SameSignerPubkey
        ))
    );
    let mut wrong = keys();
    wrong.participant_a[1] ^= 1;
    assert_eq!(
        check(&fund, &sp, wrong),
        Err(P2shAuditError::RedeemScript(
            StructuralChannelError::MismatchedWitnessPubkeys
        ))
    );
    let mut uncompressed = keys();
    uncompressed.participant_a[0] = 4;
    assert_eq!(
        check(&fund, &sp, uncompressed),
        Err(P2shAuditError::RedeemScript(
            StructuralChannelError::InvalidCompressedPubkeys
        ))
    );
    let mut sig = vec![0];
    direct(&mut sig, &fake_der(1, 2));
    direct(&mut sig, &fake_der(1, 2));
    direct(&mut sig, &redeem(false));
    assert_eq!(
        check(&fund, &spending(id, 0, &sig), keys()),
        Err(P2shAuditError::DuplicateSignatures)
    );
}

#[test]
fn malformed_transaction_bytes_wrong_network_or_sighash_reject() {
    let (fund, spend) = pair(false);
    assert_eq!(
        audit_legacy_p2sh_two_of_two_unverified(
            TradeLayerMainnet::Dogecoin,
            &fund,
            0,
            &spend,
            1,
            keys()
        ),
        Err(P2shAuditError::UnsupportedNetwork)
    );
    // BTC and LTC share this script form, not their network consensus.
    assert!(audit_legacy_p2sh_two_of_two_unverified(
        TradeLayerMainnet::Bitcoin,
        &fund,
        0,
        &spend,
        1,
        keys()
    )
    .is_ok());
    let mut malformed = spend.clone();
    malformed.push(0);
    assert_eq!(
        check(&fund, &malformed, keys()),
        Err(P2shAuditError::RawTransaction(ScanError::TrailingBytes))
    );
    let mut nonminimal = fund.clone();
    nonminimal.splice(4..5, [0xfd, 1, 0]);
    assert_eq!(
        check(&nonminimal, &spend, keys()),
        Err(P2shAuditError::RawTransaction(
            ScanError::NonCanonicalCompactSize
        ))
    );
    let mut sig = script_sig(false);
    let sighash_offset = 1 + 1 + fake_der(1, 2).len() - 1;
    sig[sighash_offset] = 0x02;
    let alt = spending(inspect_transaction(&fund).unwrap().txid, 0, &sig);
    assert_eq!(
        check(&fund, &alt, keys()),
        Err(P2shAuditError::Signature(
            StructuralChannelError::UnsupportedSignatureHashType
        ))
    );
}

#[test]
fn independently_hashed_claim_has_no_script_execution_or_chain_authority() {
    let (fund, spend) = pair(false);
    let valid = check(&fund, &spend, keys()).unwrap();
    for _ in 0..3 {
        assert_eq!(
            valid.require_verified_channel_signatures(),
            Err(P2shAuditError::SignaturesNotVerified)
        );
    }
    // Both signatures in the fixture are FAKE but correctly DER-shaped.
    // There is no P2SH legacy sighash or secp256k1 ECDSA verifier here.
    // The result cannot be converted to Ledger, TLCC or liquidation input.
}
