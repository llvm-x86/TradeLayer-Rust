//! Stage 8D5: source-ordered TXID deduplication and in-place Merkle
//! experiments. These are UNVERIFIED read-only observations, never consensus.
use tradelayer_core::parallel_txindex::{ParallelIndexConfig, ParallelIndexError};
use tradelayer_core::raw_block_ingest::{
    merkle_root_unverified, merkle_root_unverified_inplace, RawBlockIngestError, RawBlockTrustError,
    UnverifiedRawBlockView,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn config(workers: usize, window_transactions: usize) -> ParallelIndexConfig {
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

fn block(txs: &[Vec<u8>]) -> Vec<u8> {
    let mut raw = source()[..80].to_vec();
    let txids = txs
        .iter()
        .map(|tx| inspect_transaction(tx).unwrap().txid)
        .collect::<Vec<_>>();
    raw[36..68].copy_from_slice(&merkle_root_unverified(&txids).unwrap());
    compact(&mut raw, txs.len());
    for tx in txs {
        raw.extend_from_slice(tx);
    }
    raw
}

fn sized_block(count: usize, witness_percentage: usize) -> Vec<u8> {
    let raw = source();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let mut txs = vec![view.raw_transaction(0).unwrap().to_vec()];
    for index in 1..count {
        let segwit = (index * 37) % 100 < witness_percentage;
        let mut tx = view
            .raw_transaction(if segwit { 2 } else { 1 })
            .unwrap()
            .to_vec();
        let first_outpoint = if segwit { 7 } else { 5 };
        tx[first_outpoint..first_outpoint + 8].copy_from_slice(&(index as u64).to_le_bytes());
        txs.push(tx);
    }
    block(&txs)
}

fn compare_modes(view: &UnverifiedRawBlockView<'_>, cfg: ParallelIndexConfig) {
    let reference = view.inspect_parallel_one_team(cfg).unwrap();
    let hash_seen = view.inspect_parallel_one_team_hash_seen(cfg).unwrap();
    let inplace_merkle = view.inspect_parallel_one_team_inplace_merkle(cfg).unwrap();
    let both = view.inspect_parallel_one_team_fast_reduce(cfg).unwrap();
    assert_eq!(reference, hash_seen);
    assert_eq!(reference, inplace_merkle);
    assert_eq!(reference, both);
    assert_eq!(both.ordered_preindex(), reference.ordered_preindex());
    assert_eq!(both.txid_merkle_root_raw(), view.header_merkle_root_raw());
    assert_eq!(
        both.require_authenticated_block(),
        Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified)
    );
    assert_eq!(
        both.require_script_and_utxo_consensus(),
        Err(RawBlockTrustError::ScriptAndUtxoStateUnverified)
    );
    assert_eq!(
        both.require_tradelayer_state_transition(),
        Err(RawBlockTrustError::TradeLayerStateAndSignedRevisionUnverified)
    );
}

#[test]
fn bitcoin_merkle_odd_leaf_rule_and_mutation_errors_match_reference() {
    assert_eq!(
        merkle_root_unverified_inplace(&[]),
        Err(RawBlockIngestError::ZeroTransactions)
    );
    let leaves: Vec<[u8; 32]> = (0..200)
        .map(|index| {
            let mut txid = [0u8; 32];
            txid[..8].copy_from_slice(&(index as u64).to_le_bytes());
            txid[8] = 0xa7;
            txid
        })
        .collect();
    for count in [1, 2, 3, 4, 5, 7, 8, 9, 16, 31, 32, 33, 127, 128, 199, 200] {
        assert_eq!(
            merkle_root_unverified(&leaves[..count]),
            merkle_root_unverified_inplace(&leaves[..count]),
            "Merkle source count={count}"
        );
    }
    let duplicate_siblings = [leaves[0], leaves[0]];
    let higher_level_duplicate = [leaves[0], leaves[1], leaves[0], leaves[1]];
    for bad in [&duplicate_siblings[..], &higher_level_duplicate[..]] {
        assert_eq!(
            merkle_root_unverified(bad),
            Err(RawBlockIngestError::InvalidTxidMerkleTree)
        );
        assert_eq!(
            merkle_root_unverified_inplace(bad),
            Err(RawBlockIngestError::InvalidTxidMerkleTree)
        );
    }
    // Implicit Bitcoin odd-leaf duplication is allowed when the pair has
    // no second *actual* node. Last leaf is logically repeated for hashing.
    let three = [leaves[0], leaves[1], leaves[2]];
    assert_eq!(
        merkle_root_unverified(&three),
        merkle_root_unverified_inplace(&three)
    );
    assert_eq!(
        merkle_root_unverified_inplace(&leaves[..]),
        merkle_root_unverified(&leaves[..])
    );
    assert_eq!(
        merkle_root_unverified_inplace(&vec![[1u8; 32]; 200_001]),
        Err(RawBlockIngestError::TooManyTransactions)
    );
}

#[test]
fn pinned_four_transaction_observations_match_every_worker_setting() {
    let raw = source();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let serial = view.inspect_serial().unwrap();
    for workers in [1, 2, 4, 8, 16] {
        for window in [1, 7, 64, 256, 512] {
            let cfg = config(workers, window);
            compare_modes(&view, cfg);
            assert_eq!(view.inspect_parallel_one_team_fast_reduce(cfg).unwrap(), serial);
        }
    }
}

#[test]
fn mixed_seg_wit_and_large_block_preserve_exact_order() {
    for witness_percentage in [0, 50, 95] {
        let raw = sized_block(1_023, witness_percentage);
        let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
        for workers in [1, 2, 4, 8, 16] {
            for window in [1, 64, 512] {
                compare_modes(&view, config(workers, window));
            }
        }
    }
    let raw = sized_block(10_000, 95);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    for workers in [2, 4, 8, 16] {
        compare_modes(&view, config(workers, 512));
    }
}

#[test]
fn earliest_duplicate_txid_is_identical_in_all_reducers() {
    let raw = source();
    let original = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let coinbase = original.raw_transaction(0).unwrap().to_vec();
    let a = original.raw_transaction(1).unwrap().to_vec();
    let b = original.raw_transaction(2).unwrap().to_vec();
    let duplicate = block(&[coinbase, a.clone(), b, a]);
    let view = UnverifiedRawBlockView::parse(&duplicate, None).unwrap();
    let expected = Err(RawBlockIngestError::TransactionPreindex(
        ParallelIndexError::DuplicateTxid {
            first: 1,
            second: 3,
        },
    ));
    for workers in [1, 2, 4, 8, 16] {
        for window in [1, 2, 512] {
            let cfg = config(workers, window);
            assert_eq!(view.inspect_parallel_one_team(cfg), expected);
            assert_eq!(view.inspect_parallel_one_team_hash_seen(cfg), expected);
            assert_eq!(view.inspect_parallel_one_team_inplace_merkle(cfg), expected);
            assert_eq!(view.inspect_parallel_one_team_fast_reduce(cfg), expected);
        }
    }
}

#[test]
fn invalid_config_errors_precede_hash_index_allocation() {
    let raw = source();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    for cfg in [config(0, 16), config(17, 16), config(4, 0), config(4, 513)] {
        assert_eq!(
            view.inspect_parallel_one_team_fast_reduce(cfg),
            Err(RawBlockIngestError::TransactionPreindex(
                ParallelIndexError::InvalidConfig
            ))
        );
    }
}
