//! Stage8D1b opt-in persistent scoped CPU worker team against original
//! bounded-window TX preindex. NO full Bitcoin Script/UTXO or TL consensus.
use tradelayer_core::parallel_txindex::{ParallelIndexConfig, ParallelIndexError};
use tradelayer_core::raw_block_ingest::{
    merkle_root_unverified, RawBlockIngestError, RawBlockTrustError, UnverifiedRawBlockView,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn source_block() -> Vec<u8> {
    decode_hex(include_str!("../fixtures/raw_block_ingest_synthetic.hex").trim()).unwrap()
}
fn config(workers: usize, window_transactions: usize) -> ParallelIndexConfig {
    ParallelIndexConfig { workers, window_transactions }
}
fn compact(buf: &mut Vec<u8>, number: usize) {
    if number < 253 {
        buf.push(number as u8);
    } else if number <= u16::MAX as usize {
        buf.push(0xfd);
        buf.extend_from_slice(&(number as u16).to_le_bytes());
    } else {
        buf.push(0xfe);
        buf.extend_from_slice(&(number as u32).to_le_bytes());
    }
}
fn block(transactions: &[Vec<u8>]) -> Vec<u8> {
    let mut buf = source_block()[..80].to_vec();
    let hashes: Vec<_> = transactions.iter()
        .map(|t| inspect_transaction(t).unwrap().txid)
        .collect();
    let root = merkle_root_unverified(&hashes).unwrap();
    buf[36..68].copy_from_slice(&root);
    compact(&mut buf, transactions.len());
    for tx in transactions {
        buf.extend_from_slice(tx);
    }
    buf
}
fn sized_block(count: usize, witness_percentage: usize) -> Vec<u8> {
    let bytes = source_block();
    let view = UnverifiedRawBlockView::parse(&bytes, None).unwrap();
    let coinbase = view.raw_transaction(0).unwrap().to_vec();
    let legacy = view.raw_transaction(1).unwrap();
    let segwit = view.raw_transaction(2).unwrap();
    let mut records = vec![coinbase];
    for index in 1..count {
        let is_segwit = (index * 37) % 100 < witness_percentage;
        let mut tx = if is_segwit { segwit.to_vec() } else { legacy.to_vec() };
        let start = if is_segwit { 7 } else { 5 };
        tx[start..start + 8].copy_from_slice(&(index as u64).to_le_bytes());
        records.push(tx);
    }
    block(&records)
}

#[test]
fn source_four_transaction_block_matches_all_existing_worker_variants() {
    let raw = source_block();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let serial = view.inspect_serial().unwrap();
    assert_eq!(serial.ordered_preindex().candidate_positions(), &[1, 2]);
    for workers in [1, 2, 4, 8, 16] {
        for windows in [1, 7, 64, 256, 512] {
            let original = view.inspect_parallel(config(workers, windows)).unwrap();
            let team = view.inspect_parallel_one_team(config(workers, windows)).unwrap();
            assert_eq!(original, serial, "original workers={workers} windows={windows}");
            assert_eq!(team, serial, "pooled workers={workers} windows={windows}");
            assert_eq!(team.block_weight(), view.block_weight());
            assert_eq!(team.require_authenticated_block(),
                Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified));
            assert_eq!(team.require_script_and_utxo_consensus(),
                Err(RawBlockTrustError::ScriptAndUtxoStateUnverified));
            assert_eq!(team.require_tradelayer_state_transition(),
                Err(RawBlockTrustError::TradeLayerStateAndSignedRevisionUnverified));
        }
    }
}

#[test]
fn large_mixed_block_preserves_source_order_duplicate_check_and_merkles() {
    for witness_percentage in [0, 50, 95] {
        let raw = sized_block(1_023, witness_percentage);
        let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
        let old = view.inspect_parallel(config(4, 512)).unwrap();
        for workers in [1, 2, 4, 8, 16] {
            for windows in [1, 7, 64, 512] {
                let one_team = view.inspect_parallel_one_team(config(workers, windows)).unwrap();
                assert_eq!(one_team, old, "witness={witness_percentage} workers={workers} windows={windows}");
                assert_eq!(one_team.ordered_preindex().transaction_count(), 1_023);
                assert_eq!(one_team.txid_merkle_root_raw(), view.header_merkle_root_raw());
            }
        }
    }
}

#[test]
fn ten_thousand_transaction_stress_with_opt_in_one_scope_and_16_workers() {
    let raw = sized_block(10_000, 50);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let original = view.inspect_parallel(config(4, 512)).unwrap();
    for (workers, window) in [(1, 64), (2, 512), (4, 256), (8, 512), (16, 512)] {
        let pooled = view.inspect_parallel_one_team(config(workers, window)).unwrap();
        assert_eq!(pooled, original, "workers={workers}, claim window={window}");
        assert_eq!(pooled.ordered_preindex().candidate_positions().len(), 9_999);
    }
}

#[test]
fn nonadjacent_duplicate_txid_reports_original_earliest_index() {
    let source = source_block();
    let base = UnverifiedRawBlockView::parse(&source, None).unwrap();
    let coinbase = base.raw_transaction(0).unwrap().to_vec();
    let a = base.raw_transaction(1).unwrap().to_vec();
    let b = base.raw_transaction(2).unwrap().to_vec();
    let raw = block(&[coinbase, a.clone(), b, a]);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let expected = RawBlockIngestError::TransactionPreindex(
        ParallelIndexError::DuplicateTxid { first: 1, second: 3 }
    );
    for workers in [1, 2, 4, 8, 16] {
        assert_eq!(view.inspect_parallel_one_team(config(workers, 2)), Err(expected.clone()));
        assert_eq!(view.inspect_parallel(config(workers, 2)), Err(expected.clone()));
    }
}
#[test]
fn invalid_worker_and_window_configurations_reject_before_any_work() {
    let raw = source_block();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    for invalid in [config(0, 64), config(17, 64), config(4, 0), config(4, 513)] {
        assert_eq!(view.inspect_parallel_one_team(invalid), Err(
            RawBlockIngestError::TransactionPreindex(ParallelIndexError::InvalidConfig)
        ));
    }
}
