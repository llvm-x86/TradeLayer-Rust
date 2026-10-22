//! Stage 15 cross-namespace root reconciliation. All L1 observations here are
//! synthetic declarations; matching root metadata is NOT a proof of inclusion,
//! accepted economics, valid signatures, or genuine Cairo/STWO proof bytes.

use tradelayer_core::chain::{BlockHeader, BlockId, CandidateBlock, ChainId, Tip};
use tradelayer_core::ledger::AccountId;
use tradelayer_core::root_reconciliation::{
    parse_observations_tsv_unverified, parse_tlpt_tsv_unverified, reconcile_unverified,
    BridgeError, ExpectedChainRange, ObservedReceipt, MAX_ALIGNED_BLOCKS,
};
use tradelayer_core::state_commitment::TransitionStatement;
use tradelayer_core::unified_chain::{
    AuthenticatedProtocolOperation, TrustedProtocolAdapter, UnifiedChainState,
};
use tradelayer_core::zk_batch::{parse_bridge_tsv, AuditBatch, ClaimError, Digest};

const ZK_SAMPLE: &str = include_str!("../fixtures/zkTL_sample_batch.tsv");
const TLPT_SAMPLE: &str = include_str!("../fixtures/tlpt_v1_vectors.tsv");

fn account(value: &str) -> AccountId {
    AccountId::try_from(value).unwrap()
}
fn chain() -> ChainId {
    ChainId([11; 32])
}
fn synthetic_block_id(index: u8) -> BlockId {
    BlockId([20 + index; 32])
}
struct SyntheticHeadersOnly;
impl TrustedProtocolAdapter for SyntheticHeadersOnly {
    fn verify_block(&self, _: &CandidateBlock) -> Result<(), String> {
        Ok(())
    }
    fn authenticate_and_decode(
        &self,
        _: ChainId,
        _: &BlockHeader,
        _: &[u8],
    ) -> Result<Option<AuthenticatedProtocolOperation>, String> {
        Err("no synthetic transactions are permitted".to_string())
    }
}

/// Produce local reversible TLPT metadata over eight fake blocks. A real
/// archive would supply verified L1 block identifiers and transaction records.
fn mock_statements(batch: &AuditBatch, admin: &str) -> Vec<TransitionStatement> {
    let anchor = Tip {
        height: batch.start_height - 1,
        id: BlockId([19; 32]),
    };
    let mut state = UnifiedChainState::new_unverified(chain(), anchor, account(admin), 16).unwrap();
    let mut steps = Vec::new();
    for i in 0..=(batch.end_height - batch.start_height) {
        let previous = state.clone();
        let id = synthetic_block_id(u8::try_from(i).unwrap());
        state
            .apply_block(
                &CandidateBlock {
                    chain: chain(),
                    header: BlockHeader {
                        height: previous.tip().height + 1,
                        id,
                        parent: previous.tip().id,
                    },
                    raw_transactions: vec![],
                },
                &SyntheticHeadersOnly,
            )
            .unwrap();
        steps.push(
            TransitionStatement::from_reversible_states_unverified(&previous, &state).unwrap(),
        );
    }
    steps
}

fn mock_records(batch: &AuditBatch, steps: &[TransitionStatement]) -> Vec<ObservedReceipt> {
    let mut counts = vec![0u32; steps.len()];
    batch
        .receipts
        .iter()
        .map(|receipt| {
            let offset = (receipt.height - batch.start_height) as usize;
            let order = counts[offset];
            counts[offset] += 1;
            ObservedReceipt {
                height: receipt.height,
                block_id: steps[offset].child().id,
                txid: receipt.txid,
                tx_type: receipt.tx_type,
                order_in_block: order,
            }
        })
        .collect()
}

