//! Stage 25H offline raw UTXO/witness-script commitment audits.
//! Syntactically DER-looking fake signatures deliberately pass inspection,
//! but the caller can NEVER infer actual ECDSA validity or authorization.
use tradelayer_core::channel_bip143_audit::audit_native_p2wsh_bip143_sighash_all_unverified;
use tradelayer_core::channel_witness_audit::{
    audit_native_p2wsh_outpoint_unverified, ClaimedTwoOfTwoPubkeys, StructuralChannelError,
    CANONICAL_COMPRESSED_2_OF_2_SCRIPT_SIZE,
};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::read_only::{decode_hex, inspect_transaction, ScanError};

const PROGRAM: [u8; 32] = [
    0xc8, 0xe2, 0x0d, 0x4b, 0x82, 0xf2, 0xef, 0x13, 0x27, 0x35, 0xe3, 0xdb, 0x61, 0x53, 0x93, 0xc5,
    0xc8, 0xa1, 0xa9, 0x56, 0x44, 0x05, 0xfd, 0x96, 0x08, 0x46, 0x6a, 0x89, 0x3f, 0xca, 0xb6, 0x8f,
];

fn keys() -> ClaimedTwoOfTwoPubkeys {
    // Two distinct compressed secp256k1 generator encodings (G and -G).
    // Encoding is not a proof of private-key possession.
    let x: [u8; 32] =
        decode_hex("79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798")
            .unwrap()
            .try_into()
            .unwrap();
    let mut a = [0u8; 33];
    a[0] = 0x02;
    a[1..].copy_from_slice(&x);
    let mut b = [0u8; 33];
    b[0] = 0x03;
    b[1..].copy_from_slice(&x);
    ClaimedTwoOfTwoPubkeys {
        participant_a: a,
        participant_b: b,
    }
}

fn script() -> Vec<u8> {
    let signers = keys();
    let mut bytes = vec![0x52, 0x21];
    bytes.extend_from_slice(&signers.participant_a);
    bytes.push(0x21);
    bytes.extend_from_slice(&signers.participant_b);
    bytes.extend_from_slice(&[0x52, 0xae]);
    assert_eq!(bytes.len(), CANONICAL_COMPRESSED_2_OF_2_SCRIPT_SIZE);
    bytes
}

fn script_pubkey() -> Vec<u8> {
    let mut out = vec![0x00, 0x20];
    out.extend_from_slice(&PROGRAM);
    out
}

fn push_data(out: &mut Vec<u8>, data: &[u8]) {
    assert!(data.len() <= 252);
    out.push(data.len() as u8);
    out.extend_from_slice(data);
}

/// Synthetic Bitcoin-style funding bytes, without chain inclusion proof.
fn funding(script_pubkey: &[u8], amount: u64) -> Vec<u8> {
    let mut tx = Vec::new();
    tx.extend_from_slice(&2u32.to_le_bytes()); // version
    tx.push(1); // one input
    tx.extend_from_slice(&[0x11; 32]); // invented parent txid
    tx.extend_from_slice(&1u32.to_le_bytes()); // index
    push_data(&mut tx, &[]); // empty scriptSig
    tx.extend_from_slice(&u32::MAX.to_le_bytes()); // sequence
    tx.push(1); // one output
    tx.extend_from_slice(&amount.to_le_bytes());
    push_data(&mut tx, script_pubkey);
    tx.extend_from_slice(&0u32.to_le_bytes()); // locktime
    tx
}

fn fake_der(r: u8, s: u8) -> Vec<u8> {
    // Valid *shape*, but not valid ECDSA signatures of the transaction.
    vec![0x30, 0x06, 0x02, 0x01, r, 0x02, 0x01, s, 0x01]
}

fn good_stack() -> Vec<Vec<u8>> {
    vec![vec![], fake_der(1, 1), fake_der(1, 2), script()]
}

