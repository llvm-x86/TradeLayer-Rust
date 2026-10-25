//! Stage25M: BIP141 witness commitment for a synthetic Bitcoin-like block.
//! Both header anchor and source transactions remain UNVERIFIED. A valid
//! witness commitment does not validate UTXOs, scripts or ECDSA signatures.

use tradelayer_core::bitcoin_inclusion::{BitcoinPowProfile, CheckedHeaderSegment, MerklePath};
use tradelayer_core::bitcoin_witness_commitment::{
    audit_bitcoin_bip141_witness_block_unverified, WitnessBlockError,
    MAX_UNVERIFIED_BLOCK_TRANSACTIONS,
};
use tradelayer_core::chain::{BlockId, Tip};
use tradelayer_core::channel_inclusion_audit::{
    audit_bitcoin_p2wsh_relative_inclusion_unverified, ChannelInclusionError,
};
use tradelayer_core::channel_witness_audit::ClaimedTwoOfTwoPubkeys;
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::read_only::{decode_hex, inspect_transaction, ScanError};

const FUNDING_HEADER_101: &str = concat!(
    "02000000",
    "3434343434343434343434343434343434343434343434343434343434343434",
    "123874d4a84664ee8e041e639c7ff01f999953bcce8335ca2a35abece40f2bc0",
    "00f15365ffff7f2000000000"
);

fn raw(value: &str) -> Vec<u8> {
    decode_hex(value).unwrap()
}
fn vector(key: &str) -> Vec<u8> {
    include_str!("../fixtures/stage25m_bip141_vectors.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .find_map(|line| {
            let (name, value) = line.split_once('\t')?;
            (name == key).then(|| raw(value))
        })
        .unwrap()
}
fn stage25j(key: &str) -> Vec<u8> {
    include_str!("../fixtures/channel_p2wsh_ecdsa_signed_synthetic.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .find_map(|line| {
            let (name, value) = line.split_once('\t')?;
            (name == key).then(|| raw(value))
        })
        .unwrap()
}
fn segment(header_hex: &[u8]) -> CheckedHeaderSegment {
    let funding: [u8; 80] = raw(FUNDING_HEADER_101).try_into().unwrap();
    let spending: [u8; 80] = header_hex.try_into().unwrap();
    CheckedHeaderSegment::check_against_claimed_anchor(
        Tip {
            height: 100,
            id: BlockId([0x34; 32]),
        },
        &[funding, spending],
        BitcoinPowProfile::RegtestLike,
    )
    .unwrap()
}
fn check(
    block: &CheckedHeaderSegment,
    txs: &[&[u8]],
    selected_index: usize,
) -> Result<
    tradelayer_core::bitcoin_witness_commitment::UnverifiedBitcoinWitnessBlock,
    WitnessBlockError,
> {
    audit_bitcoin_bip141_witness_block_unverified(
        TradeLayerMainnet::Bitcoin,
        block,
        102,
        txs,
        selected_index,
    )
}
fn corrupted_signature(spending: &[u8]) -> Vec<u8> {
    let mut forged = spending.to_vec();
    let needle = [0x30u8, 0x44, 0x02, 0x20, 0x21, 0xd7, 0xc0, 0x87];
    let start = forged
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap();
    forged[start + 5] ^= 1;
    forged
}

#[test]
fn coinbase_zero_leaf_and_bip141_commitment_match_independent_python() {
    let cb = vector("baseline_coinbase");
    let spend = stage25j("spending_hex");
    let block = segment(&vector("baseline_header"));
    let proof = check(&block, &[&cb, &spend], 1).unwrap();
    assert_eq!(proof.transaction_count(), 2);
    assert_eq!(proof.claimed_anchor().height, 100);
    assert_eq!(proof.claimed_block().height, 102);
    assert_eq!(
        proof.selected_txid_raw(),
        inspect_transaction(&spend).unwrap().txid
    );
    assert_eq!(
        proof.selected_wtxid_raw(),
        inspect_transaction(&spend).unwrap().wtxid
    );
    assert_eq!(
        proof.witness_merkle_root_raw().as_slice(),
        raw("bc570e76dfb6258c5b98ea6533e58429484a5e0c7a1dc0977e902c0b65570483")
    );
    assert_eq!(
        proof.committed_witness_hash_raw().as_slice(),
        raw("befe334eec4cb2e7485485564bc2c3dfea31310c10be172b9dbf96c2dc3947e6")
    );
    assert_eq!(
        proof.require_authenticated_channel_spend(),
        Err(WitnessBlockError::ActiveChainAndChannelAuthorizationUnavailable)
    );
}

