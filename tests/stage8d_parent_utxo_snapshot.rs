//! Stage8D4: fail-closed external UTXO witness coverage and wave barriers.
//! All parent entries, height, scripts, hashes and fingerprint are UNVERIFIED.
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use tradelayer_core::parallel_txindex::ParallelIndexConfig;
use tradelayer_core::parent_utxo_snapshot::{
    candidate_snapshot_fingerprint, inspect_parent_candidate_snapshot,
    inspect_waves_with_candidate_snapshot, CandidateOutpoint, CandidateParentSnapshot,
    CandidateParentUtxo, CandidateSnapshotTrustError, ParentSnapshotError, SnapshotWaveError,
    BITCOIN_MAX_MONEY_SAT, CANDIDATE_PARENT_UTXO_STATUS, MAX_CANDIDATE_SNAPSHOT_ENTRIES,
    MAX_SNAPSHOT_SCRIPT_BYTES,
};
use tradelayer_core::raw_block_ingest::{
    merkle_root_unverified, DependencyAuditError, UnverifiedRawBlockView,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};
use tradelayer_core::wave_precheck::{UnverifiedProbeVerdict, WaveProbeError};

fn config(workers: usize) -> ParallelIndexConfig {
    ParallelIndexConfig {
        workers,
        window_transactions: 256,
    }
}

fn fixture() -> Vec<u8> {
    decode_hex(include_str!("../fixtures/raw_block_ingest_synthetic.hex").trim()).unwrap()
}

fn transactions() -> Vec<Vec<u8>> {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    (0..view.transaction_count())
        .map(|i| view.raw_transaction(i).unwrap().to_vec())
        .collect()
}

fn block(transactions: &[Vec<u8>]) -> Vec<u8> {
    assert!(!transactions.is_empty() && transactions.len() < 253);
    let mut raw = fixture()[..80].to_vec();
    let ids = transactions
        .iter()
        .map(|tx| inspect_transaction(tx).unwrap().txid)
        .collect::<Vec<_>>();
    raw[36..68].copy_from_slice(&merkle_root_unverified(&ids).unwrap());
    raw.push(transactions.len() as u8);
    for tx in transactions {
        raw.extend_from_slice(tx);
    }
    raw
}

fn spend(raw: &mut [u8], txid_raw: [u8; 32], vout: u32) {
    let input = if raw[4] == 0 { 7 } else { 5 };
    raw[input..input + 32].copy_from_slice(&txid_raw);
    raw[input + 32..input + 36].copy_from_slice(&vout.to_le_bytes());
}

fn chained() -> Vec<u8> {
    let mut tx = transactions();
    let producer = inspect_transaction(&tx[1]).unwrap().txid;
    spend(&mut tx[2], producer, 0);
    let intermediate = inspect_transaction(&tx[2]).unwrap().txid;
    spend(&mut tx[3], intermediate, 0);
    block(&tx)
}

fn snapshot_for(raw: &[u8], height: u32) -> CandidateParentSnapshot {
    let view = UnverifiedRawBlockView::parse(raw, None).unwrap();
    let generated = (0..view.transaction_count())
        .map(|index| {
            inspect_transaction(view.raw_transaction(index).unwrap())
                .unwrap()
                .txid
        })
        .collect::<BTreeSet<_>>();
    let mut entries = Vec::new();
    for index in 1..view.transaction_count() {
        let bytes = view.raw_transaction(index).unwrap();
        let offset = if bytes[4] == 0 { 7 } else { 5 };
        let mut txid_raw = [0u8; 32];
        txid_raw.copy_from_slice(&bytes[offset..offset + 32]);
        if generated.contains(&txid_raw) {
            continue;
        }
        let vout = u32::from_le_bytes(bytes[offset + 32..offset + 36].try_into().unwrap());
        entries.push(CandidateParentUtxo {
            outpoint: CandidateOutpoint { txid_raw, vout },
            amount_sat: 50_000 + index as u64,
            script_pubkey: vec![0x51, index as u8],
            created_height: height - 200,
            is_coinbase: false,
        });
    }
    entries.sort_by_key(|entry| entry.outpoint);
    let mut snapshot = CandidateParentSnapshot {
        parent_block_hash_raw: raw[4..36].try_into().unwrap(),
        parent_height: height,
        entries,
        expected_fingerprint: [0u8; 32],
    };
    resign(&mut snapshot);
    snapshot
}

fn resign(snapshot: &mut CandidateParentSnapshot) {
    snapshot.expected_fingerprint = candidate_snapshot_fingerprint(snapshot).unwrap();
}