/// Second input spends the funding output; first is a dummy unrelated input.
/// Includes actual SegWit marker, first-input witness count 0 and second
/// input witness stack to test strict vin selection.
fn spending(
    funding_txid_raw: [u8; 32],
    vout: u32,
    script_sig: &[u8],
    witness: &[Vec<u8>],
    use_segwit: bool,
) -> Vec<u8> {
    let mut tx = Vec::new();
    tx.extend_from_slice(&2u32.to_le_bytes());
    if use_segwit {
        tx.extend_from_slice(&[0, 1]);
    }
    tx.push(2); // inputs
    tx.extend_from_slice(&[0x22; 32]);
    tx.extend_from_slice(&7u32.to_le_bytes());
    push_data(&mut tx, &[]);
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.extend_from_slice(&funding_txid_raw);
    tx.extend_from_slice(&vout.to_le_bytes());
    push_data(&mut tx, script_sig);
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.push(1); // output
    tx.extend_from_slice(&8_000u64.to_le_bytes());
    push_data(&mut tx, &[0x51]); // OP_TRUE fake output
    if use_segwit {
        tx.push(0); // first input empty witness
        tx.push(witness.len() as u8);
        for item in witness {
            push_data(&mut tx, item);
        }
    }
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx
}

fn pair() -> (Vec<u8>, Vec<u8>) {
    let fund = funding(&script_pubkey(), 10_000);
    let txid = inspect_transaction(&fund).unwrap().txid;
    let spend = spending(txid, 0, &[], &good_stack(), true);
    (fund, spend)
}

fn checked(
    funding_bytes: &[u8],
    spend_bytes: &[u8],
    signers: ClaimedTwoOfTwoPubkeys,
) -> Result<
    tradelayer_core::channel_witness_audit::UnverifiedP2wshOutpointSpend,
    StructuralChannelError,
> {
    audit_native_p2wsh_outpoint_unverified(
        TradeLayerMainnet::Litecoin,
        funding_bytes,
        0,
        spend_bytes,
        1,
        signers,
    )
}

#[test]
fn independent_stdlib_sha256_p2wsh_program_and_txid_outpoint_match() {
    let (fund, spend) = pair();
    let report = checked(&fund, &spend, keys()).unwrap();
    assert_eq!(report.network(), TradeLayerMainnet::Litecoin);
    assert_eq!(report.funding_vout(), 0);
    assert_eq!(report.spending_vin(), 1);
    assert_eq!(report.funding_satoshis(), 10_000);
    assert_eq!(report.script_sha256(), PROGRAM);
    let mut funding_display = inspect_transaction(&fund).unwrap().txid;
    funding_display.reverse();
    assert_eq!(report.funding_txid_display(), funding_display);
    let mut spending_display = inspect_transaction(&spend).unwrap().txid;
    spending_display.reverse();
    assert_eq!(report.spending_txid_display(), spending_display);
    // Deliberately fake 9-byte DER signatures pass this STRUCTURAL audit.
    // It is *never* a proof of signer consent or ECDSA validity.
    assert_eq!(
        report.require_verified_channel_signatures(),
        Err(StructuralChannelError::SignaturesNotVerified)
    );
}

#[test]
fn witness_script_compressed_pubkeys_match_in_either_source_order() {
    let (fund, spend) = pair();
    let mut reordered = keys();
    std::mem::swap(&mut reordered.participant_a, &mut reordered.participant_b);
    let status = checked(&fund, &spend, reordered).unwrap();
    assert_eq!(
        status.require_verified_channel_signatures(),
        Err(StructuralChannelError::SignaturesNotVerified)
    );
}