#[test]
fn forged_der_signature_preserves_txid_inclusion_but_breaks_witness_commitment() {
    let cb = vector("baseline_coinbase");
    let spend = stage25j("spending_hex");
    let forged = corrupted_signature(&spend);
    let block = segment(&vector("baseline_header"));
    let original = inspect_transaction(&spend).unwrap();
    let changed = inspect_transaction(&forged).unwrap();
    assert_eq!(original.txid, changed.txid);
    assert_ne!(original.wtxid, changed.wtxid);
    let funding = stage25j("funding_hex");
    let keys = ClaimedTwoOfTwoPubkeys {
        participant_a: stage25j("pubkey_a").try_into().unwrap(),
        participant_b: stage25j("pubkey_b").try_into().unwrap(),
    };
    let relative = audit_bitcoin_p2wsh_relative_inclusion_unverified(
        TradeLayerMainnet::Bitcoin,
        &block,
        &funding,
        101,
        0,
        &MerklePath {
            tx_index: 0,
            tx_count: 1,
            siblings_raw: vec![],
        },
        &forged,
        102,
        1,
        &MerklePath {
            tx_index: 1,
            tx_count: 2,
            siblings_raw: vec![inspect_transaction(&cb).unwrap().txid],
        },
        keys,
    )
    .unwrap();
    assert_eq!(
        relative.require_authenticated_channel_spend(),
        Err(ChannelInclusionError::ActiveChainWitnessAndSignaturesUnverified)
    );
    assert_eq!(
        check(&block, &[&cb, &forged], 1),
        Err(WitnessBlockError::WitnessCommitmentMismatch)
    );
    // Recomputing the fake commitment makes altered signature bytes
    // consistent, even though the ECDSA signature itself is invalid.
    let regenerated_cb = vector("tampered_coinbase");
    let regenerated_block = segment(&vector("tampered_header"));
    let report = check(&regenerated_block, &[&regenerated_cb, &forged], 1).unwrap();
    assert_eq!(
        report.require_authenticated_channel_spend(),
        Err(WitnessBlockError::ActiveChainAndChannelAuthorizationUnavailable)
    );
}

#[test]
fn highest_index_commitment_wins_and_trailing_script_data_is_permitted() {
    let spend = stage25j("spending_hex");
    let good = vector("higher_good_coinbase");
    assert!(check(&segment(&vector("higher_good_header")), &[&good, &spend], 1).is_ok());
    let bad = vector("higher_bad_coinbase");
    assert_eq!(
        check(&segment(&vector("higher_bad_header")), &[&bad, &spend], 1),
        Err(WitnessBlockError::WitnessCommitmentMismatch)
    );
    let trailing = vector("trailing_coinbase");
    assert!(check(
        &segment(&vector("trailing_header")),
        &[&trailing, &spend],
        1
    )
    .is_ok());
}

#[test]
fn odd_leaf_merkle_padding_uses_last_wtxid_not_an_actual_duplicate_transaction() {
    let cb = vector("odd_coinbase");
    let funding = stage25j("funding_hex");
    let spending = stage25j("spending_hex");
    let block = segment(&vector("odd_header"));
    assert_eq!(
        check(&block, &[&cb, &funding, &spending], 2)
            .unwrap()
            .transaction_count(),
        3
    );
    assert_eq!(
        check(&block, &[&cb, &spending], 1),
        Err(WitnessBlockError::HeaderMerkleRootMismatch)
    );
    assert_eq!(
        check(&block, &[&cb, &spending, &funding], 1),
        Err(WitnessBlockError::HeaderMerkleRootMismatch)
    );
    assert_eq!(
        check(&block, &[&cb, &funding, &spending], 1),
        Err(WitnessBlockError::SelectedInputHasNoWitness)
    );
    assert_eq!(
        check(&block, &[&cb, &spending, &spending], 2),
        Err(WitnessBlockError::DuplicateTransactionId)
    );
}