fn setup() -> (
    AuditBatch,
    Vec<TransitionStatement>,
    Vec<ObservedReceipt>,
    ExpectedChainRange,
) {
    let batch = parse_bridge_tsv(ZK_SAMPLE).unwrap();
    batch.inspect_strict().unwrap();
    let statements = mock_statements(&batch, "admin");
    let observations = mock_records(&batch, &statements);
    let expected = ExpectedChainRange {
        chain: chain(),
        anchor: statements[0].parent(),
        final_tip: statements.last().unwrap().child(),
    };
    (batch, statements, observations, expected)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn statement_rows(statements: &[TransitionStatement]) -> String {
    let mut rows = String::new();
    for statement in statements {
        rows.push_str(&hex(&statement.id()));
        rows.push('\t');
        rows.push_str(&hex(&statement.canonical_bytes()));
        rows.push('\n');
    }
    rows
}
fn observation_rows(observations: &[ObservedReceipt]) -> String {
    let mut rows = String::new();
    for record in observations {
        rows.push_str(&format!(
            "O\t{}\t{}\t{}\t{}\t{}\n",
            record.height,
            hex(&record.block_id.0),
            record.txid.hex(),
            record.tx_type.as_u8(),
            record.order_in_block,
        ));
    }
    rows
}

#[test]
fn source_derived_26_receipts_align_across_eight_synthetic_tlpt_blocks() {
    let (batch, steps, records, expected) = setup();
    assert_eq!(steps.len(), 8);
    assert_eq!(records.len(), 26);
    let report = reconcile_unverified(&batch, &steps, &records, expected).unwrap();
    assert_eq!(report.receipt_count, 26);
    assert_eq!(report.blocks.len(), 8);
    assert_eq!(report.blocks[0].height, 132_900);
    assert_eq!(report.blocks.last().unwrap().height, 132_907);
    assert_eq!(report.expected, expected);
    assert_eq!(report.zktl_batch_id, batch.batch_id);
    assert_eq!(report.zktl_first_root, batch.first_state_root);
    assert_eq!(report.zktl_last_root, batch.final_state_root);
    assert_eq!(report.rust_first_root, steps[0].old_protocol_root());
    assert_eq!(
        report.rust_last_root,
        steps.last().unwrap().new_protocol_root()
    );
    assert_ne!(report.audit_digest, [0; 32]);
    assert_ne!(report.rust_first_root, report.zktl_first_root.as_bytes());
    assert_eq!(
        report
            .blocks
            .iter()
            .map(|block| block.receipt_count as usize)
            .sum::<usize>(),
        26
    );
    for pair in report.blocks.windows(2) {
        assert_eq!(pair[0].rust_after, pair[1].rust_before);
        assert_eq!(pair[0].zktl_after, pair[1].zktl_before);
    }
    // The report digest is deterministic for the *same* declared evidence.
    assert_eq!(
        reconcile_unverified(&batch, &steps, &records, expected).unwrap(),
        report
    );
}

#[test]
fn parsed_wire_files_reconcile_identically_to_typed_inputs() {
    let (batch, steps, observations, expected) = setup();
    let tlpt = statement_rows(&steps);
    let record_bytes = observation_rows(&observations);
    let parsed_steps = parse_tlpt_tsv_unverified(&tlpt).unwrap();
    let parsed_records = parse_observations_tsv_unverified(&record_bytes).unwrap();
    assert_eq!(parsed_steps, steps);
    assert_eq!(parsed_records, observations);
    let from_wire = reconcile_unverified(&batch, &parsed_steps, &parsed_records, expected).unwrap();
    let typed = reconcile_unverified(&batch, &steps, &observations, expected).unwrap();
    assert_eq!(from_wire.audit_digest, typed.audit_digest);

    // The previously independently generated one-block TLPT fixture still
    // parses through the same strict transport format.
    assert_eq!(parse_tlpt_tsv_unverified(TLPT_SAMPLE).unwrap().len(), 1);
    let uppercase = tlpt.to_ascii_uppercase();
    assert!(parse_tlpt_tsv_unverified(&uppercase).is_err());
    assert!(parse_tlpt_tsv_unverified(&(tlpt + "JUNK")).is_err());
    assert!(
        parse_observations_tsv_unverified(&record_bytes.replacen("\t0\n", "\t00\n", 1),).is_err()
    );
    assert!(parse_observations_tsv_unverified("O\tbad\t0\t0\t0\t0\n").is_err());
}

#[test]
fn mismatched_or_reordered_observations_are_always_rejected() {
    let (batch, steps, mut observations, expected) = setup();
    let original = observations.clone();
    observations[0].txid = observations[1].txid;
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &observations, expected),
        Err(BridgeError::ReceiptMismatch)
    ));
    observations = original.clone();
    observations[0].block_id = BlockId([1; 32]);
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &observations, expected),
        Err(BridgeError::ReceiptMismatch)
    ));
    observations = original.clone();
    observations[0].order_in_block += 1;
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &observations, expected),
        Err(BridgeError::ReceiptMismatch)
    ));
    observations = original.clone();
    observations[0].height += 1;
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &observations, expected),
        Err(BridgeError::ReceiptMismatch)
    ));
    observations = original.clone();
    observations.swap(0, 1);
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &observations, expected),
        Err(BridgeError::ReceiptMismatch)
    ));
    observations = original;
    observations.push(observations[0]);
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &observations, expected),
        Err(BridgeError::ReceiptMismatch)
    ));
}

