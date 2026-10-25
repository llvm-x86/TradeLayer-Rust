//! Runtime indexing concurrency: genuine CPU scans, bounded mock-RPC
//! fetching, deterministic transaction order and hard no-consensus gates.
//! No real network, wallet, RPC node, signed funds or historical UTXOs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;
use tradelayer_core::parallel_txindex::{
    fetch_and_inspect_unverified_parallel, fetch_and_inspect_unverified_serial,
    inspect_unverified_block_parallel, inspect_unverified_block_serial, ParallelIndexConfig,
    ParallelIndexError, ParallelIndexTrustError, MAX_BLOCK_INPUT_BYTES, MAX_INDEX_WINDOW,
    MAX_PARALLEL_WORKERS,
};
use tradelayer_core::read_only::{inspect_transaction, ScanError};

fn compact_into(dst: &mut Vec<u8>, n: usize) {
    if n < 253 {
        dst.push(n as u8);
    } else if n <= u16::MAX as usize {
        dst.push(253);
        dst.extend_from_slice(&(n as u16).to_le_bytes());
    } else {
        dst.push(254);
        dst.extend_from_slice(&(n as u32).to_le_bytes());
    }
}

/// Synthetic structurally parseable Bitcoin tx. Not a valid spend/script
/// or real block. Large per-input scriptSig payloads exercise SHA256d CPU.
fn synthetic_tx(index: usize, candidate: bool, script_sig_bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(script_sig_bytes + 128);
    out.extend_from_slice(&2u32.to_le_bytes());
    out.push(1); // exactly one input
    let mut prevout = [0u8; 32];
    prevout[..8].copy_from_slice(&(index as u64).to_le_bytes());
    prevout[8] = 0x42;
    out.extend_from_slice(&prevout);
    out.extend_from_slice(&0u32.to_le_bytes());
    compact_into(&mut out, script_sig_bytes);
    for i in 0..script_sig_bytes {
        out.push((index as u8).wrapping_add((i as u8).wrapping_mul(29)));
    }
    out.extend_from_slice(&0xffff_fffeu32.to_le_bytes());
    out.push(1); // one output
    out.extend_from_slice(&0u64.to_le_bytes());
    if candidate {
        // OP_RETURN with exactly one canonical 14-byte ASCII TL push.
        let script = b"\x6a\x0etl20;alice;5;2";
        compact_into(&mut out, script.len());
        out.extend_from_slice(script);
    } else {
        // Intentionally non-OP_RETURN script; no script execution takes
        // place in this read-only scan, just serialization inspection.
        let script = b"\x51\x21\x02\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        compact_into(&mut out, script.len());
        out.extend_from_slice(script);
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

fn synthetic_block(n: usize, pad: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| synthetic_tx(i, i % 79 == 0 || i % 157 == 1, pad + (i % 7) * 17))
        .collect()
}

fn config(workers: usize, window: usize) -> ParallelIndexConfig {
    ParallelIndexConfig {
        workers,
        window_transactions: window,
    }
}

#[test]
fn cpu_parallelism_matches_serial_exactly_even_across_small_windows() {
    let rows = synthetic_block(1047, 386);
    let expected = inspect_unverified_block_serial(&rows).unwrap();
    assert_eq!(expected.transaction_count(), rows.len());
    assert!(expected.candidate_positions().len() >= 10);
    assert!(expected.op_return_output_count() >= 10);
    assert!(expected
        .candidate_positions()
        .windows(2)
        .all(|p| p[0] < p[1]));
    let bytes: usize = rows.iter().map(Vec::len).sum();
    assert_eq!(expected.raw_bytes_inspected(), bytes);
    for workers in [1, 2, 3, 4, 8, 16] {
        for window in [1, 7, 64, 256, MAX_INDEX_WINDOW] {
            let report = inspect_unverified_block_parallel(&rows, config(workers, window)).unwrap();
            assert_eq!(report, expected, "workers {workers}, window {window}");
            for (position, row) in report.ordered().iter().enumerate() {
                assert_eq!(row.position, position);
                assert_eq!(
                    row.observation.txid,
                    inspect_transaction(&rows[position]).unwrap().txid
                );
            }
            assert_eq!(
                report
                    .candidates_in_source_order()
                    .map(|c| c.position)
                    .collect::<Vec<_>>(),
                report.candidate_positions()
            );
        }
    }
    assert_eq!(
        expected.require_authenticated_block(),
        Err(ParallelIndexTrustError::FullBlockAndChainNotVerified)
    );
    assert_eq!(
        expected.require_authenticated_transaction_state(),
        Err(ParallelIndexTrustError::ScriptAndStateNotVerified)
    );
    assert_eq!(
        expected.require_canonical_signed_channel_revision(),
        Err(ParallelIndexTrustError::SignedStateRevisionUnavailable)
    );
}

#[test]
fn first_block_order_failure_does_not_depend_on_worker_schedule() {
    let mut rows = synthetic_block(273, 32);
    rows[19].truncate(4);
    rows[150].truncate(3);
    let expected = ParallelIndexError::RawScan {
        index: 19,
        source: ScanError::Truncated,
    };
    assert_eq!(
        inspect_unverified_block_serial(&rows),
        Err(expected.clone())
    );
    for workers in [1, 2, 4, 8] {
        for window in [1, 32, 64, 256] {
            assert_eq!(
                inspect_unverified_block_parallel(&rows, config(workers, window)),
                Err(expected.clone())
            );
        }
    }
}

#[test]
fn checked_txids_stay_positional_under_out_of_order_mock_rpc_completion() {
    let rows = synthetic_block(513, 128);
    let txids: Vec<_> = rows
        .iter()
        .map(|raw| inspect_transaction(raw).unwrap().txid)
        .collect();
    let serial =
        fetch_and_inspect_unverified_serial(&txids, |index, _| Ok(rows[index].clone())).unwrap();
    let in_flight = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let parallel = fetch_and_inspect_unverified_parallel(&txids, config(6, 17), |index, _| {
        let active = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        peak.fetch_max(active, Ordering::SeqCst);
        if index % 7 == 0 {
            thread::sleep(Duration::from_millis(2));
        }
        let raw = rows[index].clone();
        in_flight.fetch_sub(1, Ordering::SeqCst);
        Ok(raw)
    })
    .unwrap();
    assert_eq!(parallel, serial);
    assert_eq!(in_flight.load(Ordering::SeqCst), 0);
    assert!(peak.load(Ordering::SeqCst) <= 6);
    assert_eq!(parallel.transaction_count(), 513);
    assert_eq!(parallel.candidate_positions(), serial.candidate_positions());
}

#[test]
fn rpc_failure_reports_lowest_position_even_if_worker_finishes_last() {
    let rows = synthetic_block(96, 128);
    let ids: Vec<_> = rows
        .iter()
        .map(|raw| inspect_transaction(raw).unwrap().txid)
        .collect();
    let result = fetch_and_inspect_unverified_parallel(&ids, config(8, 64), |index, _| {
        if index == 3 {
            thread::sleep(Duration::from_millis(8));
            return Err("slow first failure".into());
        }
        if index == 11 {
            return Err("fast second failure".into());
        }
        Ok(rows[index].clone())
    });
    assert_eq!(
        result,
        Err(ParallelIndexError::Fetch {
            index: 3,
            message: "slow first failure".into(),
        })
    );
}

#[test]
fn fake_rpc_returns_wrong_raw_bytes_and_never_gets_a_claimed_txid_pass() {
    let rows = synthetic_block(90, 90);
    let ids: Vec<_> = rows
        .iter()
        .map(|raw| inspect_transaction(raw).unwrap().txid)
        .collect();
    let result = fetch_and_inspect_unverified_parallel(&ids, config(5, 25), |index, _| {
        if index == 37 {
            Ok(rows[38].clone())
        } else {
            Ok(rows[index].clone())
        }
    });
    assert_eq!(
        result,
        Err(ParallelIndexError::ReturnedTxidMismatch { index: 37 })
    );
}

#[test]
fn duplicate_txids_in_the_same_block_fail_closed_in_both_modes() {
    let mut rows = synthetic_block(256, 64);
    rows[199] = rows[19].clone();
    let duplicate = ParallelIndexError::DuplicateTxid {
        first: 19,
        second: 199,
    };
    assert_eq!(
        inspect_unverified_block_serial(&rows),
        Err(duplicate.clone())
    );
    assert_eq!(
        inspect_unverified_block_parallel(&rows, config(8, 128)),
        Err(duplicate.clone())
    );
    let ids: Vec<_> = rows
        .iter()
        .map(|raw| inspect_transaction(raw).unwrap().txid)
        .collect();
    assert_eq!(
        fetch_and_inspect_unverified_parallel(&ids, config(8, 64), |index, _| {
            Ok(rows[index].clone())
        }),
        Err(duplicate)
    );
}

#[test]
fn configuration_and_raw_byte_limits_fail_before_any_ledger_effect() {
    let rows = synthetic_block(3, 64);
    assert_eq!(
        inspect_unverified_block_parallel(&rows, config(0, 2)),
        Err(ParallelIndexError::InvalidConfig)
    );
    assert_eq!(
        inspect_unverified_block_parallel(&rows, config(MAX_PARALLEL_WORKERS + 1, 2)),
        Err(ParallelIndexError::InvalidConfig)
    );
    assert_eq!(
        inspect_unverified_block_parallel(&rows, config(2, 0)),
        Err(ParallelIndexError::InvalidConfig)
    );
    assert_eq!(
        inspect_unverified_block_parallel(&rows, config(2, MAX_INDEX_WINDOW + 1)),
        Err(ParallelIndexError::InvalidConfig)
    );
    assert_eq!(
        inspect_unverified_block_serial(&[]),
        Err(ParallelIndexError::EmptyBlock)
    );
    let mut empty = rows.clone();
    empty[1].clear();
    assert_eq!(
        inspect_unverified_block_parallel(&empty, config(4, 2)),
        Err(ParallelIndexError::EmptyRawTransaction { index: 1 })
    );
    let mut oversized = rows.clone();
    oversized[0] = vec![0u8; tradelayer_core::read_only::MAX_RAW_TRANSACTION + 1];
    assert_eq!(
        inspect_unverified_block_parallel(&oversized, config(4, 2)),
        Err(ParallelIndexError::OversizedRawTransaction { index: 0 })
    );
    let large: Vec<_> = (0..34).map(|i| synthetic_tx(i, false, 970_000)).collect();
    assert!(large.iter().map(Vec::len).sum::<usize>() > MAX_BLOCK_INPUT_BYTES);
    assert_eq!(
        inspect_unverified_block_parallel(&large, config(4, 128)),
        Err(ParallelIndexError::ExceededBlockByteLimit)
    );
}

#[test]
fn reorg_reinspection_has_no_cross_fork_memoized_candidates_or_source_state() {
    let common = synthetic_block(101, 144);
    let old = inspect_unverified_block_parallel(&common, config(4, 32)).unwrap();
    let mut fork = common.clone();
    fork[9] = synthetic_tx(999_009, true, 144);
    let current = inspect_unverified_block_parallel(&fork, config(4, 32)).unwrap();
    assert_ne!(
        old.ordered()[9].observation.txid,
        current.ordered()[9].observation.txid
    );
    assert_eq!(
        current.ordered()[9].observation.txid,
        inspect_transaction(&fork[9]).unwrap().txid
    );
    assert!(current.candidate_positions().contains(&9));
    assert!(!old.candidate_positions().contains(&9));
    let replay = inspect_unverified_block_parallel(&common, config(8, 17)).unwrap();
    assert_eq!(old, replay);
    assert_eq!(
        current.require_authenticated_block(),
        Err(ParallelIndexTrustError::FullBlockAndChainNotVerified)
    );
}
