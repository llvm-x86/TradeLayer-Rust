//! Performance-critical per-block transaction pre-scan, without consensus.
use tradelayer_core::chain::{
    BlockHeader, BlockId, CandidateBlock, ChainId, MAX_TRANSACTIONS_PER_BLOCK,
};
use tradelayer_core::read_only::{id_hex, ScanError, MAX_RAW_TRANSACTION};
use tradelayer_core::txindex_parallel::{
    scan_candidate_block_parallel_unverified, scan_transactions_auto_unverified,
    scan_transactions_parallel_unverified,
    ParallelIndexError, ParallelScanConfig, AUTO_PARALLEL_THRESHOLD,
    MAX_PARALLEL_BLOCK_BYTES, MAX_PARALLEL_CHUNK, MAX_PARALLEL_WORKERS,
};

fn compact_size(len: usize, out: &mut Vec<u8>) {
    if len < 253 {
        out.push(len as u8);
    } else if len <= 65_535 {
        out.push(0xfd);
        out.extend_from_slice(&(len as u16).to_le_bytes());
    } else {
        out.push(0xfe);
        out.extend_from_slice(&(len as u32).to_le_bytes());
    }
}
/// Entirely fake Bitcoin-style wire serialization with a distinct first
/// previous-output TXID. No proof this spends an actual UTXO exists.
fn fake_tx(index: usize, tl: bool, extra_script_bytes: usize) -> Vec<u8> {
    let mut out = vec![2, 0, 0, 0, 1];
    let mut prev = [0x5au8; 32];
    prev[0..8].copy_from_slice(&(index as u64).to_le_bytes());
    out.extend_from_slice(&prev);
    out.extend_from_slice(&0u32.to_le_bytes());
    compact_size(extra_script_bytes, &mut out);
    out.extend(std::iter::repeat_n(0x51, extra_script_bytes));
    out.extend_from_slice(&0xffff_fffeu32.to_le_bytes());
    out.push(1);
    out.extend_from_slice(&0u64.to_le_bytes());
    let script = if tl {
        let data = b"tl20;alice;5;2";
        let mut script = vec![0x6a, data.len() as u8];
        script.extend_from_slice(data);
        script
    } else {
        let mut script=vec![0x00,0x14];
        script.extend(std::iter::repeat_n(0x11,20));
        script
    };
    compact_size(script.len(), &mut out);
    out.extend_from_slice(&script);
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}
fn synthetic_block(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| fake_tx(i, i % 97 == 5, (i % 5) * 120)).collect()
}
fn cfg(workers: usize, chunk_size: usize) -> ParallelScanConfig {
    ParallelScanConfig { workers, chunk_size }
}

#[test]
fn parallel_10k_serial_and_dynamic_workers_are_byte_for_byte_identical() {
    let block = synthetic_block(MAX_TRANSACTIONS_PER_BLOCK);
    let serial=scan_transactions_parallel_unverified(&block,cfg(1,16)).unwrap();
    assert_eq!(serial.ordered().len(),MAX_TRANSACTIONS_PER_BLOCK);
    assert_eq!(serial.ordered()[0].tx_index(),0);
    assert_eq!(serial.ordered()[9999].tx_index(),9999);
    assert!(serial.statistics().total_raw_bytes < MAX_PARALLEL_BLOCK_BYTES);
    assert_eq!(serial.statistics().workers_used,1);
    let candidate_positions=serial.trade_layer_candidates()
        .map(|row| row.tx_index()).collect::<Vec<_>>();
    assert_eq!(candidate_positions.len(),104);
    assert_eq!(candidate_positions[0],5);
    assert_eq!(candidate_positions[1],102);

    for (workers, chunk_size) in [(2,1),(3,17),(4,3),(8,32),(16,128)] {
        let parallel=scan_transactions_parallel_unverified(
            &block,cfg(workers,chunk_size)
        ).unwrap();
        assert_eq!(parallel.ordered(),serial.ordered(),"{workers} workers/{chunk_size} chunks");
        assert_eq!(parallel.statistics().transaction_count,block.len());
        assert_eq!(parallel.statistics().carrier_candidate_transactions,104);
        assert_eq!(parallel.statistics().workers_used,workers);
        assert_eq!(parallel.statistics().chunk_size,chunk_size);
        assert_eq!(parallel.statistics().total_raw_bytes,serial.statistics().total_raw_bytes);
        assert_eq!(parallel.trade_layer_candidates().map(|row| row.tx_index()).collect::<Vec<_>>(),
                   candidate_positions);
        for row in parallel.ordered().iter().step_by(53) {
            assert_eq!(id_hex(&row.txid_raw()).len(),64);
        }
        assert_eq!(
            parallel.require_authenticated_chain_and_utxo(),
            Err(ParallelIndexError::HistoricalBlockAndUtxoUnverified)
        );
        assert_eq!(
            parallel.require_signed_channel_revision(),
            Err(ParallelIndexError::CanonicalSignedRevisionUnavailable)
        );
        assert_eq!(
            parallel.require_trusted_consensus_execution(),
            Err(ParallelIndexError::TrustedConsensusExecutionUnavailable)
        );
    }
}