#[test]
fn rejects_wrong_prevout_even_if_script_and_claimed_signers_are_correct() {
    let (fund, _) = pair();
    let mut txid = inspect_transaction(&fund).unwrap().txid;
    txid[0] ^= 1;
    let mut spend = spending(txid, 0, &[], &good_stack(), true);
    assert_eq!(
        checked(&fund, &spend, keys()),
        Err(StructuralChannelError::WrongPrevout)
    );
    let txid = inspect_transaction(&fund).unwrap().txid;
    spend = spending(txid, 1, &[], &good_stack(), true);
    assert_eq!(
        checked(&fund, &spend, keys()),
        Err(StructuralChannelError::WrongPrevout)
    );
}

#[test]
fn rejects_p2sh_unsupported_output_wrong_witness_program_and_empty_value() {
    let (_, spend) = pair();
    assert_eq!(
        checked(
            &funding(&[0xa9, 0x14, 2, 3, 4, 0x87], 10_000),
            &spend,
            keys()
        ),
        Err(StructuralChannelError::UnsupportedFundingScript)
    );
    // Same raw funding txid changes when a program changes, so spend must
    // point to the UPDATED funding ID before testing the SHA mismatch.
    let mut wrong_spk = script_pubkey();
    wrong_spk[2] ^= 1;
    let bad_fund = funding(&wrong_spk, 10_000);
    let witness = spending(
        inspect_transaction(&bad_fund).unwrap().txid,
        0,
        &[],
        &good_stack(),
        true,
    );
    assert_eq!(
        checked(&bad_fund, &witness, keys()),
        Err(StructuralChannelError::WitnessCommitmentMismatch)
    );
    let zero_fund = funding(&script_pubkey(), 0);
    let zero_spend = spending(
        inspect_transaction(&zero_fund).unwrap().txid,
        0,
        &[],
        &good_stack(),
        true,
    );
    assert_eq!(
        checked(&zero_fund, &zero_spend, keys()),
        Err(StructuralChannelError::InvalidOutputValue)
    );
}

#[test]
fn distinct_signers_and_exact_2of2_checkmultisig_redeem_script_required() {
    let (fund, spend) = pair();
    let mut claimed = keys();
    claimed.participant_b = claimed.participant_a;
    assert_eq!(
        checked(&fund, &spend, claimed),
        Err(StructuralChannelError::SameSignerPubkey)
    );
    claimed = keys();
    claimed.participant_b[0] = 4;
    assert_eq!(
        checked(&fund, &spend, claimed),
        Err(StructuralChannelError::InvalidCompressedPubkeys)
    );
    let mut other_pubkey = keys();
    other_pubkey.participant_b[1] ^= 1;
    assert_eq!(
        checked(&fund, &spend, other_pubkey),
        Err(StructuralChannelError::MismatchedWitnessPubkeys)
    );
    let mut stack = good_stack();
    stack[3][0] = 0x51; // not OP_2
    let mutated = spending(
        inspect_transaction(&fund).unwrap().txid,
        0,
        &[],
        &stack,
        true,
    );
    assert_eq!(
        checked(&fund, &mutated, keys()),
        Err(StructuralChannelError::InvalidWitnessScript)
    );
}

#[test]
fn a_claimed_2of2_must_have_empty_dummy_and_exactly_two_sig_slots() {
    let (fund, _) = pair();
    let id = inspect_transaction(&fund).unwrap().txid;
    let mut stack = good_stack();
    stack[0] = vec![0];
    let spend = spending(id, 0, &[], &stack, true);
    assert_eq!(
        checked(&fund, &spend, keys()),
        Err(StructuralChannelError::InvalidWitnessShape)
    );
    stack = good_stack();
    stack.remove(2);
    let spend = spending(id, 0, &[], &stack, true);
    assert_eq!(
        checked(&fund, &spend, keys()),
        Err(StructuralChannelError::InvalidWitnessShape)
    );
    stack = good_stack();
    stack[2] = stack[1].clone();
    let spend = spending(id, 0, &[], &stack, true);
    assert_eq!(
        checked(&fund, &spend, keys()),
        Err(StructuralChannelError::InvalidWitnessShape)
    );
}