#[test]
fn exact_three_external_prevouts_and_all_worker_configs() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let preindex = view.inspect_serial().unwrap();
    let snapshot = snapshot_for(&raw, 900_000);
    assert_eq!(snapshot.entries.len(), 3);
    for workers in [1, 2, 3, 4, 8, 16] {
        let audited = inspect_waves_with_candidate_snapshot(
            &view,
            &preindex,
            config(workers),
            &snapshot,
            |_, _| UnverifiedProbeVerdict::NoLocalRejection,
        )
        .unwrap();
        assert_eq!(
            audited.parent_witness.status(),
            CANDIDATE_PARENT_UTXO_STATUS
        );
        assert_eq!(audited.parent_witness.external_inputs_matched(), 3);
        assert_eq!(audited.parent_witness.same_block_outputs_deferred(), 0);
        assert_eq!(
            audited.parent_witness.candidate_fingerprint(),
            snapshot.expected_fingerprint
        );
        assert_eq!(audited.probe.positions_observed(), 4);
    }
}

#[test]
fn parent_child_grandchild_spends_are_deferred_from_parent_snapshot() {
    let raw = chained();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let preindex = view.inspect_parallel(config(4)).unwrap();
    let snapshot = snapshot_for(&raw, 900_000);
    assert_eq!(snapshot.entries.len(), 1);
    let audit =
        inspect_waves_with_candidate_snapshot(&view, &preindex, config(4), &snapshot, |_, _| {
            UnverifiedProbeVerdict::NoLocalRejection
        })
        .unwrap();
    assert_eq!(audit.parent_witness.external_inputs_matched(), 1);
    assert_eq!(audit.parent_witness.same_block_outputs_deferred(), 2);
    assert_eq!(audit.probe.wave_count(), 3);
    assert_eq!(
        audit.parent_witness.require_same_block_outputs_validated(),
        Err(CandidateSnapshotTrustError::SameBlockOutputsNotValidated)
    );
}

#[test]
fn missing_external_witness_denies_precheck_before_any_callback() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let preindex = view.inspect_serial().unwrap();
    let mut snapshot = snapshot_for(&raw, 900_000);
    snapshot.entries.remove(1);
    resign(&mut snapshot);
    let calls = AtomicUsize::new(0);
    let failure =
        inspect_waves_with_candidate_snapshot(&view, &preindex, config(8), &snapshot, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            UnverifiedProbeVerdict::NoLocalRejection
        });
    assert!(matches!(
        failure,
        Err(SnapshotWaveError::Snapshot(
            ParentSnapshotError::MissingExternalPrevout { .. }
        ))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn extra_snapshot_entry_is_denied() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let mut snapshot = snapshot_for(&raw, 900_000);
    let mut extra = snapshot.entries[0].clone();
    extra.outpoint.txid_raw = [0x92; 32];
    snapshot.entries.push(extra);
    snapshot.entries.sort_by_key(|entry| entry.outpoint);
    resign(&mut snapshot);
    assert!(matches!(
        inspect_parent_candidate_snapshot(&view, &indexed, &snapshot),
        Err(ParentSnapshotError::UnusedCandidateOutpoint { .. })
    ));
}

#[test]
fn duplicate_and_unsorted_snapshot_entries_fail_closed() {
    let raw = fixture();
    let mut snapshot = snapshot_for(&raw, 900_000);
    snapshot.entries.insert(1, snapshot.entries[0].clone());
    assert_eq!(
        candidate_snapshot_fingerprint(&snapshot),
        Err(ParentSnapshotError::DuplicateCandidateOutpoint { index: 1 })
    );
    let mut snapshot = snapshot_for(&raw, 900_000);
    snapshot.entries.swap(0, 1);
    assert_eq!(
        candidate_snapshot_fingerprint(&snapshot),
        Err(ParentSnapshotError::UnsortedCandidateOutpoints { index: 1 })
    );
}

#[test]
fn fingerprint_detects_script_amount_height_and_coinbase_flag_changes() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    for kind in 0..4 {
        let mut snapshot = snapshot_for(&raw, 900_000);
        match kind {
            0 => snapshot.entries[0].script_pubkey.push(0x52),
            1 => snapshot.entries[0].amount_sat += 1,
            2 => snapshot.entries[0].created_height += 1,
            3 => snapshot.entries[0].is_coinbase = true,
            _ => unreachable!(),
        }
        assert_eq!(
            inspect_parent_candidate_snapshot(&view, &indexed, &snapshot),
            Err(ParentSnapshotError::CandidateFingerprintMismatch)
        );
    }
}