#[test]
fn invalid_coinbase_reserved_and_missing_commitment_are_rejected() {
    let spend = stage25j("spending_hex");
    let cb = vector("baseline_coinbase");
    let block = segment(&vector("baseline_header"));
    let mut altered = cb.clone();
    let reserved_first = altered.len() - 36;
    altered[reserved_first] ^= 1;
    assert_eq!(
        check(&block, &[&altered, &spend], 1),
        Err(WitnessBlockError::WitnessCommitmentMismatch)
    );
    let mut malformed = cb.clone();
    let len_offset = malformed.len() - 37;
    malformed[len_offset] = 31;
    malformed.remove(len_offset + 1);
    assert_eq!(
        check(&block, &[&malformed, &spend], 1),
        Err(WitnessBlockError::InvalidCoinbaseReservedValue)
    );
    let mut bad_script = cb.clone();
    let magic = [0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];
    let magic_offset = bad_script
        .windows(magic.len())
        .position(|w| w == magic)
        .unwrap();
    bad_script[magic_offset] = 0x6b;
    assert_eq!(
        check(&block, &[&bad_script, &spend], 1),
        Err(WitnessBlockError::MissingCoinbaseCommitment)
    );
    let mut bad_compact = cb.clone();
    let stack_count_offset = bad_compact.len() - 38;
    bad_compact.splice(
        stack_count_offset..stack_count_offset + 1,
        [0xfd, 0x01, 0x00],
    );
    assert_eq!(
        check(&block, &[&bad_compact, &spend], 1),
        Err(WitnessBlockError::RawTransaction(
            ScanError::NonCanonicalCompactSize
        ))
    );
}

#[test]
fn wrong_header_height_reparenting_and_replay_inputs_fail_closed() {
    let cb = vector("baseline_coinbase");
    let spend = stage25j("spending_hex");
    let block = segment(&vector("baseline_header"));
    assert_eq!(
        audit_bitcoin_bip141_witness_block_unverified(
            TradeLayerMainnet::Bitcoin,
            &block,
            103,
            &[&cb, &spend],
            1,
        ),
        Err(WitnessBlockError::InvalidBlockHeight)
    );
    assert_eq!(
        audit_bitcoin_bip141_witness_block_unverified(
            TradeLayerMainnet::Litecoin,
            &block,
            102,
            &[&cb, &spend],
            1,
        ),
        Err(WitnessBlockError::UnsupportedNetwork)
    );
    assert_eq!(
        check(&block, &[&cb, &spend], 0),
        Err(WitnessBlockError::InvalidSelectedInput)
    );
    assert_eq!(
        check(&block, &[&cb], 0),
        Err(WitnessBlockError::InvalidTransactionCount)
    );
    let too_many: Vec<&[u8]> = (0..=MAX_UNVERIFIED_BLOCK_TRANSACTIONS)
        .map(|_| spend.as_slice())
        .collect();
    assert_eq!(
        check(&block, &too_many, 1),
        Err(WitnessBlockError::InvalidTransactionCount)
    );
    let mut orphaned = vector("baseline_header");
    orphaned[4] ^= 1;
    let anchor = Tip {
        height: 100,
        id: BlockId([0x34; 32]),
    };
    let first_header: [u8; 80] = raw(FUNDING_HEADER_101).try_into().unwrap();
    let second_header: [u8; 80] = orphaned.try_into().unwrap();
    assert!(CheckedHeaderSegment::check_against_claimed_anchor(
        anchor,
        &[first_header, second_header],
        BitcoinPowProfile::RegtestLike
    )
    .is_err());
}

#[test]
fn bogus_extra_coinbase_and_malformed_selected_witness_never_gain_authority() {
    let cb = vector("baseline_coinbase");
    let spend = stage25j("spending_hex");
    let block = segment(&vector("baseline_header"));
    let mut fake_second = spend.clone();
    // The first outpoint starts after version, SegWit marker/flag and vin count.
    fake_second[7..39].fill(0);
    fake_second[39..43].fill(0xff);
    assert_eq!(
        check(&block, &[&cb, &fake_second], 1),
        Err(WitnessBlockError::UnexpectedCoinbase)
    );
    let mut corrupt = spend.clone();
    let der = [0x30u8, 0x44, 0x02, 0x20, 0x21, 0xd7, 0xc0, 0x87];
    let start = corrupt.windows(der.len()).position(|w| w == der).unwrap();
    corrupt[start] = 0x31;
    assert_eq!(
        check(&block, &[&cb, &corrupt], 1),
        Err(WitnessBlockError::WitnessCommitmentMismatch)
    );
}
