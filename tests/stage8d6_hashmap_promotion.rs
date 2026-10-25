//! Stage8D6: hash-index promotion compared to original one-team BTreeMap.
/// Independent of BTC consensus or TradeLayer ledger authorization.
use tradelayer_core::parallel_txindex::{ParallelIndexConfig, ParallelIndexError};
use tradelayer_core::raw_block_ingest::{
    merkle_root_unverified, RawBlockIngestError, RawBlockTrustError, UnverifiedRawBlockView,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn cfg(workers: usize, window_transactions: usize) -> ParallelIndexConfig {
    ParallelIndexConfig {
        workers,
        window_transactions,
    }
}
fn source() -> Vec<u8> {
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
fn block(transactions: &[Vec<u8>]) -> Vec<u8> {
    let mut raw = source()[..80].to_vec();
    let txids = transactions
        .iter()
        .map(|tx| inspect_transaction(tx).unwrap().txid)
        .collect::<Vec<_>>();
    raw[36..68].copy_from_slice(&merkle_root_unverified(&txids).unwrap());
    compact(&mut raw, transactions.len());
    for transaction in transactions {
        raw.extend_from_slice(transaction);
    }
    raw
}
fn mixed_block(count: usize, witness_percent: usize) -> Vec<u8> {
    let raw = source();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let mut txs = vec![view.raw_transaction(0).unwrap().to_vec()];
    for index in 1..count {
        let witness = (index * 37) % 100 < witness_percent;
        let mut tx = view
            .raw_transaction(if witness { 2 } else { 1 })
            .unwrap()
            .to_vec();
        let prevout = if witness { 7 } else { 5 };
        tx[prevout..prevout + 8].copy_from_slice(&(index as u64).to_le_bytes());
        txs.push(tx);
    }
    block(&txs)
}
fn compare(view: &UnverifiedRawBlockView<'_>, config: ParallelIndexConfig) {
    let original = view.inspect_parallel_one_team_tree_reference(config).unwrap();
    let hashed = view.inspect_parallel_one_team(config).unwrap();
    assert_eq!(hashed, original);
    assert_eq!(hashed.txid_merkle_root_raw(), view.header_merkle_root_raw());
    assert_eq!(hashed.ordered_preindex().ordered(), original.ordered_preindex().ordered());
    assert_eq!(
        hashed.require_authenticated_block(),
        Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified)
    );
    assert_eq!(
        hashed.require_script_and_utxo_consensus(),
        Err(RawBlockTrustError::ScriptAndUtxoStateUnverified)
    );
    assert_eq!(
        hashed.require_tradelayer_state_transition(),
        Err(RawBlockTrustError::TradeLayerStateAndSignedRevisionUnverified)
    );
}

#[test]
fn independent_four_tx_vector_preserves_all_worker_and_window_variants() {
    let raw = source();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let serial = view.inspect_serial().unwrap();
    for workers in [1, 2, 4, 8, 16] {
        for window in [1, 7, 64, 256, 512] {
            let config = cfg(workers, window);
            compare(&view, config);
            assert_eq!(view.inspect_parallel_one_team(config).unwrap(), serial);
        }
    }
}

#[test]
fn authentic_mainnet_block_one_coinbase_matches_both_reducers() {
    // Independently published mainnet block-1 header and serialized coinbase.
    // The header Merkle is checked locally, not the active chain or scripts.
    let header_hex = include_str!("../fixtures/bitcoin_mainnet_genesis_block1_headers.txt")
        .lines()
        .nth(1)
        .unwrap();
    let tx_hex = include_str!("../fixtures/bitcoin_mainnet_block1_coinbase.hex").trim();
    let mut raw = decode_hex(header_hex).unwrap();
    raw.push(1);
    raw.extend_from_slice(&decode_hex(tx_hex).unwrap());
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    assert_eq!(view.transaction_count(), 1);
    for workers in [1, 2, 4, 8] {
        compare(&view, cfg(workers, 512));
    }
}

#[test]
fn mixed_witness_parity_and_ten_thousand_transaction_stress() {
    for witness_percent in [0, 50, 95] {
        let raw = mixed_block(1_023, witness_percent);
        let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
        let reference = view.inspect_parallel(cfg(4, 512)).unwrap();
        for workers in [1, 2, 4, 8, 16] {
            for window in [1, 64, 512] {
                let config = cfg(workers, window);
                compare(&view, config);
                assert_eq!(view.inspect_parallel_one_team(config).unwrap(), reference);
            }
        }
    }
    let raw = mixed_block(10_000, 95);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    for workers in [2, 4, 8, 16] {
        compare(&view, cfg(workers, 512));
    }
}

#[test]
fn duplicate_txid_still_reports_first_canonical_position() {
    let raw = source();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let coinbase = view.raw_transaction(0).unwrap().to_vec();
    let a = view.raw_transaction(1).unwrap().to_vec();
    let b = view.raw_transaction(2).unwrap().to_vec();
    let raw = block(&[coinbase, a.clone(), b, a]);
    let duplicate = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let expected = Err(RawBlockIngestError::TransactionPreindex(
        ParallelIndexError::DuplicateTxid {
            first: 1,
            second: 3,
        },
    ));
    for workers in [1, 2, 4, 8, 16] {
        for window in [1, 2, 512] {
            let config = cfg(workers, window);
            assert_eq!(duplicate.inspect_parallel_one_team(config), expected);
            assert_eq!(
                duplicate.inspect_parallel_one_team_tree_reference(config),
                expected
            );
        }
    }
}

#[test]
fn malformed_configuration_denied_before_index_allocations() {
    let raw = source();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    for config in [cfg(0, 64), cfg(17, 64), cfg(4, 0), cfg(4, 513)] {
        let expected = Err(RawBlockIngestError::TransactionPreindex(
            ParallelIndexError::InvalidConfig,
        ));
        assert_eq!(view.inspect_parallel_one_team(config), expected);
        assert_eq!(
            view.inspect_parallel_one_team_tree_reference(config),
            expected
        );
    }
}