#[test]
fn first_error_is_lowest_tx_index_not_first_worker_to_complete() {
    let mut block=synthetic_block(1_009);
    // Work-stealing may discover malformed TX 906 before malformed TX 78.
    // The returned failure must be the canonical lowest TX index.
    block[906]=vec![0x02];
    block[78]=vec![0x02,0x00];
    let expected=ParallelIndexError::BadTransaction {
        tx_index:78,source:ScanError::Truncated
    };
    for (workers, chunk_size) in [(1,7),(2,1),(4,32),(16,5)] {
        for _ in 0..2 {
            assert_eq!(scan_transactions_parallel_unverified(
                &block,cfg(workers,chunk_size)
            ),Err(expected.clone()));
        }
    }
    // A later duplicate cannot supersede an earlier malformed payload.
    block[789]=block[12].clone();
    assert_eq!(
        scan_transactions_parallel_unverified(&block,cfg(8,3)),
        Err(expected)
    );
}

#[test]
fn duplicate_txid_fails_for_one_and_multiple_workers() {
    let mut block=synthetic_block(513);
    block[499]=block[9].clone();
    let duplicate=ParallelIndexError::DuplicateTxid {
        first_index:9,duplicate_index:499
    };
    for workers in [1,2,4,16] {
        assert_eq!(scan_transactions_parallel_unverified(
            &block,cfg(workers,7)
        ),Err(duplicate.clone()));
    }
    // Legitimate data at different positions produces distinct hashes.
    block[499]=fake_tx(499,false,0);
    let okay=scan_transactions_parallel_unverified(&block,cfg(4,7)).unwrap();
    assert_eq!(okay.ordered().len(),513);
}

#[test]
fn strict_resource_bounds_fail_before_thread_spawning() {
    let good=synthetic_block(3);
    for bad in [cfg(0,16),cfg(MAX_PARALLEL_WORKERS+1,16),
                cfg(2,0),cfg(2,MAX_PARALLEL_CHUNK+1)] {
        assert_eq!(scan_transactions_parallel_unverified(&good,bad),
            Err(ParallelIndexError::InvalidWorkerConfig));
    }
    assert_eq!(scan_transactions_parallel_unverified(&[],cfg(1,1)),
        Err(ParallelIndexError::EmptyBlock));
    let too_many=vec![vec![0x01];MAX_TRANSACTIONS_PER_BLOCK+1];
    assert_eq!(scan_transactions_parallel_unverified(&too_many,cfg(4,8)),
        Err(ParallelIndexError::TooManyTransactions));
    let malformed=vec![vec![]];
    assert_eq!(scan_transactions_parallel_unverified(&malformed,cfg(2,1)),
        Err(ParallelIndexError::BadTransactionSize { tx_index:0 }));
    let oversize=vec![vec![0x01;MAX_RAW_TRANSACTION+1]];
    assert_eq!(scan_transactions_parallel_unverified(&oversize,cfg(2,1)),
        Err(ParallelIndexError::BadTransactionSize { tx_index:0 }));
    let count=MAX_PARALLEL_BLOCK_BYTES/MAX_RAW_TRANSACTION+1;
    let huge=vec![vec![0x01;MAX_RAW_TRANSACTION];count+1];
    assert_eq!(scan_transactions_parallel_unverified(&huge,cfg(4,16)),
        Err(ParallelIndexError::TotalRawBytesTooLarge));
}

#[test]
fn small_blocks_use_serial_auto_and_explicit_workers_are_capped_by_tx_count() {
    for len in [1,2,AUTO_PARALLEL_THRESHOLD-1,AUTO_PARALLEL_THRESHOLD] {
        let block=synthetic_block(len);
        let auto=scan_transactions_auto_unverified(&block).unwrap();
        assert_eq!(auto.ordered().len(),len);
        if len < AUTO_PARALLEL_THRESHOLD {
            assert_eq!(auto.statistics().workers_used,1);
        } else {
            assert!((1..=MAX_PARALLEL_WORKERS).contains(&auto.statistics().workers_used));
        }
    }
    assert_eq!(scan_transactions_parallel_unverified(
        &synthetic_block(2),cfg(16,1)
    ).unwrap().statistics().workers_used,2);
}

#[test]
fn ordered_causal_projection_is_not_commutative_even_after_parallel_hashing() {
    let block=synthetic_block(2);
    let rows=scan_transactions_parallel_unverified(&block,cfg(8,1)).unwrap();
    let mut balance=0i64;
    for entry in rows.ordered() {
        let delta=match entry.tx_index() {
            0=>5,
            1=>-5,
            _=>unreachable!(),
        };
        balance+=delta;
        assert!(balance>=0,"ordered 0=>credit then 1=>debit");
    }
    assert_eq!(balance,0);
    // If consensus mutations were applied in arbitrary completion order,
    // the debit could execute before credit and violate collateral floor.
    let mut reversed_balance=0i64;
    reversed_balance-=5;
    assert!(reversed_balance<0);
    // No public method here converts a thread-completion order into a
    // trusted chain adapter or authorizes settlement.
}

#[test]
fn existing_candidate_block_shape_can_be_prescanned_without_trusting_header() {
    let txs=synthetic_block(256);
    let candidate=CandidateBlock {
        chain:ChainId([0x11;32]),
        header:BlockHeader {
            height:101,id:BlockId([0xa1;32]),parent:BlockId([0xa0;32]),
        },
        raw_transactions:txs.clone(),
    };
    let output=scan_candidate_block_parallel_unverified(&candidate,cfg(4,16)).unwrap();
    let direct=scan_transactions_parallel_unverified(&txs,cfg(1,16)).unwrap();
    assert_eq!(output.ordered(),direct.ordered());
    assert_eq!(output.statistics().workers_used,4);
    assert_eq!(output.require_authenticated_chain_and_utxo(),
        Err(ParallelIndexError::HistoricalBlockAndUtxoUnverified));
    // The claimed chain header and fake parent are NOT accepted consensus.
}
