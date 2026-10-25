//! Stage8D5 raw funding evidence: byte-derived value/script match without
//! pretending that a funding transaction belongs to the active Bitcoin chain.
use std::sync::atomic::{AtomicUsize, Ordering};

use tradelayer_core::parallel_txindex::ParallelIndexConfig;
use tradelayer_core::parent_funding_evidence::{
    inspect_raw_funding_evidence, inspect_waves_with_raw_funding_evidence, FundingEvidenceError,
    FundingEvidenceTrustError, FundingWaveError, RAW_FUNDING_EVIDENCE_STATUS,
};
use tradelayer_core::parent_utxo_snapshot::{
    candidate_snapshot_fingerprint, CandidateOutpoint, CandidateParentSnapshot,
    CandidateParentUtxo, ParentSnapshotError,
};
use tradelayer_core::raw_block_ingest::{merkle_root_unverified, UnverifiedRawBlockView};
use tradelayer_core::read_only::{
    decode_hex, inspect_transaction, select_unverified_transaction_output,
};
use tradelayer_core::wave_precheck::{UnverifiedProbeVerdict, WaveProbeError};

fn config(workers: usize) -> ParallelIndexConfig {
    ParallelIndexConfig {
        workers,
        window_transactions: 256,
    }
}

fn frozen() -> Vec<u8> {
    decode_hex(include_str!("../fixtures/raw_block_ingest_synthetic.hex").trim()).unwrap()
}

fn base_transactions() -> Vec<Vec<u8>> {
    let raw = frozen();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    (0..view.transaction_count())
        .map(|i| view.raw_transaction(i).unwrap().to_vec())
        .collect()
}

fn block(txs: &[Vec<u8>]) -> Vec<u8> {
    assert!(!txs.is_empty() && txs.len() < 253);
    let mut raw = frozen()[..80].to_vec();
    let hashes = txs
        .iter()
        .map(|tx| inspect_transaction(tx).unwrap().txid)
        .collect::<Vec<_>>();
    raw[36..68].copy_from_slice(&merkle_root_unverified(&hashes).unwrap());
    raw.push(txs.len() as u8);
    for tx in txs {
        raw.extend_from_slice(tx);
    }
    raw
}

fn rewire(tx: &mut [u8], prev: [u8; 32], n: u32) {
    let at = if tx[4] == 0 { 7 } else { 5 };
    tx[at..at + 32].copy_from_slice(&prev);
    tx[at + 32..at + 36].copy_from_slice(&n.to_le_bytes());
}

fn funding(seed: u8) -> (Vec<u8>, Vec<(u64, Vec<u8>)>) {
    let outputs: Vec<(u64, Vec<u8>)> = vec![
        (13_000, vec![0x51]),
        (42_000, [&[0x00, 0x14][..], &[seed; 20]].concat()),
        (90_000, vec![0x6a, 0x01, seed]),
    ];
    let mut raw = vec![2, 0, 0, 0, 1];
    raw.extend_from_slice(&[seed; 32]);
    raw.extend_from_slice(&0u32.to_le_bytes());
    raw.extend_from_slice(&[1, 0x51]);
    raw.extend_from_slice(&u32::MAX.to_le_bytes());
    raw.push(outputs.len() as u8);
    for (amount, script) in &outputs {
        raw.extend_from_slice(&amount.to_le_bytes());
        raw.push(script.len() as u8);
        raw.extend_from_slice(script);
    }
    raw.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(inspect_transaction(&raw).unwrap().output_count, 3);
    (raw, outputs)
}

fn sign(snapshot: &mut CandidateParentSnapshot) {
    snapshot.expected_fingerprint = candidate_snapshot_fingerprint(snapshot).unwrap();
}

fn assemble(seed: u8) -> (Vec<u8>, CandidateParentSnapshot, Vec<Vec<u8>>) {
    let (raw_funding, outputs) = funding(seed);
    let txid = inspect_transaction(&raw_funding).unwrap().txid;
    let mut txs = base_transactions();
    for (index, tx) in txs.iter_mut().enumerate().take(4).skip(1) {
        rewire(tx, txid, (index - 1) as u32);
    }
    let raw = block(&txs);
    let mut snapshot = CandidateParentSnapshot {
        parent_block_hash_raw: raw[4..36].try_into().unwrap(),
        parent_height: 900_000,
        entries: outputs
            .iter()
            .enumerate()
            .map(|(index, (amount, script))| CandidateParentUtxo {
                outpoint: CandidateOutpoint {
                    txid_raw: txid,
                    vout: index as u32,
                },
                amount_sat: *amount,
                script_pubkey: script.clone(),
                created_height: 899_500,
                is_coinbase: false,
            })
            .collect(),
        expected_fingerprint: [0; 32],
    };
    sign(&mut snapshot);
    (raw, snapshot, vec![raw_funding])
}

