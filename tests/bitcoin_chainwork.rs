//! Stage 17: actual Bitcoin-mainnet genesis and block 1 plus explicitly
//! UNTRUSTED checkpoint/retarget metadata. No best-chain or script verification.

use std::cmp::Ordering;
use tradelayer_core::bitcoin_chainwork::{
    mainnet_block_proof_work, mainnet_retarget_bits, AnchorStatus, ClaimedMainnetCheckpoint,
    MainnetWorkChain, Work256, WorkError, MAINNET_ADJUSTMENT_INTERVAL, MAINNET_TARGET_TIMESPAN,
    MTP_WINDOW,
};
use tradelayer_core::bitcoin_inclusion::MerklePath;
use tradelayer_core::chain::{BlockHeader, CandidateBlock, ChainId};
use tradelayer_core::chain::{BlockId, Tip};
use tradelayer_core::ledger::AccountId;
use tradelayer_core::read_only::decode_hex;
use tradelayer_core::root_reconciliation::{
    reconcile_mainnet_work_headers_unverified, BridgeError, ExpectedChainRange,
};
use tradelayer_core::state_commitment::TransitionStatement;
use tradelayer_core::unified_chain::{
    AuthenticatedProtocolOperation, TrustedProtocolAdapter, UnifiedChainState,
};
use tradelayer_core::zk_batch::{AuditBatch, Digest, Receipt, SupportLevel};
use tradelayer_core::TransactionType;