#[test]
fn coupled_forgery_can_recompute_digest_but_never_authenticate_state() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let mut snapshot = snapshot_for(&raw, 900_000);
    snapshot.entries[0].script_pubkey = vec![0x6a, 0x01, 0x99];
    snapshot.entries[0].amount_sat = 42;
    resign(&mut snapshot);
    let audit = inspect_parent_candidate_snapshot(&view, &indexed, &snapshot).unwrap();
    assert_eq!(audit.external_inputs_matched(), 3);
    assert_eq!(
        audit.require_authenticated_parent_state(),
        Err(CandidateSnapshotTrustError::ParentStateRootNotAuthenticated)
    );
    assert_eq!(
        audit.require_verified_bitcoin_scripts(),
        Err(CandidateSnapshotTrustError::BitcoinScriptsAndFeesNotVerified)
    );
    assert_eq!(
        audit.require_tradelayer_or_bitcoin_apply(),
        Err(CandidateSnapshotTrustError::EconomicStateTransitionUnavailable)
    );
}

#[test]
fn claimed_parent_hash_mismatch_rejected_even_with_recomputed_fingerprint() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let mut snapshot = snapshot_for(&raw, 900_000);
    snapshot.parent_block_hash_raw[0] ^= 1;
    resign(&mut snapshot);
    assert_eq!(
        inspect_parent_candidate_snapshot(&view, &indexed, &snapshot),
        Err(ParentSnapshotError::ParentBlockMismatch)
    );
    // Wrong fork: a different parent header reference even if all raw TXs
    // and their merkle root remain identical.
    let mut other = raw.clone();
    other[4] ^= 0x01;
    let fork_view = UnverifiedRawBlockView::parse(&other, None).unwrap();
    let fork_index = fork_view.inspect_serial().unwrap();
    let correct_for_original = snapshot_for(&raw, 900_000);
    assert_eq!(
        inspect_parent_candidate_snapshot(&fork_view, &fork_index, &correct_for_original),
        Err(ParentSnapshotError::ParentBlockMismatch)
    );
}

#[test]
fn immature_coinbase_claimed_prevout_rejected() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let mut snapshot = snapshot_for(&raw, 900_000);
    snapshot.entries[0].is_coinbase = true;
    snapshot.entries[0].created_height = 899_999;
    resign(&mut snapshot);
    assert!(matches!(
        inspect_parent_candidate_snapshot(&view, &indexed, &snapshot),
        Err(ParentSnapshotError::ImmatureClaimedCoinbase { .. })
    ));
    snapshot.entries[0].created_height = 899_900;
    resign(&mut snapshot);
    assert!(inspect_parent_candidate_snapshot(&view, &indexed, &snapshot).is_ok());
}

#[test]
fn bounds_reject_oversized_script_invalid_value_and_future_creation() {
    let raw = fixture();
    let original = snapshot_for(&raw, 900_000);
    let mut invalid = original.clone();
    invalid.entries[0].script_pubkey = vec![0; MAX_SNAPSHOT_SCRIPT_BYTES + 1];
    assert_eq!(
        candidate_snapshot_fingerprint(&invalid),
        Err(ParentSnapshotError::OversizedCandidateScript { index: 0 })
    );
    invalid = original.clone();
    invalid.entries[0].amount_sat = BITCOIN_MAX_MONEY_SAT + 1;
    assert_eq!(
        candidate_snapshot_fingerprint(&invalid),
        Err(ParentSnapshotError::InvalidCandidateValue { index: 0 })
    );
    invalid = original.clone();
    invalid.entries[0].created_height = original.parent_height + 1;
    assert_eq!(
        candidate_snapshot_fingerprint(&invalid),
        Err(ParentSnapshotError::FutureCandidateOutput { index: 0 })
    );
    invalid = original;
    invalid.entries.resize(
        MAX_CANDIDATE_SNAPSHOT_ENTRIES + 1,
        invalid.entries[0].clone(),
    );
    assert_eq!(
        candidate_snapshot_fingerprint(&invalid),
        Err(ParentSnapshotError::CandidateSnapshotTooLarge)
    );
}

#[test]
fn parent_height_overflow_rejected_before_snapshot_lookup() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let mut snapshot = snapshot_for(&raw, 900_000);
    snapshot.parent_height = u32::MAX;
    resign(&mut snapshot);
    assert_eq!(
        inspect_parent_candidate_snapshot(&view, &indexed, &snapshot),
        Err(ParentSnapshotError::ParentHeightOverflow)
    );
}