#[test]
fn der_encoding_is_structural_not_a_real_ecdsa_validity_check() {
    let (fund, _) = pair();
    let id = inspect_transaction(&fund).unwrap().txid;
    for fake in [
        vec![],
        vec![1, 2, 3],
        vec![0x30, 0x08, 2, 1, 1, 2, 1, 2, 1], // wrong DER length
        vec![0x30, 0x06, 2, 1, 0x80, 2, 1, 2, 1], // negative R
        vec![0x30, 0x07, 2, 2, 0, 1, 2, 1, 2, 1], // nonminimal R
        vec![0x30, 0x06, 2, 1, 1, 2, 1, 0x80, 1], // negative S
    ] {
        let mut stack = good_stack();
        stack[1] = fake;
        let spend = spending(id, 0, &[], &stack, true);
        assert_eq!(
            checked(&fund, &spend, keys()),
            Err(StructuralChannelError::MalformedSignatureEncoding)
        );
    }
    let mut stack = good_stack();
    *stack[1].last_mut().unwrap() = 0x02; // SIGHASH_NONE unratified
    let spend = spending(id, 0, &[], &stack, true);
    assert_eq!(
        checked(&fund, &spend, keys()),
        Err(StructuralChannelError::UnsupportedSignatureHashType)
    );
}

#[test]
fn nested_or_legacy_spend_and_dogecoin_are_explicitly_unsupported() {
    let (fund, _) = pair();
    let id = inspect_transaction(&fund).unwrap().txid;
    let nested = spending(id, 0, &[0x16, 0, 0x14], &good_stack(), true);
    assert_eq!(
        checked(&fund, &nested, keys()),
        Err(StructuralChannelError::NonemptySpendingScriptSig)
    );
    let legacy = spending(id, 0, &[], &good_stack(), false);
    assert_eq!(
        checked(&fund, &legacy, keys()),
        Err(StructuralChannelError::NonNativeWitness)
    );
    let canonical = spending(id, 0, &[], &good_stack(), true);
    assert_eq!(
        audit_native_p2wsh_outpoint_unverified(
            TradeLayerMainnet::Dogecoin,
            &fund,
            0,
            &canonical,
            1,
            keys()
        ),
        Err(StructuralChannelError::UnsupportedNetwork)
    );
}

#[test]
fn invalid_indices_and_canonical_compactsize_bounds_fail_closed() {
    let (fund, spend) = pair();
    assert_eq!(
        audit_native_p2wsh_outpoint_unverified(
            TradeLayerMainnet::Litecoin,
            &fund,
            1,
            &spend,
            1,
            keys()
        ),
        Err(StructuralChannelError::InvalidOutputIndex)
    );
    assert_eq!(
        audit_native_p2wsh_outpoint_unverified(
            TradeLayerMainnet::Litecoin,
            &fund,
            0,
            &spend,
            2,
            keys()
        ),
        Err(StructuralChannelError::InvalidInputIndex)
    );
    let mut nonminimal = fund.clone();
    nonminimal.splice(4..5, [0xfd, 0x01, 0x00]); // input count 1 non-minimal
    assert!(matches!(
        checked(&nonminimal, &spend, keys()),
        Err(StructuralChannelError::RawTransaction(
            ScanError::NonCanonicalCompactSize
        ))
    ));
    let mut truncated = spend.clone();
    truncated.pop();
    assert!(matches!(
        checked(&fund, &truncated, keys()),
        Err(StructuralChannelError::RawTransaction(ScanError::Truncated))
    ));
    let mut trailing = spend.clone();
    trailing.push(0);
    assert_eq!(
        checked(&fund, &trailing, keys()),
        Err(StructuralChannelError::RawTransaction(
            ScanError::TrailingBytes
        ))
    );
}

