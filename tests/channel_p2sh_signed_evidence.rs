//! Stage25L: real signed synthetic P2SH digest differential tests.
//! OpenSSL is an independent, test-only ECDSA oracle in Python CI.
use std::collections::BTreeMap;
use tradelayer_core::channel_p2sh_audit::P2shAuditError;
use tradelayer_core::channel_p2sh_sighash_audit::{
    audit_legacy_p2sh_sighash_all_unverified, UnverifiedP2shLegacySighashDigest,
};
use tradelayer_core::channel_witness_audit::ClaimedTwoOfTwoPubkeys;
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn fields() -> BTreeMap<&'static str, &'static str> {
    let mut values = BTreeMap::new();
    for line in include_str!("../fixtures/channel_p2sh_ecdsa_signed_synthetic.tsv").lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('\t').expect("tab-separated fixture");
        assert!(values.insert(key, value).is_none(), "duplicate fixture key");
    }
    values
}

fn exact32(hex: &str) -> [u8; 32] {
    decode_hex(hex).unwrap().try_into().unwrap()
}

fn pair() -> (Vec<u8>, Vec<u8>, ClaimedTwoOfTwoPubkeys) {
    let f = fields();
    assert_eq!(f["network"], "Litecoin");
    (
        decode_hex(f["funding_hex"]).unwrap(),
        decode_hex(f["spending_hex"]).unwrap(),
        ClaimedTwoOfTwoPubkeys {
            participant_a: decode_hex(f["pubkey_a"]).unwrap().try_into().unwrap(),
            participant_b: decode_hex(f["pubkey_b"]).unwrap().try_into().unwrap(),
        },
    )
}

fn checked(
    funding: &[u8],
    spending: &[u8],
    keys: ClaimedTwoOfTwoPubkeys,
) -> Result<UnverifiedP2shLegacySighashDigest, P2shAuditError> {
    audit_legacy_p2sh_sighash_all_unverified(
        TradeLayerMainnet::Litecoin,
        funding,
        0,
        spending,
        1,
        keys,
    )
}

#[test]
fn signed_fixture_digest_matches_independent_python_and_denies_authority() {
    let f = fields();
    let (funding, spending, keys) = pair();
    let result = checked(&funding, &spending, keys).unwrap();
    assert_eq!(result.digest_raw(), exact32(f["expected_legacy_digest"]));
    assert_eq!(
        inspect_transaction(&funding).unwrap().txid,
        exact32(f["funding_txid_raw"])
    );
    assert_eq!(
        inspect_transaction(&spending).unwrap().txid,
        exact32(f["spending_txid_raw"])
    );
    assert_eq!(result.structural().funding_satoshis(), 20_000);
    assert_eq!(
        result.require_verified_channel_signatures(),
        Err(P2shAuditError::SignaturesNotVerified)
    );
    let bitcoin = audit_legacy_p2sh_sighash_all_unverified(
        TradeLayerMainnet::Bitcoin,
        &funding,
        0,
        &spending,
        1,
        keys,
    )
    .unwrap();
    assert_eq!(bitcoin.digest_raw(), result.digest_raw());
    assert_eq!(
        bitcoin.require_verified_channel_signatures(),
        Err(P2shAuditError::SignaturesNotVerified)
    );
}

#[test]
fn forged_legacy_script_signature_changes_txid_but_not_message_digest() {
    let f = fields();
    let (funding, mut spending, keys) = pair();
    let first_sig = decode_hex(f["signature_a"]).unwrap();
    let offset = spending
        .windows(first_sig.len())
        .position(|window| window == first_sig)
        .expect("first genuine legacy signature");
    let original = checked(&funding, &spending, keys).unwrap();
    let original_txid = inspect_transaction(&spending).unwrap().txid;
    spending[offset + 9] ^= 1;
    let forged = checked(&funding, &spending, keys).unwrap();
    assert_eq!(forged.digest_raw(), original.digest_raw());
    assert_ne!(inspect_transaction(&spending).unwrap().txid, original_txid);
    assert_eq!(
        forged.require_verified_channel_signatures(),
        Err(P2shAuditError::SignaturesNotVerified)
    );
}

#[test]
fn sighash_all_commits_output_other_input_sequence_version_and_locktime() {
    let (funding, spending, keys) = pair();
    let baseline = checked(&funding, &spending, keys).unwrap().digest_raw();
    let output = 18_000u64.to_le_bytes();
    let output_offset = spending
        .windows(output.len())
        .position(|window| window == output)
        .expect("payment output in signed transaction");
    for offset in [output_offset, 42, 4 + 1, 0, spending.len() - 1] {
        let mut tampered = spending.clone();
        tampered[offset] ^= 1;
        let changed = checked(&funding, &tampered, keys).unwrap();
        assert_ne!(changed.digest_raw(), baseline);
        assert_eq!(
            changed.require_verified_channel_signatures(),
            Err(P2shAuditError::SignaturesNotVerified)
        );
    }
}

#[test]
fn changed_funding_value_requires_matching_new_outpoint_and_digest() {
    let (mut funding, mut spending, keys) = pair();
    let baseline = checked(&funding, &spending, keys).unwrap().digest_raw();
    // Funding version + input count + 36-byte outpoint + empty script +
    // sequence + output count = 4 + 1 + 36 + 1 + 4 + 1.
    funding[47] ^= 1;
    assert_eq!(
        checked(&funding, &spending, keys),
        Err(P2shAuditError::WrongPrevout)
    );
    let updated_txid = inspect_transaction(&funding).unwrap().txid;
    // Spending input 0 consumes 36 + 1 + 4 bytes after version/vin count.
    spending[46..78].copy_from_slice(&updated_txid);
    let changed = checked(&funding, &spending, keys).unwrap();
    assert_ne!(changed.digest_raw(), baseline);
}

#[test]
fn unsupported_network_and_misclaimed_pubkey_are_never_authorized() {
    let (funding, spending, keys) = pair();
    assert_eq!(
        audit_legacy_p2sh_sighash_all_unverified(
            TradeLayerMainnet::Dogecoin,
            &funding,
            0,
            &spending,
            1,
            keys
        ),
        Err(P2shAuditError::UnsupportedNetwork)
    );
    let mut wrong = keys;
    wrong.participant_a[1] ^= 1;
    assert!(checked(&funding, &spending, wrong).is_err());
}
