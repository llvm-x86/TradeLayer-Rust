//! Stage8D: first-pass TX layout reuse, deterministic in-block prevout waves,
//! pure fixture equivalence and permanent denial of true Bitcoin consensus.
use tradelayer_core::parallel_txindex::{ParallelIndexConfig, ParallelIndexTrustError};
use tradelayer_core::raw_block_ingest::{
    merkle_root_unverified, DependencyAuditError, DependencyTrustError, UnverifiedRawBlockView,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

fn config(workers: usize, window_transactions: usize) -> ParallelIndexConfig {
    ParallelIndexConfig {
        workers,
        window_transactions,
    }
}
fn frozen() -> Vec<u8> {
    decode_hex(include_str!("../fixtures/raw_block_ingest_synthetic.hex").trim()).unwrap()
}
fn transactions() -> Vec<Vec<u8>> {
    let raw = frozen();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    (0..view.transaction_count())
        .map(|i| view.raw_transaction(i).unwrap().to_vec())
        .collect()
}
fn block(transactions: &[Vec<u8>]) -> Vec<u8> {
    assert!(!transactions.is_empty());
    assert!(transactions.len() < 253);
    let mut result = frozen()[..80].to_vec();
    let hashes = transactions
        .iter()
        .map(|tx| inspect_transaction(tx).unwrap().txid)
        .collect::<Vec<_>>();
    let root = merkle_root_unverified(&hashes).unwrap();
    result[36..68].copy_from_slice(&root);
    result.push(transactions.len() as u8);
    for tx in transactions {
        result.extend_from_slice(tx);
    }
    result
}
fn spend(tx: &mut [u8], parent: [u8; 32], vout: u32) {
    let input = if tx[4] == 0 { 7 } else { 5 };
    tx[input..input + 32].copy_from_slice(&parent);
    tx[input + 32..input + 36].copy_from_slice(&vout.to_le_bytes());
}
fn schedule(
    tx: &[Vec<u8>],
) -> Result<tradelayer_core::raw_block_ingest::UnverifiedDependencyWaves, DependencyAuditError> {
    let block = block(tx);
    let view = UnverifiedRawBlockView::parse(&block, None).unwrap();
    let index = view.inspect_parallel(config(4, 4)).unwrap();
    view.plan_dependency_waves(&index)
}
#[test]
fn reused_layout_matches_existing_independent_read_only_tx_parser() {
    let raw = frozen();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    for (i, span) in view.transaction_ranges().iter().enumerate() {
        let borrowed = view.raw_transaction(i).unwrap();
        assert_eq!(borrowed.as_ptr(), raw[span.start..].as_ptr());
        assert_eq!(
            inspect_transaction(borrowed).unwrap().txid,
            view.inspect_serial().unwrap().ordered_preindex().ordered()[i]
                .observation
                .txid
        );
    }
    for workers in [1, 2, 4, 8, 16] {
        for window in [1, 2, 3, 4, 64, 512] {
            let serial = view.inspect_serial().unwrap();
            let parallel = view.inspect_parallel(config(workers, window)).unwrap();
            assert_eq!(parallel, serial, "worker={workers} window={window}");
            assert_eq!(
                parallel.ordered_preindex().require_authenticated_block(),
                Err(ParallelIndexTrustError::FullBlockAndChainNotVerified)
            );
        }
    }
}
#[test]
fn independent_external_utxos_remain_unverified_even_in_same_wave() {
    let tx = transactions();
    let plan = schedule(&tx).unwrap();
    assert_eq!(plan.waves(), &[vec![0, 1, 2, 3]]);
    assert_eq!(plan.wave_by_transaction(), &[0, 0, 0, 0]);
    assert_eq!(plan.same_block_edges(), 0);
    assert_eq!(plan.unverified_external_prevouts(), 3);
    assert_eq!(
        plan.require_parent_utxo_snapshot(),
        Err(DependencyTrustError::ParentUtxoSnapshotUnavailable)
    );
    assert_eq!(
        plan.require_verified_script_and_coinbase(),
        Err(DependencyTrustError::InputScriptsAndCoinbaseRulesUnverified)
    );
    assert_eq!(
        plan.require_canonical_block_application(),
        Err(DependencyTrustError::CanonicalBlockStateTransitionUnavailable)
    );
}
#[test]
fn chain_of_intrablock_spends_builds_stable_wave_barriers() {
    let mut tx = transactions();
    let parent_id = inspect_transaction(&tx[1]).unwrap().txid;
    spend(&mut tx[2], parent_id, 0);
    let middle_id = inspect_transaction(&tx[2]).unwrap().txid;
    spend(&mut tx[3], middle_id, 0);
    let result = schedule(&tx).unwrap();
    assert_eq!(result.waves(), &[vec![0, 1], vec![2], vec![3]]);
    assert_eq!(result.wave_by_transaction(), &[0, 0, 1, 2]);
    assert_eq!(result.same_block_edges(), 2);
    assert_eq!(result.unverified_external_prevouts(), 1);
    for _ in 0..25 {
        assert_eq!(schedule(&tx).unwrap(), result);
    }
}
#[test]
fn duplicate_outpoint_rejects_before_independent_parallel_script_precheck() {
    let mut tx = transactions();
    let parent_id = inspect_transaction(&tx[1]).unwrap().txid;
    spend(&mut tx[2], parent_id, 0);
    spend(&mut tx[3], parent_id, 0);
    assert_eq!(
        schedule(&tx),
        Err(DependencyAuditError::DuplicateInputSpend {
            first: 2,
            second: 3,
        })
    );
}
#[test]
fn later_transaction_references_cannot_be_prechecked_in_earlier_waves() {
    let mut tx = transactions();
    let later = inspect_transaction(&tx[2]).unwrap().txid;
    spend(&mut tx[1], later, 0);
    assert_eq!(
        schedule(&tx),
        Err(DependencyAuditError::ForwardOrSelfReference {
            index: 1,
            producer: 2,
        })
    );
}
#[test]
fn nonexistent_intrablock_output_rejected_without_inventing_a_utxo() {
    let mut tx = transactions();
    let parent = inspect_transaction(&tx[1]).unwrap().txid;
    spend(&mut tx[2], parent, 20);
    assert_eq!(
        schedule(&tx),
        Err(DependencyAuditError::InvalidSameBlockOutput {
            index: 2,
            producer: 1,
        })
    );
}
#[test]
fn coinbase_and_immature_same_block_coinbase_spends_rejected() {
    let mut tx = transactions();
    spend(&mut tx[0], [17; 32], 0);
    assert_eq!(
        schedule(&tx),
        Err(DependencyAuditError::MissingFirstCoinbase)
    );
    let mut tx = transactions();
    spend(&mut tx[2], [0; 32], u32::MAX);
    assert_eq!(
        schedule(&tx),
        Err(DependencyAuditError::UnexpectedAdditionalCoinbase { index: 2 })
    );
    let mut tx = transactions();
    let coinbase_id = inspect_transaction(&tx[0]).unwrap().txid;
    spend(&mut tx[1], coinbase_id, 0);
    assert_eq!(
        schedule(&tx),
        Err(DependencyAuditError::ImmatureSameBlockCoinbaseSpend { index: 1 })
    );
}
#[test]
fn old_and_fused_path_infer_same_candidate_positions_and_witness_hashes() {
    let raw = frozen();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let prepared = view.inspect_parallel(config(4, 2)).unwrap();
    let raw_owned = transactions();
    let old = tradelayer_core::parallel_txindex::inspect_unverified_block_parallel(
        &raw_owned,
        config(4, 2),
    )
    .unwrap();
    assert_eq!(*prepared.ordered_preindex(), old);
    assert_eq!(prepared.ordered_preindex().candidate_positions(), &[1, 2]);
    assert_ne!(
        prepared.ordered_preindex().ordered()[2].observation.txid,
        prepared.ordered_preindex().ordered()[2].observation.wtxid
    );
}