#[test]
fn three_prevouts_use_one_exact_raw_funding_transaction() {
    let (raw, snapshot, evidence) = assemble(0x53);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let index = view.inspect_parallel(config(4)).unwrap();
    for workers in [1, 2, 3, 4, 8, 16] {
        let report = inspect_waves_with_raw_funding_evidence(
            &view,
            &index,
            config(workers),
            &snapshot,
            &evidence,
            |_, _| UnverifiedProbeVerdict::NoLocalRejection,
        )
        .unwrap();
        assert_eq!(report.funding.status(), RAW_FUNDING_EVIDENCE_STATUS);
        assert_eq!(report.funding.funding_transactions_parsed(), 1);
        assert_eq!(report.funding.exact_prevouts_matched(), 3);
        assert_eq!(report.funding.parent_coverage.external_inputs_matched(), 3);
        assert_eq!(
            report.funding.parent_coverage.same_block_outputs_deferred(),
            0
        );
        assert_eq!(report.probe.positions_observed(), 4);
        assert_eq!(report.probe.wave_count(), 1);
        assert_eq!(report.funding.raw_evidence_bytes(), evidence[0].len());
    }
}

#[test]
fn borrower_selects_exact_output_from_both_legacy_and_segwit_funding() {
    let (raw, expected) = funding(0x21);
    for (index, (amount, script)) in expected.iter().enumerate() {
        let (decoded, decoded_script) = select_unverified_transaction_output(&raw, index as u32)
            .unwrap()
            .unwrap();
        assert_eq!(decoded, *amount);
        assert_eq!(decoded_script, script.as_slice());
    }
}

#[test]
fn independent_python_known_answer_for_one_raw_funding_transaction() {
    let (raw, _) = funding(0x53);
    assert_eq!(raw.len(), 105);
    // Pinned from hashlib.sha256(hashlib.sha256(bytes).digest()).digest().
    let python_raw_digest =
        decode_hex("01adfa763d8cb8bdf850869cd34797535ecd02ff76ff6ee7d28e97d35e85ffd8").unwrap();
    assert_eq!(
        inspect_transaction(&raw).unwrap().txid.as_slice(),
        python_raw_digest.as_slice()
    );
}

#[test]
fn several_funding_transactions_must_appear_once_in_raw_txid_sort_order() {
    let (fund_a, outputs_a) = funding(0x31);
    let (fund_b, outputs_b) = funding(0x32);
    let id_a = inspect_transaction(&fund_a).unwrap().txid;
    let id_b = inspect_transaction(&fund_b).unwrap().txid;
    let mut txs = base_transactions();
    rewire(&mut txs[1], id_a, 0);
    rewire(&mut txs[2], id_a, 1);
    rewire(&mut txs[3], id_b, 0);
    let raw = block(&txs);
    let mut snapshot = CandidateParentSnapshot {
        parent_block_hash_raw: raw[4..36].try_into().unwrap(),
        parent_height: 900_000,
        entries: vec![
            CandidateParentUtxo {
                outpoint: CandidateOutpoint {
                    txid_raw: id_a,
                    vout: 0,
                },
                amount_sat: outputs_a[0].0,
                script_pubkey: outputs_a[0].1.clone(),
                created_height: 899_500,
                is_coinbase: false,
            },
            CandidateParentUtxo {
                outpoint: CandidateOutpoint {
                    txid_raw: id_a,
                    vout: 1,
                },
                amount_sat: outputs_a[1].0,
                script_pubkey: outputs_a[1].1.clone(),
                created_height: 899_500,
                is_coinbase: false,
            },
            CandidateParentUtxo {
                outpoint: CandidateOutpoint {
                    txid_raw: id_b,
                    vout: 0,
                },
                amount_sat: outputs_b[0].0,
                script_pubkey: outputs_b[0].1.clone(),
                created_height: 899_500,
                is_coinbase: false,
            },
        ],
        expected_fingerprint: [0; 32],
    };
    snapshot.entries.sort_by_key(|entry| entry.outpoint);
    sign(&mut snapshot);
    let mut evidence = vec![fund_a, fund_b];
    evidence.sort_by_key(|fund| inspect_transaction(fund).unwrap().txid);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let index = view.inspect_serial().unwrap();
    let report = inspect_raw_funding_evidence(&view, &index, &snapshot, &evidence).unwrap();
    assert_eq!(report.funding_transactions_parsed(), 2);
    assert_eq!(report.exact_prevouts_matched(), 3);

    evidence.reverse();
    assert_eq!(
        inspect_raw_funding_evidence(&view, &index, &snapshot, &evidence),
        Err(FundingEvidenceError::UnsortedFundingTransactions { index: 1 })
    );
    evidence.pop();
    assert!(matches!(
        inspect_raw_funding_evidence(&view, &index, &snapshot, &evidence),
        Err(FundingEvidenceError::MissingFundingTransaction { .. })
    ));
}