#[test]
fn wrong_anchors_chain_and_block_coverage_fail() {
    let (batch, steps, records, mut expected) = setup();
    expected.chain = ChainId([88; 32]);
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &records, expected),
        Err(BridgeError::WrongChain)
    ));
    expected = setup().3;
    expected.anchor.id = BlockId([55; 32]);
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &records, expected),
        Err(BridgeError::WrongAnchors)
    ));
    expected = setup().3;
    expected.final_tip.height += 1;
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &records, expected),
        Err(BridgeError::WrongAnchors)
    ));
    expected = setup().3;
    assert!(matches!(
        reconcile_unverified(&batch, &steps[..steps.len() - 1], &records, expected),
        Err(BridgeError::MissingBlock)
    ));
    let mut displaced = steps.clone();
    displaced.swap(2, 3);
    assert!(matches!(
        reconcile_unverified(&batch, &displaced, &records, expected),
        Err(BridgeError::MissingBlock)
    ));
    let many = vec![steps[0]; MAX_ALIGNED_BLOCKS + 1];
    assert!(matches!(
        reconcile_unverified(&batch, &many, &records, expected),
        Err(BridgeError::TooManyBlocks)
    ));
}

#[test]
fn equivalent_block_tips_with_different_rust_state_are_not_equivalent() {
    let (batch, mut statements, observations, expected) = setup();
    let other = mock_statements(&batch, "different-admin");
    assert_eq!(statements[3].parent(), other[3].parent());
    assert_eq!(statements[3].child(), other[3].child());
    assert_ne!(
        statements[3].old_protocol_root(),
        other[3].old_protocol_root()
    );
    statements[3] = other[3];
    assert!(matches!(
        reconcile_unverified(&batch, &statements, &observations, expected),
        Err(BridgeError::DiscontinuousRustState)
    ));
    statements = mock_statements(&batch, "admin");
    let mut alt = mock_statements(&batch, "admin");
    // Change exactly the block ID for block 132902: the alternate TLPT
    // child tip conflicts with the next declaration's parent hash.
    let anchor = Tip {
        height: batch.start_height - 1,
        id: BlockId([19; 32]),
    };
    let mut s = UnifiedChainState::new_unverified(chain(), anchor, account("admin"), 16).unwrap();
    for i in 0..3 {
        let prior = s.clone();
        let mut block_id = synthetic_block_id(i);
        if i == 2 {
            block_id = BlockId([251; 32]);
        }
        s.apply_block(
            &CandidateBlock {
                chain: chain(),
                header: BlockHeader {
                    height: prior.tip().height + 1,
                    parent: prior.tip().id,
                    id: block_id,
                },
                raw_transactions: vec![],
            },
            &SyntheticHeadersOnly,
        )
        .unwrap();
        if i == 2 {
            alt[i as usize] =
                TransitionStatement::from_reversible_states_unverified(&prior, &s).unwrap();
        }
    }
    statements[2] = alt[2];
    assert!(matches!(
        reconcile_unverified(&batch, &statements, &observations, expected),
        Err(BridgeError::DiscontinuousTip)
    ));
}

#[test]
fn corrupted_zktl_batch_is_rejected_before_cross_namespace_comparison() {
    let (mut batch, steps, observations, expected) = setup();
    batch.receipts[0].valid = false;
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &observations, expected),
        Err(BridgeError::Zk(ClaimError::Invalid("invalid receipt")))
    ));
    batch = setup().0;
    batch.batch_id =
        Digest::parse("1111111111111111111111111111111111111111111111111111111111111111").unwrap();
    assert!(matches!(
        reconcile_unverified(&batch, &steps, &observations, expected),
        Err(BridgeError::Zk(ClaimError::Mismatch("batchId")))
    ));
}
