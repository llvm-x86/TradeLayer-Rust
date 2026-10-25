//! Stage25J: public-only cryptographically signed fixture, Rust digest parity.
//! OpenSSL verifies ECDSA in the *separate* Python harness. This Rust
//! module NEVER treats an off-chain signature as accepted-chain consensus.
use std::collections::BTreeMap;
use tradelayer_core::channel_bip143_audit::{
    audit_native_p2wsh_bip143_sighash_all_unverified, UnverifiedP2wshBip143Digest,
};
use tradelayer_core::channel_witness_audit::{
    ClaimedTwoOfTwoPubkeys, StructuralChannelError,
};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

const FIXTURE: &str = include_str!("../fixtures/channel_p2wsh_ecdsa_signed_synthetic.tsv");

fn fields() -> BTreeMap<&'static str, &'static str> {
    let mut result = BTreeMap::new();
    for line in FIXTURE.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('\t').expect("fixture key/value");
        assert!(result.insert(key, value).is_none(), "duplicate key");
    }
    result
}

fn exact32(input: &str) -> [u8; 32] {
    decode_hex(input).unwrap().try_into().unwrap()
}

fn witness_fixture() -> (Vec<u8>, Vec<u8>, ClaimedTwoOfTwoPubkeys) {
    let f = fields();
    assert_eq!(f["network"], "Litecoin");
    assert_eq!(f["funding_vout"], "0");
    assert_eq!(f["spending_vin"], "1");
    let a: [u8; 33] = decode_hex(f["pubkey_a"]).unwrap().try_into().unwrap();
    let b: [u8; 33] = decode_hex(f["pubkey_b"]).unwrap().try_into().unwrap();
    (
        decode_hex(f["funding_hex"]).unwrap(),
        decode_hex(f["spending_hex"]).unwrap(),
        ClaimedTwoOfTwoPubkeys {
            participant_a: a,
            participant_b: b,
        },
    )
}

fn checked(funding: &[u8], spending: &[u8], keys: ClaimedTwoOfTwoPubkeys)
    -> Result<UnverifiedP2wshBip143Digest, StructuralChannelError>
{
    audit_native_p2wsh_bip143_sighash_all_unverified(
        TradeLayerMainnet::Litecoin,
        funding,
        0,
        spending,
        1,
        keys,
    )
}

#[test]
fn genuine_synthetic_ecdsa_fixture_agrees_with_rust_bip143_but_remains_unverified() {
    let f = fields();
    let (funding, spending, keys) = witness_fixture();
    let report = checked(&funding, &spending, keys).unwrap();
    assert_eq!(report.digest_raw(), exact32(f["expected_bip143_digest"]));
    assert_eq!(report.hash_prevouts_raw(), exact32(f["expected_prevouts"]));
    assert_eq!(report.hash_sequence_raw(), exact32(f["expected_sequence"]));
    assert_eq!(report.hash_outputs_raw(), exact32(f["expected_outputs"]));
    assert_eq!(inspect_transaction(&funding).unwrap().txid, exact32(f["funding_txid_raw"]));
    assert_eq!(inspect_transaction(&spending).unwrap().txid, exact32(f["spending_txid_raw"]));
    assert_eq!(inspect_transaction(&spending).unwrap().wtxid, exact32(f["spending_wtxid_raw"]));
    assert_eq!(report.structural().funding_satoshis(), 20_000);
    assert_eq!(
        report.require_verified_channel_signatures(),
        Err(StructuralChannelError::SignaturesNotVerified)
    );
}

#[test]
fn changed_funding_amount_rejects_old_spend_and_forged_signer_claim() {
    let (mut funding, spending, keys) = witness_fixture();
    // Raw funding transaction output value starts after 4 version,
    // 1 input count, 32 prevout TXID, 4 vout, 1 empty scriptSig,
    // 4 sequence, 1 output count.
    let value_offset = 4 + 1 + 32 + 4 + 1 + 4 + 1;
    funding[value_offset] ^= 1;
    assert_eq!(
        checked(&funding, &spending, keys),
        Err(StructuralChannelError::WrongPrevout)
    );
    let (funding, spending, mut keys) = witness_fixture();
    keys.participant_a[1] ^= 1;
    assert_eq!(
        checked(&funding, &spending, keys),
        Err(StructuralChannelError::MismatchedWitnessPubkeys)
    );
}

#[test]
fn witness_signature_bytes_are_not_digested_and_never_establish_consent() {
    let (funding, spending, keys) = witness_fixture();
    let original = checked(&funding, &spending, keys).unwrap();
    // Change one R-byte in the first DER signature. The witness shape
    // remains syntactically valid; an independent OpenSSL test will
    // mathematically reject it even though BIP143 digest is unchanged.
    let needle = [0x30u8, 0x44, 0x02, 0x20, 0x21, 0xd7, 0xc0, 0x87];
    let start = spending
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("first DER witness in serialized fixture");
    let mut forged = spending.clone();
    forged[start + 5] ^= 1;
    let changed = checked(&funding, &forged, keys).unwrap();
    assert_eq!(changed.digest_raw(), original.digest_raw());
    assert_eq!(
        changed.structural().spending_txid_display(),
        original.structural().spending_txid_display()
    );
    assert_ne!(
        inspect_transaction(&forged).unwrap().wtxid,
        inspect_transaction(&spending).unwrap().wtxid
    );
    assert_eq!(
        changed.require_verified_channel_signatures(),
        Err(StructuralChannelError::SignaturesNotVerified)
    );
}