#[test]
fn missing_funding_evidence_never_invokes_wave_callback() {
    let (raw, snapshot, _) = assemble(0x43);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let indexed = view.inspect_serial().unwrap();
    let calls = AtomicUsize::new(0);
    let result = inspect_waves_with_raw_funding_evidence(
        &view,
        &indexed,
        config(8),
        &snapshot,
        &[],
        |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            UnverifiedProbeVerdict::NoLocalRejection
        },
    );
    assert!(matches!(
        result,
        Err(FundingWaveError::Funding(
            FundingEvidenceError::MissingFundingTransaction { .. }
        ))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn modified_candidate_amount_with_updated_checksum_is_rejected() {
    let (raw, mut snapshot, evidence) = assemble(0x45);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    snapshot.entries[1].amount_sat += 1;
    sign(&mut snapshot);
    assert_eq!(
        inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence),
        Err(FundingEvidenceError::ClaimedAmountDiffers { index: 0, vout: 1 })
    );
}

#[test]
fn modified_candidate_script_with_updated_checksum_is_rejected() {
    let (raw, mut snapshot, evidence) = assemble(0x46);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    snapshot.entries[2].script_pubkey[1] ^= 1;
    sign(&mut snapshot);
    assert_eq!(
        inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence),
        Err(FundingEvidenceError::ClaimedScriptDiffers { index: 0, vout: 2 })
    );
}

#[test]
fn incorrect_raw_funding_transaction_changes_derived_txid() {
    let (raw, snapshot, mut evidence) = assemble(0x47);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    let size = evidence[0].len();
    evidence[0][size - 9] ^= 1;
    assert_eq!(
        inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence),
        Err(FundingEvidenceError::UnexpectedFundingTransaction { index: 0 })
    );
}

#[test]
fn duplicate_funding_transaction_is_not_accepted() {
    let (raw, snapshot, mut evidence) = assemble(0x48);
    evidence.push(evidence[0].clone());
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    assert_eq!(
        inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence),
        Err(FundingEvidenceError::DuplicateFundingTransaction { index: 1 })
    );
}

#[test]
fn malformed_funding_transaction_fails_before_claimed_outputs_are_accepted() {
    let (raw, snapshot, mut evidence) = assemble(0x49);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    evidence[0].truncate(13);
    assert!(matches!(
        inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence),
        Err(FundingEvidenceError::FundingTransactionMalformed { index: 0, .. })
    ));
    evidence[0] = vec![];
    assert_eq!(
        inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence),
        Err(FundingEvidenceError::EmptyFundingTransaction { index: 0 })
    );
}

#[test]
fn changing_spend_outpoint_and_claim_in_lockstep_still_detects_absent_vout() {
    let (mut raw, mut snapshot, evidence) = assemble(0x50);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let mut txs = (0..view.transaction_count())
        .map(|index| view.raw_transaction(index).unwrap().to_vec())
        .collect::<Vec<_>>();
    let fund_hash = snapshot.entries[2].outpoint.txid_raw;
    rewire(&mut txs[3], fund_hash, 9);
    raw = block(&txs);
    snapshot.parent_block_hash_raw = raw[4..36].try_into().unwrap();
    snapshot.entries[2].outpoint.vout = 9;
    sign(&mut snapshot);
    let changed_view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = changed_view.inspect_serial().unwrap();
    assert_eq!(
        inspect_raw_funding_evidence(&changed_view, &scanned, &snapshot, &evidence),
        Err(FundingEvidenceError::FundingOutputOutOfRange { index: 0, vout: 9 })
    );
}

