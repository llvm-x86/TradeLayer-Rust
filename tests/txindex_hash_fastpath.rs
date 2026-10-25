//! Independent pinned hashlib fixture plus large mixed SegWit source-order
//! equivalence for the CPU preindexer. All TXIDs are observations only.
use tradelayer_core::parallel_txindex::{
    inspect_unverified_block_parallel, inspect_unverified_block_serial, ParallelIndexConfig,
    ParallelIndexTrustError,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn source_fixture() -> Vec<(&'static str, Vec<u8>, Vec<u8>, [u8; 32], [u8; 32], bool, usize)> {
    include_str!("../fixtures/txindex_segwit_hash_vectors.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("label\t"))
        .map(|line| {
            let parts: Vec<_> = line.split('\t').collect();
            assert_eq!(parts.len(), 7);
            (
                parts[0],
                decode_hex(parts[1]).unwrap(),
                decode_hex(parts[2]).unwrap(),
                decode_hex(parts[3]).unwrap().try_into().unwrap(),
                decode_hex(parts[4]).unwrap().try_into().unwrap(),
                parts[5] == "1",
                parts[6].parse().unwrap(),
            )
        })
        .collect()
}
#[test]
fn hashlib_legacy_and_segwit_raw_txids_are_byte_exact() {
    let cases = source_fixture();
    assert_eq!(cases.len(), 3);
    for (label, raw, stripped, txid, wtxid, witness, carriers) in cases {
        let observed = inspect_transaction(&raw).unwrap();
        assert_eq!(observed.txid, txid, "{label}: txid");
        assert_eq!(observed.wtxid, wtxid, "{label}: wtxid");
        assert_eq!(observed.has_witness, witness, "{label}: witness marker");
        assert_eq!(observed.carriers.len(), carriers, "{label}: carriers");
        assert_eq!(inspect_transaction(&stripped).unwrap().txid, txid, "{label}: stripped");
        if !witness {
            assert_eq!(observed.txid, observed.wtxid, "{label}: identical hashes");
        } else {
            assert_ne!(observed.txid, observed.wtxid, "{label}: signed-witness hash");
        }
    }
}
#[test]
fn mixed_legacy_and_witness_source_order_remains_identical_parallel_or_serial() {
    let cases = source_fixture();
    let legacy = &cases[0].1;
    let witness = &cases[1].1;
    let mut raws = Vec::with_capacity(1047);
    for index in 0..1047u32 {
        let mut tx = if index % 3 == 0 { witness.clone() } else { legacy.clone() };
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
                ParallelIndexConfig { workers, window_transactions: window },
            ).unwrap();
            assert_eq!(parallel, serial, "workers={workers} window={window}");
            assert_eq!(
                parallel.require_authenticated_transaction_state(),
                Err(ParallelIndexTrustError::ScriptAndStateNotVerified)
            );
        }
    }
}
