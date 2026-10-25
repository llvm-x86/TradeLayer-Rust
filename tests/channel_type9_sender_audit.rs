//! Stage25O: synthetic genuine P2WPKH first-input origin + always-deny.
//! Header is FABRICATED at a caller-selected anchor; ECDSA math verified
//! independently by Python + system OpenSSL in the CI harness.
use tradelayer_core::bitcoin_inclusion::{
    BitcoinPowProfile, CheckedHeaderSegment, MerklePath,
};
use tradelayer_core::chain::{BlockId, Tip};
use tradelayer_core::channel_admission_audit::ClaimedDirectAction;
use tradelayer_core::channel_type9_sender_audit::{
    audit_bitcoin_type9_p2wpkh_first_input_unverified, P2wpkhSenderError,
};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn field(key: &str) -> String {
    include_str!("../fixtures/channel_type9_p2wpkh_signed_synthetic.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            let (name, value) = line.split_once('\t')?;
            (name == key).then_some(value.to_owned())
        })
        .unwrap_or_else(|| panic!("missing Stage25O fixture key {key}"))
}

fn raw(key: &str) -> Vec<u8> {
    decode_hex(&field(key)).unwrap()
}
fn fixed32(key: &str) -> [u8; 32] {
    raw(key).try_into().unwrap()
}
fn segment() -> CheckedHeaderSegment {
    let header: [u8; 80] = raw("header_hex").try_into().unwrap();
    CheckedHeaderSegment::check_against_claimed_anchor(
        Tip {
            height: 100,
            id: BlockId([0x55; 32]),
        },
        &[header],
        BitcoinPowProfile::RegtestLike,
    )
    .unwrap()
}
fn parent_path() -> MerklePath {
    MerklePath {
        tx_index: 0,
        tx_count: 2,
        siblings_raw: vec![fixed32("funding_merkle_sibling_raw")],
    }
}
fn spend_path() -> MerklePath {
    MerklePath {
        tx_index: 1,
        tx_count: 2,
        siblings_raw: vec![fixed32("spending_merkle_sibling_raw")],
    }
}
fn audit(
    funding: &[u8],
    spending: &[u8],
    sender: &str,
) -> Result<
    tradelayer_core::channel_type9_sender_audit::UnverifiedP2wpkhType9FirstInput,
    P2wpkhSenderError,
> {
    audit_bitcoin_type9_p2wpkh_first_input_unverified(
        TradeLayerMainnet::Bitcoin,
        &segment(),
        funding,
        101,
        0,
        &parent_path(),
        spending,
        101,
        &spend_path(),
        sender,
    )
}

#[test]
fn genuine_synthetic_parent_and_type9_origin_match_hash160_address_and_bip143_digest() {
    let funding = raw("funding_hex");
    let spending = raw("spending_hex");
    let sender = field("sender_bech32");
    let report = audit(&funding, &spending, &sender).unwrap();
    assert_eq!(report.funding().height(), 101);
    assert_eq!(report.funding().tx_index(), 0);
    assert_eq!(report.type9_spending().tx_index(), 1);
    assert_eq!(report.funding_satoshis(), 20_000);
    assert_eq!(report.address_hash160().to_vec(), raw("hash160"));
    assert_eq!(report.public_key().to_vec(), raw("pubkey"));
    assert_eq!(report.claimed_sender_address(), sender);
    assert_eq!(report.digest_raw(), fixed32("expected_digest"));
    assert_eq!(inspect_transaction(&funding).unwrap().txid, fixed32("funding_txid_raw"));
    assert_eq!(
        inspect_transaction(&spending).unwrap().txid,
        fixed32("spending_txid_raw")
    );
    assert_eq!(
        inspect_transaction(&spending).unwrap().wtxid,
        fixed32("spending_wtxid_raw")
    );
    let type9 = report.type9_payload();
    assert_eq!(type9.action(), ClaimedDirectAction::Attest);
    assert_eq!(type9.clearlist_id(), 7);
    assert_eq!(type9.target(), "alice");
    assert_eq!(type9.metadata(), "OK");
    assert_eq!(
        type9.require_authorized_attestation(),
        Err(tradelayer_core::channel_type9_inclusion_audit::Type9OriginError::OriginIsNotAuthorization)
    );
    assert_eq!(
        report.require_authenticated_sender(),
        Err(P2wpkhSenderError::SenderSignatureAndScriptNotVerified)
    );
    assert_eq!(
        report.require_historical_admin_permission(),
        Err(P2wpkhSenderError::HistoricalListAdministratorNotVerified)
    );
    assert_eq!(
        report.require_canonical_channel_revision(),
        Err(P2wpkhSenderError::CanonicalStateRevisionUnspecified)
    );
}

