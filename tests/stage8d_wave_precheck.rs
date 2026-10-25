//! Stage 8D pure-probe scheduler: parallel dependency waves, deterministic
//! fail-closed barriers, source-order failure parity and NO UTXO authority.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tradelayer_core::parallel_txindex::ParallelIndexConfig;
use tradelayer_core::raw_block_ingest::{
    merkle_root_unverified, DependencyAuditError, UnverifiedRawBlockView,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};
use tradelayer_core::wave_precheck::{
    inspect_dependency_waves_with_probe, UnverifiedProbeVerdict,
    UnverifiedWaveTrustError, WaveProbeError, WAVE_OBSERVATION_STATUS,
};

fn config(workers: usize, window_transactions: usize) -> ParallelIndexConfig {
    ParallelIndexConfig {
        workers,
        window_transactions,
    }
}

fn fixture() -> Vec<u8> {
    decode_hex(include_str!("../fixtures/raw_block_ingest_synthetic.hex").trim()).unwrap()
}

fn transaction_bytes() -> Vec<Vec<u8>> {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    (0..view.transaction_count())
        .map(|i| view.raw_transaction(i).unwrap().to_vec())
        .collect()
}

fn block(transactions: &[Vec<u8>]) -> Vec<u8> {
    assert!(!transactions.is_empty() && transactions.len() < 253);
    let mut result = fixture()[..80].to_vec();
    let ids = transactions
        .iter()
        .map(|tx| inspect_transaction(tx).unwrap().txid)
        .collect::<Vec<_>>();
    let merkle = merkle_root_unverified(&ids).unwrap();
    result[36..68].copy_from_slice(&merkle);
    result.push(transactions.len() as u8);
    for transaction in transactions {
        result.extend_from_slice(transaction);
    }
    result
}

fn spend(transaction: &mut [u8], parent: [u8; 32], vout: u32) {
    let offset = if transaction[4] == 0 { 7 } else { 5 };
    transaction[offset..offset + 32].copy_from_slice(&parent);
    transaction[offset + 32..offset + 36].copy_from_slice(&vout.to_le_bytes());
}

fn chained() -> Vec<u8> {
    let mut tx = transaction_bytes();
    let parent = inspect_transaction(&tx[1]).unwrap().txid;
    spend(&mut tx[2], parent, 0);
    let middle = inspect_transaction(&tx[2]).unwrap().txid;
    spend(&mut tx[3], middle, 0);
    block(&tx)
}

fn one_child_one_independent() -> Vec<u8> {
    let mut tx = transaction_bytes();
    let parent = inspect_transaction(&tx[1]).unwrap().txid;
    spend(&mut tx[2], parent, 0);
    block(&tx)
}

#[test]
fn clean_independent_wave_matches_serial_across_worker_window_matrix() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let wave_plan = view.plan_dependency_waves(&indexed).unwrap();
    assert_eq!(wave_plan.waves(), &[vec![0, 1, 2, 3]]);
    for workers in [1, 2, 3, 4, 8, 16] {
        for window in [1, 2, 4, 16, 512] {
            let observed = inspect_dependency_waves_with_probe(
                &view,
                &indexed,
                config(workers, window),
                |position, borrowed| {
                    assert_eq!(
                        borrowed.as_ptr(),
                        view.raw_transaction(position).unwrap().as_ptr()
                    );
                    UnverifiedProbeVerdict::NoLocalRejection
                },
            )
            .unwrap();
            assert_eq!(observed.status(), WAVE_OBSERVATION_STATUS);
            assert_eq!(observed.positions_observed(), 4);
            assert_eq!(observed.wave_count(), 1);
            assert_eq!(observed.unverified_external_prevouts(), 3);
        }
    }
}

#[test]
fn dependent_wave_barriers_do_not_interleave() {
    let raw = chained();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    assert_eq!(
        view.plan_dependency_waves(&indexed).unwrap().waves(),
        &[vec![0, 1], vec![2], vec![3]]
    );
    for workers in [1, 2, 4, 16] {
        let seen = AtomicUsize::new(0);
        let report = inspect_dependency_waves_with_probe(
            &view,
            &indexed,
            config(workers, 1),
            |index, _raw| {
                match index {
                    2 => assert_eq!(seen.load(Ordering::SeqCst), 2),
                    3 => assert_eq!(seen.load(Ordering::SeqCst), 3),
                    _ => {}
                }
                seen.fetch_add(1, Ordering::SeqCst);
                UnverifiedProbeVerdict::NoLocalRejection
            },
        )
        .unwrap();
        assert_eq!(report.wave_count(), 3);
        assert_eq!(report.positions_observed(), 4);
        assert_eq!(report.unverified_external_prevouts(), 1);
    }
}

#[test]
fn first_rejection_cancels_all_later_dependency_waves() {
    let raw = chained();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let index = view.inspect_parallel(config(4, 2)).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&seen);
    let result = inspect_dependency_waves_with_probe(&view, &index, config(8, 2), |i, _| {
        captured.lock().unwrap().push(i);
        if i == 2 {
            UnverifiedProbeVerdict::Reject
        } else {
            UnverifiedProbeVerdict::NoLocalRejection
        }
    });
    assert_eq!(
        result,
        Err(WaveProbeError::ProbeRejected { wave: 1, index: 2 })
    );
    let checked = seen.lock().unwrap().clone();
    assert!(checked.contains(&0) && checked.contains(&1) && checked.contains(&2));
    assert!(!checked.contains(&3));
}