// Genuine, publicly documented Bitcoin mainnet genesis block header.
const GENESIS: &str = concat!(
    "01000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a",
    "29ab5f49ffff001d1dac2b7c",
);
// Genuine mainnet height 1 header published in the Bitcoin developer docs.
// Hash: 00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048.
const BLOCK_ONE: &str = concat!(
    "01000000",
    "6fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000",
    "982051fd1e4ba744bbbe680e1fee14677ba1a3c3540bf7b1cdb606e857233e0e",
    "61bc6649ffff001d01e36299",
);
fn header(s: &str) -> [u8; 80] {
    decode_hex(s).unwrap().try_into().unwrap()
}
fn hex(raw: &[u8]) -> String {
    raw.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn claimed_epoch_checkpoint(first_time: u32, last_time: u32) -> ClaimedMainnetCheckpoint {
    let mut work = [0u8; 32];
    work[24..].copy_from_slice(&4_295_032_833u64.to_be_bytes());
    ClaimedMainnetCheckpoint {
        tip: Tip {
            height: 2015,
            id: BlockId([0x88; 32]),
        },
        bits: 0x1d00ffff,
        timestamp: last_time,
        first_period_timestamp: first_time,
        prior_timestamps: vec![last_time; MTP_WINDOW],
        cumulative_work: Work256::from_be_bytes(work),
    }
}

#[test]
fn actual_genesis_and_block_one_validate_pow_link_mtp_and_accumulated_work() {
    let mut chain = MainnetWorkChain::from_published_genesis(&header(GENESIS)).unwrap();
    assert_eq!(chain.anchor_status(), AnchorStatus::PublishedGenesis);
    assert_eq!(chain.tip().height, 0);
    assert_eq!(chain.checkpoint().bits, 0x1d00ffff);
    assert_eq!(chain.checkpoint().timestamp, 1231006505);
    assert_eq!(chain.checkpoint().prior_timestamps, vec![1231006505]);
    assert_eq!(chain.expected_next_bits().unwrap(), 0x1d00ffff);
    assert_eq!(
        hex(&chain.checkpoint().cumulative_work.to_be_bytes()),
        "0000000000000000000000000000000000000000000000000000000100010001"
    );

    let previous = chain.clone();
    let seg = chain.extend_checked(&[header(BLOCK_ONE)]).unwrap();
    assert_eq!(seg.anchor(), previous.tip());
    assert_eq!(seg.tip(), chain.tip());
    assert_eq!(seg.tip().height, 1);
    assert_eq!(
        hex(&seg.tip().id.0),
        "00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048"
    );
    assert_eq!(seg.headers()[0].n_bits(), 0x1d00ffff);
    let coinbase =
        decode_hex(include_str!("../fixtures/bitcoin_mainnet_block1_coinbase.hex").trim()).unwrap();
    let coinbase_included = seg
        .check_transaction_inclusion(
            1,
            &coinbase,
            &MerklePath {
                tx_index: 0,
                tx_count: 1,
                siblings_raw: vec![],
            },
        )
        .unwrap();
    assert_eq!(coinbase_included.height(), 1);
    assert_eq!(coinbase_included.block_id(), seg.tip().id);
    assert_eq!(coinbase_included.legacy_type_hint(), None);
    assert_eq!(
        hex(&coinbase_included.txid_display()),
        "0e3e2357e806b6cdb1f70b54c3a3a17b6714ee1f0e68bebb44a74b1efd512098"
    );
    assert_eq!(chain.checkpoint().timestamp, 1231469665);
    assert_eq!(chain.checkpoint().first_period_timestamp, 1231006505);
    assert_eq!(chain.checkpoint().prior_timestamps.len(), 2);
    assert_eq!(
        hex(&chain.checkpoint().cumulative_work.to_be_bytes()),
        "0000000000000000000000000000000000000000000000000000000200020002"
    );
    assert_eq!(
        previous.compare_work_since_same_anchor(&chain).unwrap(),
        Ordering::Less
    );
    assert_eq!(
        chain.compare_work_since_same_anchor(&previous).unwrap(),
        Ordering::Greater
    );
    assert_eq!(chain.expected_next_bits().unwrap(), 0x1d00ffff);

    // The stronger cross-root audit path accepts only opaque work receipts
    // produced by the nBits/MTP verifier, with explicit anchor provenance.
    let mut receipt_chain = previous.clone();
    let work_receipt = receipt_chain
        .extend_with_work_receipt(&[header(BLOCK_ONE)])
        .unwrap();
    assert_eq!(work_receipt.anchor_status(), AnchorStatus::PublishedGenesis);
    assert_eq!(work_receipt.checked_headers().anchor(), previous.tip());
    assert_eq!(work_receipt.checked_headers().tip(), chain.tip());
    assert_eq!(
        work_receipt.work_before(),
        previous.checkpoint().cumulative_work
    );
    assert_eq!(
        work_receipt.work_after(),
        chain.checkpoint().cumulative_work
    );
    assert_ne!(work_receipt.work_before(), work_receipt.work_after());

    let same = MainnetWorkChain::from_published_genesis(&header(GENESIS)).unwrap();
    assert_eq!(
        previous.compare_work_since_same_anchor(&same).unwrap(),
        Ordering::Equal
    );
    assert!(matches!(
        chain.compare_work_since_same_anchor(
            &MainnetWorkChain::from_claimed_checkpoint(claimed_epoch_checkpoint(
                1_000_000_000,
                1_000_604_800
            ))
            .unwrap()
        ),
        Err(WorkError::NotCommonCheckpoint)
    ));
}

#[test]
fn wrong_parent_or_pow_fails_atomically_after_real_genesis() {
    let mut chain = MainnetWorkChain::from_published_genesis(&header(GENESIS)).unwrap();
    let before = chain.clone();
    let mut bad = header(BLOCK_ONE);
    bad[4] ^= 1; // header parent
    assert!(chain.extend_checked(&[bad]).is_err());
    assert_eq!(before, chain);

    let wrong = header(BLOCK_ONE);
    assert!(chain.extend_checked(&[wrong, wrong]).is_err());
    assert_eq!(before, chain);
    assert!(chain.extend_checked(&[]).is_err());
    assert_eq!(before, chain);
    assert!(chain.extend_checked(&[header(GENESIS)]).is_err());
    assert_eq!(before, chain);
    assert!(MainnetWorkChain::from_published_genesis(&header(BLOCK_ONE)).is_err());
}

#[test]
fn bitcoin_core_mainnet_retarget_and_clamp_regression() {
    assert_eq!(MAINNET_ADJUSTMENT_INTERVAL, 2016);
    assert_eq!(MAINNET_TARGET_TIMESPAN, 1_209_600);
    assert_eq!(MTP_WINDOW, 11);
    assert_eq!(mainnet_retarget_bits(0x1d00ffff, 0, 0).unwrap(), 0x1c3fffc0);
    assert_eq!(
        mainnet_retarget_bits(0x1d00ffff, 0, 302_400).unwrap(),
        0x1c3fffc0
    );
    assert_eq!(
        mainnet_retarget_bits(0x1d00ffff, 0, 604_800).unwrap(),
        0x1c7fff80
    );
    assert_eq!(
        mainnet_retarget_bits(0x1d00ffff, 0, 1_209_600).unwrap(),
        0x1d00ffff
    );
    assert_eq!(
        mainnet_retarget_bits(0x1d00ffff, 0, 9_676_800).unwrap(),
        0x1d00ffff
    );
    // For a harder *previous* epoch target, retargeting allows a 2x
    // reduction in difficulty and a maximum 4x adjustment.
    assert_eq!(
        mainnet_retarget_bits(0x1c00ffff, 0, 2_419_200).unwrap(),
        0x1c01fffe
    );
    assert_eq!(
        mainnet_retarget_bits(0x1c00ffff, 0, 4_838_400).unwrap(),
        0x1c03fffc
    );

    let anchored = claimed_epoch_checkpoint(1_000_000_000, 1_000_604_800);
    let chain = MainnetWorkChain::from_claimed_checkpoint(anchored.clone()).unwrap();
    assert_eq!(chain.anchor_status(), AnchorStatus::ClaimedExternal);
    assert_eq!(chain.tip().height, 2015);
    assert_eq!(chain.expected_next_bits().unwrap(), 0x1c7fff80);
    assert_eq!(chain.checkpoint(), &anchored);
    assert_eq!(
        mainnet_block_proof_work(0x1d00ffff).unwrap().to_be_bytes()[24..],
        4_295_032_833u64.to_be_bytes()
    );
}

#[test]
fn forged_checkpoint_shape_and_cross_anchor_comparison_fail_closed() {
    let checkpoint = claimed_epoch_checkpoint(1_000_000_000, 1_000_604_800);
    let mut bogus = checkpoint.clone();
    bogus.prior_timestamps.pop();
    assert!(matches!(
        MainnetWorkChain::from_claimed_checkpoint(bogus),
        Err(WorkError::InvalidCheckpoint)
    ));
    bogus = checkpoint.clone();
    bogus.first_period_timestamp = 0;
    assert!(matches!(
        MainnetWorkChain::from_claimed_checkpoint(bogus),
        Err(WorkError::InvalidCheckpoint)
    ));
    bogus = checkpoint.clone();
    bogus.bits = 0x207fffff;
    assert!(MainnetWorkChain::from_claimed_checkpoint(bogus).is_err());
    bogus = checkpoint.clone();
    bogus.cumulative_work = Work256::from_be_bytes([0; 32]);
    assert!(matches!(
        MainnetWorkChain::from_claimed_checkpoint(bogus),
        Err(WorkError::InvalidCheckpoint)
    ));
    bogus = checkpoint.clone();
    bogus.tip.height = 2016;
    assert!(matches!(
        MainnetWorkChain::from_claimed_checkpoint(bogus),
        Err(WorkError::InvalidCheckpoint)
    ));
    bogus = checkpoint.clone();
    bogus.tip.id.0[0] ^= 1;
    let left = MainnetWorkChain::from_claimed_checkpoint(checkpoint).unwrap();
    let right = MainnetWorkChain::from_claimed_checkpoint(bogus).unwrap();
    assert!(matches!(
        left.compare_work_since_same_anchor(&right),
        Err(WorkError::NotCommonCheckpoint)
    ));
}

#[test]
fn twenty_five_python_bigint_vectors_cover_full_u256_chainwork_width() {
    let mut checked = 0;
    for row in include_str!("../fixtures/bitcoin_chainwork_vectors.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let (bits, expected) = row.split_once('\t').expect("two columns");
        let bits = u32::from_str_radix(bits, 16).unwrap();
        let actual = mainnet_block_proof_work(bits).unwrap();
        assert_eq!(hex(&actual.to_be_bytes()), expected);
        checked += 1;
    }
    assert_eq!(checked, 25);
    for bits in [0u32, 0x1d80ffff, 0x1d000000, 0x1e00ffff, 0x2100ffff] {
        assert!(
            mainnet_block_proof_work(bits).is_err(),
            "invalid bits: {bits:08x}"
        );
    }
}

struct SyntheticNoTransactions;
impl TrustedProtocolAdapter for SyntheticNoTransactions {
    fn verify_block(&self, _: &CandidateBlock) -> Result<(), String> {
        Ok(())
    }
    fn authenticate_and_decode(
        &self,
        _: ChainId,
        _: &BlockHeader,
        _: &[u8],
    ) -> Result<Option<AuthenticatedProtocolOperation>, String> {
        Ok(None)
    }
}

fn mock_digest(byte: u8) -> Digest {
    Digest::parse(&format!("{byte:02x}").repeat(32)).unwrap()
}

#[test]
fn genuine_mainnet_merkle_inclusion_cannot_impersonate_a_trade_layer_receipt() {
    let mut chain = MainnetWorkChain::from_published_genesis(&header(GENESIS)).unwrap();
    let receipt = chain
        .extend_with_work_receipt(&[header(BLOCK_ONE)])
        .unwrap();
    assert_eq!(receipt.anchor_status(), AnchorStatus::PublishedGenesis);
    let coinbase =
        decode_hex(include_str!("../fixtures/bitcoin_mainnet_block1_coinbase.hex").trim()).unwrap();
    let included = receipt
        .checked_headers()
        .check_transaction_inclusion(
            1,
            &coinbase,
            &MerklePath {
                tx_index: 0,
                tx_count: 1,
                siblings_raw: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(included.legacy_type_hint(), None);

    // Deliberately invent a zkTL batch claiming the genuine Bitcoin coinbase
    // was a TradeLayer type-2 send. Every zkTL metadata hash is consistent;
    // the actual transaction, however, has no TradeLayer OP_RETURN.
    let txid = Digest::parse(&hex(&included.txid_display())).unwrap();
    let mut tx = Receipt {
        receipt_id: mock_digest(0),
        txid,
        tx_type: TransactionType::try_from(2).unwrap(),
        height: 1,
        old_state_root: mock_digest(1),
        new_state_root: mock_digest(2),
        payload_hash: mock_digest(3),
        witness_root: mock_digest(4),
        support: SupportLevel::StateKernel,
        economic_mutation: true,
        valid: true,
    };
    tx.receipt_id = tx.calculate_receipt_id();
    let mut batch = AuditBatch {
        batch_id: mock_digest(0),
        transition_root: mock_digest(0),
        first_state_root: tx.old_state_root,
        final_state_root: tx.new_state_root,
        start_height: 1,
        end_height: 1,
        receipts: vec![tx],
    };
    batch.transition_root = batch.calculate_transition_root();
    batch.batch_id = batch.calculate_batch_id();
    batch.inspect_strict().unwrap();

    // Construct one local TLPT statement only to exercise *the audit
    // boundary*. Its actor, protocol state and local adapter are synthetic.
    let namespace = ChainId([7; 32]);
    let anchor = receipt.checked_headers().anchor();
    let prior = UnifiedChainState::new_unverified(
        namespace,
        anchor,
        AccountId::try_from("admin").unwrap(),
        2,
    )
    .unwrap();
    let mut next = prior.clone();
    next.apply_block(
        &CandidateBlock {
            chain: namespace,
            header: BlockHeader {
                height: 1,
                id: receipt.checked_headers().tip().id,
                parent: anchor.id,
            },
            raw_transactions: Vec::new(),
        },
        &SyntheticNoTransactions,
    )
    .unwrap();
    let statement = TransitionStatement::from_reversible_states_unverified(&prior, &next).unwrap();
    let expected = ExpectedChainRange {
        chain: namespace,
        anchor,
        final_tip: receipt.checked_headers().tip(),
    };

    assert!(matches!(
        reconcile_mainnet_work_headers_unverified(
            &batch,
            &[statement],
            &receipt,
            &[included],
            expected,
        ),
        Err(BridgeError::ReceiptMismatch)
    ));
}

#[test]
fn fraudulent_checkpoint_context_cannot_bypass_mtp_or_nbits_checks() {
    let genesis = MainnetWorkChain::from_published_genesis(&header(GENESIS)).unwrap();
    let mut declared = genesis.checkpoint().clone();
    // Claimed timestamp is later than the genuine block-1 timestamp.
    // The real block-1 PoW and parent still check out, but the separately
    // declared MTP checkpoint context is inconsistent with its time.
    declared.timestamp = 1_231_469_666;
    declared.first_period_timestamp = declared.timestamp;
    declared.prior_timestamps = vec![declared.timestamp];
    let mut chain = MainnetWorkChain::from_claimed_checkpoint(declared).unwrap();
    let before = chain.clone();
    assert!(matches!(
        chain.extend_checked(&[header(BLOCK_ONE)]),
        Err(WorkError::MedianTimePast)
    ));
    assert_eq!(chain, before);

    // A bogus checkpoint pretending Genesis was mined at *harder* bits
    // cannot accept the genuine block-1 easier target as the next nBits.
    let mut declared = genesis.checkpoint().clone();
    declared.bits = 0x1c00ffff;
    declared.cumulative_work = mainnet_block_proof_work(declared.bits).unwrap();
    let mut chain = MainnetWorkChain::from_claimed_checkpoint(declared).unwrap();
    let before = chain.clone();
    assert!(matches!(
        chain.extend_checked(&[header(BLOCK_ONE)]),
        Err(WorkError::IncorrectDifficulty {
            expected: 0x1c00ffff,
            actual: 0x1d00ffff,
        })
    ));
    assert_eq!(chain, before);
}