#[test]
fn candidate_snapshot_coverage_denial_precedes_any_raw_funding_parse() {
    let (raw, mut snapshot, mut evidence) = assemble(0x51);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let index = view.inspect_serial().unwrap();
    snapshot.entries.remove(1);
    sign(&mut snapshot);
    evidence[0] = vec![0]; // Never inspected because the candidate table is incomplete.
    assert!(matches!(
        inspect_raw_funding_evidence(&view, &index, &snapshot, &evidence),
        Err(FundingEvidenceError::Snapshot(
            ParentSnapshotError::MissingExternalPrevout { .. }
        ))
    ));
}

#[test]
fn raw_funding_evidence_rejects_wrong_fork_parent() {
    let (raw, snapshot, evidence) = assemble(0x52);
    let mut changed = raw;
    changed[4] ^= 1;
    let view = UnverifiedRawBlockView::parse(&changed, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    assert_eq!(
        inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence),
        Err(FundingEvidenceError::Snapshot(
            ParentSnapshotError::ParentBlockMismatch
        ))
    );
}

#[test]
fn segregated_funding_witness_can_change_without_changing_funding_txid() {
    let (raw, snapshot, mut evidence) = assemble(0x61);
    let basic = evidence[0].clone();
    let mut segwit = Vec::new();
    segwit.extend_from_slice(&basic[..4]);
    segwit.extend_from_slice(&[0, 1]);
    segwit.extend_from_slice(&basic[4..basic.len() - 4]);
    segwit.extend_from_slice(&[1, 1, 0x51]);
    segwit.extend_from_slice(&basic[basic.len() - 4..]);
    let basic_obs = inspect_transaction(&basic).unwrap();
    let wit_obs = inspect_transaction(&segwit).unwrap();
    assert_eq!(basic_obs.txid, wit_obs.txid);
    assert_ne!(basic_obs.wtxid, wit_obs.wtxid);
    evidence[0] = segwit;
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    let observed = inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence).unwrap();
    assert_eq!(observed.exact_prevouts_matched(), 3);
    assert_eq!(
        observed.require_funding_chain_inclusion(),
        Err(FundingEvidenceTrustError::FundingChainInclusionUnknown)
    );
}

#[test]
fn an_untrusted_coinbase_flag_can_still_be_claimed_after_raw_byte_parity() {
    let (raw, mut snapshot, evidence) = assemble(0x62);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    snapshot.entries[0].is_coinbase = true;
    snapshot.entries[0].created_height = 899_500;
    sign(&mut snapshot);
    let report = inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence).unwrap();
    assert_eq!(report.exact_prevouts_matched(), 3);
    assert_eq!(
        report.require_authenticated_utxo_set(),
        Err(FundingEvidenceTrustError::CurrentUtxoStateUnauthenticated)
    );
}

#[test]
fn failed_wave_probe_after_exact_funding_witness_still_aborts() {
    let (raw, snapshot, evidence) = assemble(0x64);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    let result = inspect_waves_with_raw_funding_evidence(
        &view,
        &scanned,
        config(8),
        &snapshot,
        &evidence,
        |position, _| {
            if position == 2 {
                UnverifiedProbeVerdict::Reject
            } else {
                UnverifiedProbeVerdict::NoLocalRejection
            }
        },
    );
    assert_eq!(
        result,
        Err(FundingWaveError::Wave(WaveProbeError::ProbeRejected {
            wave: 0,
            index: 2
        }))
    );
}

#[test]
fn successful_raw_funding_comparison_never_validates_scripts_or_tl_balances() {
    let (raw, snapshot, evidence) = assemble(0x65);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let scanned = view.inspect_serial().unwrap();
    let result = inspect_raw_funding_evidence(&view, &scanned, &snapshot, &evidence).unwrap();
    assert_eq!(
        result.require_valid_scripts(),
        Err(FundingEvidenceTrustError::ScriptAndSignaturesNotVerified)
    );
    assert_eq!(
        result.require_economic_state_apply(),
        Err(FundingEvidenceTrustError::TradeLayerAndBitcoinStateApplyUnavailable)
    );
}