#[test]
fn complete_snapshot_preflight_denies_double_spends_before_callback() {
    let mut tx = transactions();
    let parent = inspect_transaction(&tx[1]).unwrap().txid;
    spend(&mut tx[2], parent, 0);
    spend(&mut tx[3], parent, 0);
    let raw = block(&tx);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let snapshot = snapshot_for(&raw, 900_000);
    let calls = AtomicUsize::new(0);
    assert_eq!(
        inspect_waves_with_candidate_snapshot(&view, &indexed, config(4), &snapshot, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            UnverifiedProbeVerdict::NoLocalRejection
        }),
        Err(SnapshotWaveError::Snapshot(
            ParentSnapshotError::Dependencies(DependencyAuditError::DuplicateInputSpend {
                first: 2,
                second: 3
            })
        ))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn valid_snapshot_still_denies_wave_probe_failure() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let snapshot = snapshot_for(&raw, 900_000);
    assert_eq!(
        inspect_waves_with_candidate_snapshot(&view, &indexed, config(4), &snapshot, |index, _| {
            if index == 2 {
                UnverifiedProbeVerdict::Reject
            } else {
                UnverifiedProbeVerdict::NoLocalRejection
            }
        }),
        Err(SnapshotWaveError::Wave(WaveProbeError::ProbeRejected {
            wave: 0,
            index: 2
        }))
    );
}

#[test]
fn pinned_independent_python_candidate_fingerprint_vector() {
    // Separately encoded in scripts/audit_stage8d4_candidate_fingerprint.py.
    // This tests ordering/endianness, never authenticated Bitcoin UTXOs.
    let mut snapshot = CandidateParentSnapshot {
        parent_block_hash_raw: decode_hex(&"0123456789abcdef".repeat(4))
            .unwrap()
            .try_into()
            .unwrap(),
        parent_height: 903_000,
        entries: vec![
            CandidateParentUtxo {
                outpoint: CandidateOutpoint {
                    txid_raw: [0x11; 32],
                    vout: 2,
                },
                amount_sat: 12_345_678,
                script_pubkey: decode_hex(&format!("0014{}", "aa".repeat(20))).unwrap(),
                created_height: 902_500,
                is_coinbase: false,
            },
            CandidateParentUtxo {
                outpoint: CandidateOutpoint {
                    txid_raw: [0x22; 32],
                    vout: 0,
                },
                amount_sat: 5_000_000_000,
                script_pubkey: vec![0x51],
                created_height: 902_000,
                is_coinbase: true,
            },
        ],
        expected_fingerprint: [0u8; 32],
    };
    let expected: [u8; 32] =
        decode_hex("6b8b85dde893bbc5adf6c3e50e3cae6d11984038d25bc3d329e6dc76ea4fe198")
            .unwrap()
            .try_into()
            .unwrap();
    assert_eq!(candidate_snapshot_fingerprint(&snapshot).unwrap(), expected);
    snapshot.expected_fingerprint = expected;
    snapshot.entries[0].amount_sat += 1;
    assert_ne!(candidate_snapshot_fingerprint(&snapshot).unwrap(), expected);
}

#[test]
fn exact_same_header_cannot_reuse_preindex_for_changed_witness_bytes() {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let mut alternative = None;
    'search: for index in 1..view.transaction_count() {
        let original = view.raw_transaction(index).unwrap();
        if !inspect_transaction(original).unwrap().has_witness {
            continue;
        }
        let span = &view.transaction_ranges()[index];
        for relative in 6..original.len().saturating_sub(4) {
            let mut changed = raw.clone();
            changed[span.start + relative] ^= 1;
            if let Ok(changed_view) = UnverifiedRawBlockView::parse(&changed, None) {
                if let Ok(fresh) = changed_view.inspect_serial() {
                    if fresh.txid_merkle_root_raw() == indexed.txid_merkle_root_raw()
                        && fresh.ordered_preindex() != indexed.ordered_preindex()
                    {
                        alternative = Some(changed);
                        break 'search;
                    }
                }
            }
        }
    }
    let changed = alternative.expect("fixture must expose a mutable witness byte");
    let altered_view = UnverifiedRawBlockView::parse(&changed, None).unwrap();
    let snapshot = snapshot_for(&raw, 900_000);
    assert!(matches!(
        inspect_parent_candidate_snapshot(&altered_view, &indexed, &snapshot),
        Err(ParentSnapshotError::StalePreindex { .. })
    ));
}
