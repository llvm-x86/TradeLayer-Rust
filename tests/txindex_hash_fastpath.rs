//! Independent pinned hashlib fixture plus large mixed SegWit source-order
//! equivalence for the CPU preindexer. All TXIDs are observations only.
use tradelayer_core::parallel_txindex::{
    inspect_unverified_block_parallel, inspect_unverified_block_serial, ParallelIndexConfig,
    ParallelIndexTrustError,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

struct HashFixture {
    label: &'static str,
    raw: Vec<u8>,
    stripped: Vec<u8>,
    txid: [u8; 32],
    wtxid: [u8; 32],
    has_witness: bool,
    carriers: usize,
}

fn source_fixture() -> Vec<HashFixture> {
    include_str!("../fixtures/txindex_segwit_hash_vectors.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("label\t"))
        .map(|line| {
            let parts: Vec<_> = line.split('\t').collect();
            assert_eq!(parts.len(), 7);
            HashFixture {
                label: parts[0],
                raw: decode_hex(parts[1]).unwrap(),
                stripped: decode_hex(parts[2]).unwrap(),
                txid: decode_hex(parts[3]).unwrap().try_into().unwrap(),
                wtxid: decode_hex(parts[4]).unwrap().try_into().unwrap(),
                has_witness: parts[5] == "1",
                carriers: parts[6].parse().unwrap(),
            }
        })
        .collect()
}
#[test]
fn hashlib_legacy_and_segwit_raw_txids_are_byte_exact() {
    let cases = source_fixture();
    assert_eq!(cases.len(), 3);
    for HashFixture {
        label,
        raw,
        stripped,
        txid,
        wtxid,
        has_witness,
        carriers,
    } in cases {
        let observed = inspect_transaction(&raw).unwrap();
        assert_eq!(observed.txid, txid, "{label}: txid");
        assert_eq!(observed.wtxid, wtxid, "{label}: wtxid");
        assert_eq!(observed.has_witness, has_witness, "{label}: witness marker");
        assert_eq!(observed.carriers.len(), carriers, "{label}: carriers");
        assert_eq!(
            inspect_transaction(&stripped).unwrap().txid,
            txid,
            "{label}: stripped"
        );
        if !has_witness {
            assert_eq!(observed.txid, observed.wtxid, "{label}: identical hashes");
        } else {
            assert_ne!(
                observed.txid, observed.wtxid,
                "{label}: signed-witness hash"
            );
        }
    }
}
#[test]
fn mixed_legacy_and_witness_source_order_remains_identical_parallel_or_serial() {
    let cases = source_fixture();
    let legacy = &cases[0].raw;
    let witness = &cases[1].raw;
    let mut raws = Vec::with_capacity(1047);
    for index in 0..1047u32 {
        let mut tx = if index % 3 == 0 {
            witness.clone()
        } else {
            legacy.clone()
        };
        // Only a synthetic first-input parent outpoint is varied for uniqueness.
        // Offset: 4 version + (2 witness flag) + 1 input count.
        let outpoint_start = if index % 3 == 0 { 7 } else { 5 };
        tx[outpoint_start..outpoint_start + 4].copy_from_slice(&index.to_le_bytes());
        raws.push(tx);
    }
    let serial = inspect_unverified_block_serial(&raws).unwrap();
    assert_eq!(serial.transaction_count(), 1047);
    assert_eq!(serial.candidate_positions().len(), 1047);
    for workers in [1, 2, 4, 8, 16] {
        for window in [1, 7, 64, 512] {
            let parallel = inspect_unverified_block_parallel(
                &raws,
                ParallelIndexConfig {
                    workers,
                    window_transactions: window,
                },
            )
            .unwrap();
            assert_eq!(parallel, serial, "workers={workers} window={window}");
            assert_eq!(
                parallel.require_authenticated_transaction_state(),
                Err(ParallelIndexTrustError::ScriptAndStateNotVerified)
            );
        }
    }
}
