//! Stage8D2 opt-in static source-contiguous CPU shards, balanced by estimated
//! SHA256d bytes. All results are observations, never Bitcoin/TL authority.
use tradelayer_core::parallel_txindex::{ParallelIndexConfig, ParallelIndexError};
use tradelayer_core::raw_block_ingest::{
    merkle_root_unverified, RawBlockIngestError, RawBlockTrustError, UnverifiedRawBlockView,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn source_block() -> Vec<u8> {
    decode_hex(include_str!("../fixtures/raw_block_ingest_synthetic.hex").trim()).unwrap()
}

fn config(workers: usize, window_transactions: usize) -> ParallelIndexConfig {
    ParallelIndexConfig {
        workers,
        window_transactions,
    }
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

fn block(transactions: &[Vec<u8>]) -> Vec<u8> {
    let mut raw = source_block()[..80].to_vec();
    let txids = transactions
        .iter()
        .map(|tx| inspect_transaction(tx).unwrap().txid)
        .collect::<Vec<_>>();
    raw[36..68].copy_from_slice(&merkle_root_unverified(&txids).unwrap());
    compact(&mut raw, transactions.len());
    for tx in transactions {
        raw.extend_from_slice(tx);
    }
    raw
}

fn sized_block(count: usize, witness_percent: usize) -> Vec<u8> {
    let raw = source_block();
    let source = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let mut transactions = vec![source.raw_transaction(0).unwrap().to_vec()];
    for i in 1..count {
        let segwit = (i * 37) % 100 < witness_percent;
        let mut tx = source
            .raw_transaction(if segwit { 2 } else { 1 })
            .unwrap()
            .to_vec();
        let prevout_start = if segwit { 7 } else { 5 };
        tx[prevout_start..prevout_start + 8].copy_from_slice(&(i as u64).to_le_bytes());
        transactions.push(tx);
    }
    block(&transactions)
}

#[test]
fn pinned_source_matches_serial_windowed_and_dynamic_team() {
    let raw = source_block();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let reference = view.inspect_serial().unwrap();
    for workers in [1, 2, 4, 8, 16] {
        for window in [1, 7, 64, 256, 512] {
            let cfg = config(workers, window);
            assert_eq!(
                view.inspect_parallel_weighted_shards(cfg).unwrap(),
                reference,
                "shards workers={workers} window={window}"
            );
            assert_eq!(view.inspect_parallel_one_team(cfg).unwrap(), reference);
        }
    }
    let observed = view
        .inspect_parallel_weighted_shards(config(4, 512))
        .unwrap();
    assert_eq!(
        observed.require_authenticated_block(),
        Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified)
    );
    assert_eq!(
        observed.require_script_and_utxo_consensus(),
        Err(RawBlockTrustError::ScriptAndUtxoStateUnverified)
    );
    assert_eq!(
        observed.require_tradelayer_state_transition(),
        Err(RawBlockTrustError::TradeLayerStateAndSignedRevisionUnverified)
    );
}

#[test]
fn mixed_witness_blocks_match_all_workers_and_windows() {
    for witness_percent in [0, 50, 95] {
        let raw = sized_block(1_023, witness_percent);
        let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
        let reference = view.inspect_parallel(config(4, 512)).unwrap();
        for workers in [1, 2, 4, 8, 16] {
            for window in [1, 64, 512] {
                let actual = view
                    .inspect_parallel_weighted_shards(config(workers, window))
                    .unwrap();
                assert_eq!(
                    actual, reference,
                    "witness={witness_percent} workers={workers} window={window}"
                );
                assert_eq!(
                    actual.txid_merkle_root_raw(),
                    view.header_merkle_root_raw()
                );
            }
        }
    }
}

#[test]
fn ten_thousand_transaction_source_order_is_exact() {
    let raw = sized_block(10_000, 95);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let expected = view.inspect_parallel_one_team(config(8, 512)).unwrap();
    for workers in [2, 4, 8, 16] {
        let actual = view
            .inspect_parallel_weighted_shards(config(workers, 512))
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.ordered_preindex().transaction_count(), 10_000);
    }
}

#[test]
fn duplicate_txid_in_later_shard_reports_original_position() {
    let raw = source_block();
    let base = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let coinbase = base.raw_transaction(0).unwrap().to_vec();
    let a = base.raw_transaction(1).unwrap().to_vec();
    let b = base.raw_transaction(2).unwrap().to_vec();
    let raw = block(&[coinbase, a.clone(), b, a]);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let error = RawBlockIngestError::TransactionPreindex(ParallelIndexError::DuplicateTxid {
        first: 1,
        second: 3,
    });
    for workers in [1, 2, 3, 4, 8, 16] {
        assert_eq!(
            view.inspect_parallel_weighted_shards(config(workers, 2)),
            Err(error.clone())
        );
    }
}

#[test]
fn invalid_configuration_fails_closed() {
    let raw = source_block();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    for cfg in [config(0, 32), config(17, 32), config(4, 0), config(4, 513)] {
        assert_eq!(
            view.inspect_parallel_weighted_shards(cfg),
            Err(RawBlockIngestError::TransactionPreindex(
                ParallelIndexError::InvalidConfig
            ))
        );
    }
}
