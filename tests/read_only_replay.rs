//! Offline evidence-based tests. Fixtures are observations, not consensus proofs.
use tradelayer_core::read_only::{
    decode_hex, id_hex, inspect_transaction, replay_tsv, CarrierFormat, ScanError,
};

const HISTORICAL: &str = include_str!("../fixtures/real_transactions.tsv");
const SYNTHETIC: &str = include_str!("../fixtures/synthetic_transactions.tsv");

#[test]
fn real_bitcoin_transactions_match_independently_published_txids() {
    let rows = replay_tsv(HISTORICAL).unwrap();
    assert_eq!(rows.len(), 3);
    let genesis = &rows[0];
    assert_eq!(genesis.reported_height, Some(0));
    assert_eq!(id_hex(&genesis.observation.txid), "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b");
    assert_eq!(genesis.observation.input_count, 1);
    assert_eq!(genesis.observation.output_count, 1);
    assert_eq!(genesis.observation.op_return_count, 0);
    assert!(!genesis.observation.has_witness);
    assert!(genesis.observation.carriers.is_empty());

    let other = &rows[1];
    assert_eq!(id_hex(&other.observation.txid), "d38ff6587f43cea5a399462bc735920f30073bbe06b5d7df5154196c5dd8d637");
    assert_eq!(other.observation.output_count, 2);
    assert!(other.observation.carriers.is_empty());
    assert_eq!(other.reported_height, None);

    let witness = &rows[2];
    assert!(witness.observation.has_witness);
    assert_eq!(witness.observation.input_count, 1);
    assert_eq!(witness.observation.output_count, 2);
    assert_eq!(witness.observation.op_return_count, 1);
    assert!(witness.observation.carriers.is_empty());
    assert_eq!(id_hex(&witness.observation.txid), "00fddd900b735a88ab0ef5eeaa3efbc869d4e52493da8a9467ec782355cb39ff");
    assert_eq!(id_hex(&witness.observation.wtxid), "b4c4817e908d9ba317e9b1a08180b51c0ce77adca946e7f894560fb07447acf8");
    assert_ne!(witness.observation.txid, witness.observation.wtxid);
}

#[test]
fn synthetic_tl_carriers_stay_read_only() {
    let rows = replay_tsv(SYNTHETIC).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].observation.carriers[0].format, CarrierFormat::LegacyAscii);
    assert_eq!(rows[0].observation.carriers[0].transaction_type, Some(2));
    assert_eq!(rows[0].observation.carriers[0].status, "typed-syntax");
    assert_eq!(rows[1].observation.carriers[0].format, CarrierFormat::ProposedBinaryV1);
    assert_eq!(rows[1].observation.carriers[0].transaction_type, Some(2));
    assert_eq!(rows[1].observation.carriers[0].status, "typed-syntax");
    assert_eq!(rows[2].observation.carriers[0].status, "invalid-body");
    for row in rows {
        assert_eq!(row.observation.op_return_count, 1);
        assert_eq!(row.observation.carriers.len(), 1);
        assert_eq!(row.observation.carriers[0].vout, 0);
        assert_eq!(row.reported_height, None);
    }
}

#[test]
fn replay_refuses_mismatches_and_duplicate_evidence() {
    let rows = replay_tsv(HISTORICAL).unwrap();
    assert_eq!(rows.len(), 3);
    let first = HISTORICAL.lines().nth(1).unwrap();
    let mismatch = first.replace("4a5e1e4b", "5a5e1e4b");
    assert!(replay_tsv(&mismatch).unwrap_err().reason.contains("txid mismatch"));
    let duplicate = format!("{first}\n{first}\n");
    assert!(replay_tsv(&duplicate).unwrap_err().reason.contains("duplicate"));
    let corrupted = first.replace("0100000001", "0200000001");
    assert!(replay_tsv(&corrupted).is_err());
    let badly_formed = format!("{first}\tgarbage\n");
    assert!(replay_tsv(&badly_formed).is_err());
    let height_error = HISTORICAL.replace("bitcoin-genesis\tbitcoin-mainnet\t0\t", "bitcoin-genesis\tbitcoin-mainnet\tx\t");
    assert!(replay_tsv(&height_error).is_err());
}

#[test]
fn parser_rejects_witness_flag_and_trailing_bytes() {
    let line = HISTORICAL.lines().nth(3).unwrap();
    let raw_hex = line.split('\t').nth(5).unwrap();
    let mut tx = decode_hex(raw_hex).unwrap();
    tx[5] = 2;
    assert!(inspect_transaction(&tx).is_err());
    let mut tx = decode_hex(raw_hex).unwrap();
    tx.push(1);
    assert_eq!(inspect_transaction(&tx), Err(ScanError::TrailingBytes));
    let mut tx = decode_hex(raw_hex).unwrap();
    tx.truncate(35);
    assert!(inspect_transaction(&tx).is_err());
}

#[test]
fn deterministic_byte_fuzz_has_no_panics() {
    let mut state = 0x626974636f696e31_u64;
    for _ in 0..4000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let len = (state % 260) as usize;
        let mut bytes = vec![0u8; len];
        for byte in &mut bytes {
            state = state.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
            *byte = (state >> 24) as u8;
        }
        let _ = inspect_transaction(&bytes);
    }
}