#[test]
fn structural_observation_does_not_spend_or_admit_liquidation() {
    let (fund, spend) = pair();
    let report = checked(&fund, &spend, keys()).unwrap();
    assert_eq!(report.funding_vout(), 0);
    // Deliberately minimal DER fake signatures are NOT mathematically valid
    // for the transaction. This read-only module does not verify ECDSA.
    for _ in 0..3 {
        assert_eq!(
            report.require_verified_channel_signatures(),
            Err(StructuralChannelError::SignaturesNotVerified)
        );
    }
    // There is no conversion to a Ledger Operation, ClearingSandbox, or
    // type-4/type-19 admission authorization. No mutation API is exposed.
}


fn bip143_checked(
    funding_bytes: &[u8],
    spend_bytes: &[u8],
) -> Result<tradelayer_core::channel_bip143_audit::UnverifiedP2wshBip143Digest, StructuralChannelError> {
    audit_native_p2wsh_bip143_sighash_all_unverified(
        TradeLayerMainnet::Litecoin,
        funding_bytes,
        0,
        spend_bytes,
        1,
        keys(),
    )
}

fn expected_32(hex: &str) -> [u8; 32] {
    decode_hex(hex).unwrap().try_into().unwrap()
}

#[test]
fn independently_calculated_python_bip143_v0_sighash_all_vector() {
    // Python hashlib.sha256 (twice), independent from Rust's custom SHA.
    // scripts/audit_bip143_channel_fixture.py freezes this second-language
    // computation of serialized funding, selected SECOND input, 71-byte
    // scriptCode, base-chain funding amount, sequences and all outputs.
    let (fund, spend) = pair();
    let checked = bip143_checked(&fund, &spend).unwrap();
    assert_eq!(
        checked.digest_raw(),
        expected_32("5dc9f28f470cf591f0e7aacd431bbbdd31c8011ba96f3eb13c17e1ec8d408210")
    );
    assert_eq!(
        checked.hash_prevouts_raw(),
        expected_32("ea816a41f5f26c3bf6798e1120432065e3ff7679880d45d1bc24820e23d3d308")
    );
    assert_eq!(
        checked.hash_sequence_raw(),
        expected_32("752adad0a7b9ceca853768aebb6965eca126a62965f698a0c1bc43d83db632ad")
    );
    assert_eq!(
        checked.hash_outputs_raw(),
        expected_32("7ce42f6d181802782fc3ea5329d76e8fe344cbc1aa49742806d5a71528d6cd49")
    );
    assert_eq!(checked.structural().spending_vin(), 1);
    assert_eq!(checked.structural().funding_satoshis(), 10_000);
    assert_eq!(
        checked.require_verified_channel_signatures(),
        Err(StructuralChannelError::SignaturesNotVerified)
    );
}

#[test]
fn sighash_all_changes_on_funding_amount_outputs_version_sequence_and_locktime() {
    let (fund, spend) = pair();
    let digest = bip143_checked(&fund, &spend).unwrap().digest_raw();
    let new_fund = funding(&script_pubkey(), 10_001);
    let new_spend = spending(
        inspect_transaction(&new_fund).unwrap().txid,
        0,
        &[],
        &good_stack(),
        true,
    );
    assert_ne!(bip143_checked(&new_fund, &new_spend).unwrap().digest_raw(), digest);

    let mut modified = spend.clone();
    modified[0] ^= 1; // nVersion, not SegWit marker
    assert_ne!(bip143_checked(&fund, &modified).unwrap().digest_raw(), digest);

    let mut modified = spend.clone();
    // First input's 4-byte sequence follows its outpoint and empty script.
    let first_seq = 4 + 2 + 1 + 32 + 4 + 1;
    modified[first_seq] ^= 1;
    let sequence_change = bip143_checked(&fund, &modified).unwrap();
    assert_ne!(sequence_change.digest_raw(), digest);
    assert_ne!(
        sequence_change.hash_sequence_raw(),
        bip143_checked(&fund, &spend).unwrap().hash_sequence_raw()
    );

    let mut modified = spend.clone();
    // All outputs, including amounts and scripts, must bind in SIGHASH_ALL.
    let idx = modified
        .windows(8)
        .position(|window| window == 8_000u64.to_le_bytes())
        .unwrap();
    modified[idx] ^= 1; // output amount, not funding amount
    let output_change = bip143_checked(&fund, &modified).unwrap();
    assert_ne!(output_change.digest_raw(), digest);
    assert_ne!(
        output_change.hash_outputs_raw(),
        bip143_checked(&fund, &spend).unwrap().hash_outputs_raw()
    );

    let mut modified = spend.clone();
    let locktime_start = modified.len() - 4;
    modified[locktime_start] = 1;
    assert_ne!(bip143_checked(&fund, &modified).unwrap().digest_raw(), digest);
}

