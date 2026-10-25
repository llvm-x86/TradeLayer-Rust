//! Stage8D4 persistent read-only TXID worker team across bounded windows,
//! dynamically claiming byte-weighted work. No Bitcoin/TradeLayer consensus.
use tradelayer_core::parallel_txindex::{ParallelIndexConfig, ParallelIndexError};
use tradelayer_core::raw_block_ingest::{
    merkle_root_unverified, RawBlockIngestError, RawBlockTrustError, UnverifiedRawBlockView,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn config(workers: usize, window_transactions: usize) -> ParallelIndexConfig {
    ParallelIndexConfig {
        workers,
        window_transactions,
    }
}
fn source_block() -> Vec<u8> {
    decode_hex(include_str!("../fixtures/raw_block_ingest_synthetic.hex").trim()).unwrap()
}
fn compact(buf: &mut Vec<u8>, count: usize) {
    if count < 253 {
        buf.push(count as u8);
    } else if count <= u16::MAX as usize {
        buf.push(0xfd);
        buf.extend_from_slice(&(count as u16).to_le_bytes());
    } else {
        buf.push(0xfe);
        buf.extend_from_slice(&(count as u32).to_le_bytes());
    }
}
fn block(txs: &[Vec<u8>]) -> Vec<u8> {
    let mut buf = source_block()[..80].to_vec();
    let txids = txs
        .iter()
        .map(|tx| inspect_transaction(tx).unwrap().txid)
        .collect::<Vec<_>>();
    buf[36..68].copy_from_slice(&merkle_root_unverified(&txids).unwrap());
    compact(&mut buf, txs.len());
    for tx in txs {
        buf.extend_from_slice(tx);
    }
    buf
}
fn sized_block(count: usize, witness_percent: usize) -> Vec<u8> {
    let raw = source_block();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let coinbase = view.raw_transaction(0).unwrap().to_vec();
    let legacy = view.raw_transaction(1).unwrap();
    let witness = view.raw_transaction(2).unwrap();
    let mut rows = vec![coinbase];
    for index in 1..count {
        let segwit = (index * 37) % 100 < witness_percent;
        let mut tx = if segwit {
            witness.to_vec()
        } else {
            legacy.to_vec()
        };
        let offset = if segwit { 7 } else { 5 };
        tx[offset..offset + 8].copy_from_slice(&(index as u64).to_le_bytes());
        rows.push(tx);
    }
    block(&rows)
}

#[test]
fn pinned_source_equal_for_all_worker_and_window_settings() {
    let bytes = source_block();
    let view = UnverifiedRawBlockView::parse(&bytes, None).unwrap();
    let serial = view.inspect_serial().unwrap();
    assert_eq!(serial.ordered_preindex().candidate_positions(), &[1, 2]);
    for workers in [1, 2, 4, 8, 16] {
        for window in [1, 7, 64, 256, 512] {
            let cfg = config(workers, window);
            let pooled = view.inspect_parallel_pooled_weighted(cfg).unwrap();
            assert_eq!(pooled, serial, "pool workers={workers} window={window}");
            assert_eq!(pooled, view.inspect_parallel_one_team(cfg).unwrap());
            assert_eq!(pooled, view.inspect_parallel(cfg).unwrap());
        }
    }
    let pooled = view
        .inspect_parallel_pooled_weighted(config(4, 2))
        .unwrap();
    assert_eq!(
        pooled.require_authenticated_block(),
        Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified)
    );
    assert_eq!(
        pooled.require_script_and_utxo_consensus(),
        Err(RawBlockTrustError::ScriptAndUtxoStateUnverified)
    );
    assert_eq!(
        pooled.require_tradelayer_state_transition(),
        Err(RawBlockTrustError::TradeLayerStateAndSignedRevisionUnverified)
    );
}

#[test]
fn bounded_window_crossings_keep_all_source_observations() {
    for witness_percent in [0, 50, 95] {
        let raw = sized_block(1_023, witness_percent);
        let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
        let expected = view.inspect_parallel(config(4, 512)).unwrap();
        for workers in [1, 2, 4, 8, 16] {
            for window in [1, 7, 64, 256, 512] {
                let cfg = config(workers, window);
                let observed = view.inspect_parallel_pooled_weighted(cfg).unwrap();
                assert_eq!(
                    observed, expected,
                    "witness={witness_percent}, workers={workers}, window={window}"
                );
                assert_eq!(
                    observed.txid_merkle_root_raw(),
                    view.header_merkle_root_raw()
                );
            }
        }
    }
}

#[test]
fn stress_ten_thousand_transactions_across_many_windows() {
    let raw = sized_block(10_000, 95);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let expected = view.inspect_parallel_one_team(config(8, 512)).unwrap();
    for (workers, window) in [(2, 7), (4, 64), (8, 512), (16, 256)] {
        let observed = view
            .inspect_parallel_pooled_weighted(config(workers, window))
            .unwrap();
        assert_eq!(observed, expected, "workers={workers} window={window}");
        assert_eq!(observed.ordered_preindex().transaction_count(), 10_000);
    }
}

#[test]
fn repeated_txid_in_another_window_preserves_first_failure_index() {
    let source = source_block();
    let view = UnverifiedRawBlockView::parse(&source, None).unwrap();
    let coinbase = view.raw_transaction(0).unwrap().to_vec();
    let a = view.raw_transaction(1).unwrap().to_vec();
    let b = view.raw_transaction(2).unwrap().to_vec();
    let raw = block(&[coinbase, a.clone(), b, a]);
    let block = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let expected = RawBlockIngestError::TransactionPreindex(ParallelIndexError::DuplicateTxid {
        first: 1,
        second: 3,
    });
    for workers in [1, 2, 4, 8, 16] {
        for window in [1, 2, 3, 4, 512] {
            assert_eq!(
                block.inspect_parallel_pooled_weighted(config(workers, window)),
                Err(expected.clone()),
                "workers={workers} window={window}"
            );
        }
    }
}

#[test]
fn input_validation_and_transaction_count_are_unchanged() {
    let raw = source_block();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    for cfg in [config(0, 64), config(17, 64), config(4, 0), config(4, 513)] {
        assert_eq!(
            view.inspect_parallel_pooled_weighted(cfg),
            Err(RawBlockIngestError::TransactionPreindex(
                ParallelIndexError::InvalidConfig
            ))
        );
    }
}