#[test]
fn lower_source_rejection_wins_despite_parallel_worker_completion() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    for _ in 0..100 {
        let outcome = inspect_dependency_waves_with_probe(
            &view,
            &indexed,
            config(16, 512),
            |i, _| {
                if i == 1 {
                    for _ in 0..8 {
                        std::thread::yield_now();
                    }
                }
                if i == 1 || i == 3 {
                    UnverifiedProbeVerdict::Reject
                } else {
                    UnverifiedProbeVerdict::NoLocalRejection
                }
            },
        );
        assert_eq!(
            outcome,
            Err(WaveProbeError::ProbeRejected { wave: 0, index: 1 })
        );
    }
}

#[test]
fn wave_failure_precedes_lower_transaction_index_in_unvisited_wave() {
    let raw = one_child_one_independent();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    assert_eq!(
        view.plan_dependency_waves(&indexed).unwrap().waves(),
        &[vec![0, 1, 3], vec![2]]
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&seen);
    let result = inspect_dependency_waves_with_probe(
        &view,
        &indexed,
        config(3, 64),
        |index, _| {
            captured.lock().unwrap().push(index);
            if index == 2 || index == 3 {
                UnverifiedProbeVerdict::Reject
            } else {
                UnverifiedProbeVerdict::NoLocalRejection
            }
        },
    );
    // Barrier policy is (wave, source index), not smallest theoretical
    // error index in work that has deliberately not been authorized to run.
    assert_eq!(
        result,
        Err(WaveProbeError::ProbeRejected { wave: 0, index: 3 })
    );
    assert!(!seen.lock().unwrap().contains(&2));
}

#[test]
fn probe_panics_are_denied_instead_of_granting_partial_success() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    for workers in [1, 2, 16] {
        let failure = inspect_dependency_waves_with_probe(
            &view,
            &indexed,
            config(workers, 4),
            |index, _| {
                if index == 2 {
                    panic!("deliberate untrusted precheck panic");
                }
                UnverifiedProbeVerdict::NoLocalRejection
            },
        );
        assert_eq!(
            failure,
            Err(WaveProbeError::ProbePanicked { wave: 0, index: 2 })
        );
    }
}

#[test]
fn rejected_callback_precedes_later_panic_within_wave() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let failed = inspect_dependency_waves_with_probe(
        &view,
        &indexed,
        config(4, 1),
        |i, _| match i {
            1 => UnverifiedProbeVerdict::Reject,
            3 => panic!("later panicking probe"),
            _ => UnverifiedProbeVerdict::NoLocalRejection,
        },
    );
    assert_eq!(
        failed,
        Err(WaveProbeError::ProbeRejected { wave: 0, index: 1 })
    );
}

#[test]
fn different_header_or_merkle_preindex_cannot_be_reused() {
    let source = fixture();
    let source_view = UnverifiedRawBlockView::parse(&source, None).unwrap();
    let scanned = source_view.inspect_serial().unwrap();
    let mut other = source.clone();
    other[0] ^= 0x01;
    let other_view = UnverifiedRawBlockView::parse(&other, None).unwrap();
    assert_eq!(
        inspect_dependency_waves_with_probe(
            &other_view,
            &scanned,
            config(2, 64),
            |_, _| UnverifiedProbeVerdict::NoLocalRejection
        ),
        Err(WaveProbeError::Dependencies(
            DependencyAuditError::MismatchedPreindex
        ))
    );
}

#[test]
fn invalid_configs_never_invoke_precheck() {
    let source = fixture();
    let view = UnverifiedRawBlockView::parse(&source, None).unwrap();
    let index = view.inspect_serial().unwrap();
    let calls = AtomicUsize::new(0);
    for bad in [config(0, 1), config(17, 1), config(1, 0), config(1, 513)] {
        let result = inspect_dependency_waves_with_probe(&view, &index, bad, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            UnverifiedProbeVerdict::NoLocalRejection
        });
        assert_eq!(result, Err(WaveProbeError::InvalidWorkerConfig));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn double_spend_is_denied_before_any_wave_precheck_is_called() {
    let mut tx = transaction_bytes();
    let parent = inspect_transaction(&tx[1]).unwrap().txid;
    spend(&mut tx[2], parent, 0);
    spend(&mut tx[3], parent, 0);
    let raw = block(&tx);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let calls = AtomicUsize::new(0);
    let result = inspect_dependency_waves_with_probe(
        &view,
        &indexed,
        config(8, 512),
        |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            UnverifiedProbeVerdict::NoLocalRejection
        },
    );
    assert_eq!(
        result,
        Err(WaveProbeError::Dependencies(
            DependencyAuditError::DuplicateInputSpend { first: 2, second: 3 }
        ))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn every_no_rejection_result_still_denies_consensus_and_tl_apply() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_parallel(config(4, 2)).unwrap();
    let result = inspect_dependency_waves_with_probe(
        &view,
        &scanned,
        config(4, 512),
        |_, _| UnverifiedProbeVerdict::NoLocalRejection,
    )
    .unwrap();
    assert_eq!(
        result.require_authenticated_parent_utxos(),
        Err(UnverifiedWaveTrustError::ParentUtxoSnapshotNotAuthenticated)
    );
    assert_eq!(
        result.require_valid_bitcoin_scripts(),
        Err(UnverifiedWaveTrustError::BitcoinScriptsNotVerified)
    );
    assert_eq!(
        result.require_consensus_or_economic_apply(),
        Err(UnverifiedWaveTrustError::CanonicalBitcoinAndTradeLayerApplyUnavailable)
    );
}