#[test]
fn signature_bytes_are_excluded_from_bip143_preimage_and_not_verified() {
    let (fund, original) = pair();
    let original_tx = inspect_transaction(&original).unwrap();
    let original_digest = bip143_checked(&fund, &original).unwrap();
    let mut stack = good_stack();
    stack[1] = fake_der(3, 4); // fake DER from a different non-keypair
    stack[2] = fake_der(4, 5);
    let altered = spending(
        inspect_transaction(&fund).unwrap().txid,
        0,
        &[],
        &stack,
        true,
    );
    let changed = bip143_checked(&fund, &altered).unwrap();
    assert_eq!(changed.digest_raw(), original_digest.digest_raw());
    assert_eq!(changed.structural().spending_txid_display(), original_digest.structural().spending_txid_display());
    assert_ne!(inspect_transaction(&altered).unwrap().wtxid, original_tx.wtxid);
    assert_eq!(
        changed.require_verified_channel_signatures(),
        Err(StructuralChannelError::SignaturesNotVerified)
    );
}

#[test]
fn wrong_prevout_script_spend_vin_and_non_all_sighash_fail_before_digest() {
    let (fund, spend) = pair();
    assert_eq!(
        audit_native_p2wsh_bip143_sighash_all_unverified(
            TradeLayerMainnet::Dogecoin, &fund, 0, &spend, 1, keys()
        ),
        Err(StructuralChannelError::UnsupportedNetwork)
    );
    assert_eq!(
        audit_native_p2wsh_bip143_sighash_all_unverified(
            TradeLayerMainnet::Litecoin, &fund, 0, &spend, 2, keys()
        ),
        Err(StructuralChannelError::InvalidInputIndex)
    );
    let mut altered_fund = fund.clone();
    altered_fund[20] ^= 1;
    assert_eq!(
        bip143_checked(&altered_fund, &spend),
        Err(StructuralChannelError::WrongPrevout)
    );
    let mut stack = good_stack();
    *stack[1].last_mut().unwrap() = 2;
    let unsupported = spending(
        inspect_transaction(&fund).unwrap().txid, 0, &[], &stack, true
    );
    assert_eq!(
        bip143_checked(&fund, &unsupported),
        Err(StructuralChannelError::UnsupportedSignatureHashType)
    );
    let mut altered = spend.clone();
    altered.push(0);
    assert_eq!(
        bip143_checked(&fund, &altered),
        Err(StructuralChannelError::RawTransaction(ScanError::TrailingBytes))
    );
}

#[test]
fn bip143_digest_alone_cannot_authorize_channel_or_liquidation() {
    let (fund, spend) = pair();
    let observation = bip143_checked(&fund, &spend).unwrap();
    let digest = observation.digest_raw();
    assert_ne!(digest, [0u8; 32]);
    // Even after source-correct BIP143 arithmetic, neither ECDSA signer
    // is checked. Stage25G's always-deny authorization remains separate.
    assert_eq!(
        observation.require_verified_channel_signatures(),
        Err(StructuralChannelError::SignaturesNotVerified)
    );
}