#[test]
fn another_valid_bitcoin_bech32_address_cannot_impersonate_this_sender() {
    let funding = raw("funding_hex");
    let spending = raw("spending_hex");
    // Published BIP173 address for compressed secp256k1 generator public key.
    let other = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
    assert_eq!(
        audit(&funding, &spending, other),
        Err(P2wpkhSenderError::SenderAddressMismatch)
    );
    let sender = field("sender_bech32");
    let forged_checksum = format!("{}x", &sender[..sender.len() - 1]);
    assert_eq!(
        audit(&funding, &spending, &forged_checksum),
        Err(P2wpkhSenderError::InvalidBech32Address)
    );
    assert_eq!(
        audit(&funding, &spending, &sender.to_ascii_uppercase()),
        Err(P2wpkhSenderError::InvalidBech32Address)
    );
    assert_eq!(
        audit(&funding, &spending, &sender.replacen("bc1", "ltc1", 1)),
        Err(P2wpkhSenderError::InvalidBech32Address)
    );
}

#[test]
fn forged_signature_witness_preserves_txid_merkle_digest_but_never_grants_authority() {
    let funding = raw("funding_hex");
    let mut spending = raw("spending_hex");
    let original = audit(&funding, &spending, &field("sender_bech32")).unwrap();
    let original_txid = inspect_transaction(&spending).unwrap().txid;
    let original_wtxid = inspect_transaction(&spending).unwrap().wtxid;
    let signature = raw("signature_der_sighash");
    let start = spending
        .windows(signature.len())
        .position(|bytes| bytes == signature)
        .expect("signed P2WPKH witness in synthetic raw tx");
    spending[start + 10] ^= 1; // DER R data, preserve the BIP66 shape.
    let forged = audit(&funding, &spending, &field("sender_bech32")).unwrap();
    assert_eq!(forged.digest_raw(), original.digest_raw());
    assert_eq!(inspect_transaction(&spending).unwrap().txid, original_txid);
    assert_ne!(inspect_transaction(&spending).unwrap().wtxid, original_wtxid);
    assert_eq!(
        forged.require_authenticated_sender(),
        Err(P2wpkhSenderError::SenderSignatureAndScriptNotVerified)
    );
}

#[test]
fn wrong_merkle_funding_txid_fork_and_spending_order_reject() {
    let funding = raw("funding_hex");
    let spending = raw("spending_hex");
    let sender = field("sender_bech32");
    let mut bad_parent = funding.clone();
    let amount_offset = 4 + 1 + 36 + 1 + 4 + 1;
    bad_parent[amount_offset] ^= 1;
    assert!(matches!(
        audit(&bad_parent, &spending, &sender),
        Err(P2wpkhSenderError::HeaderOrMerkle(_))
    ));
    let mut bad_spend = spending.clone();
    bad_spend[45] ^= 1; // serialized first-input outpoint
    assert!(matches!(
        audit(&funding, &bad_spend, &sender),
        Err(P2wpkhSenderError::HeaderOrMerkle(_))
    ));
    let mut bad_path = spend_path();
    bad_path.siblings_raw[0][0] ^= 1;
    assert!(matches!(
        audit_bitcoin_type9_p2wpkh_first_input_unverified(
            TradeLayerMainnet::Bitcoin, &segment(), &funding, 101, 0, &parent_path(),
            &spending, 101, &bad_path, &sender
        ),
        Err(P2wpkhSenderError::HeaderOrMerkle(_))
    ));
    assert_eq!(
        audit_bitcoin_type9_p2wpkh_first_input_unverified(
            TradeLayerMainnet::Bitcoin, &segment(), &funding, 101, 0, &spend_path(),
            &spending, 101, &parent_path(), &sender
        ),
        Err(P2wpkhSenderError::NoncausalTransactionOrder)
    );
}

#[test]
fn unsupported_networks_and_wrong_raw_marker_cannot_become_a_sender() {
    let funding = raw("funding_hex");
    let spending = raw("spending_hex");
    let sender = field("sender_bech32");
    for network in [TradeLayerMainnet::Litecoin, TradeLayerMainnet::Dogecoin] {
        assert_eq!(
            audit_bitcoin_type9_p2wpkh_first_input_unverified(
                network, &segment(), &funding, 101, 0, &parent_path(),
                &spending, 101, &spend_path(), &sender
            ),
            Err(P2wpkhSenderError::UnsupportedNetwork)
        );
    }
    let mut changed = spending.clone();
    let marker = b"tl90,7,alice,OK";
    let start = changed
        .windows(marker.len())
        .position(|bytes| bytes == marker)
        .expect("type9 payload in signed fixture");
    changed[start + 2] = b'8';
    assert!(matches!(
        audit(&funding, &changed, &sender),
        Err(P2wpkhSenderError::HeaderOrMerkle(_))
    ));
}
