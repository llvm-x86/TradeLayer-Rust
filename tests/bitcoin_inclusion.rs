//! Stage 16 offline Bitcoin SHA256d txid-Merkle proofs.
//! The regtest-like headers below are EXPLICITLY FABRICATED test data.
//! Hash/PoW/Merkle consistency does not prove best-chain membership,
//! valid scripts, authenticated spend authority or TradeLayer economics.

use tradelayer_core::bitcoin_inclusion::{
    BitcoinPowProfile, CheckedHeader, CheckedHeaderSegment, InclusionError, MerklePath,
    MAX_HEADER_SEGMENT,
};
use tradelayer_core::chain::{BlockHeader, BlockId, CandidateBlock, ChainId, Tip};
use tradelayer_core::ledger::AccountId;
use tradelayer_core::read_only::{decode_hex, inspect_transaction};
use tradelayer_core::root_reconciliation::{
    reconcile_checked_bitcoin_inclusions_unverified, BridgeError, ExpectedChainRange,
};
use tradelayer_core::state_commitment::TransitionStatement;
use tradelayer_core::unified_chain::{
    AuthenticatedProtocolOperation, TrustedProtocolAdapter, UnifiedChainState,
};
use tradelayer_core::zk_batch::{AuditBatch, Digest, Receipt, SupportLevel};
use tradelayer_core::TransactionType;

// Known published Bitcoin genesis 80-byte header, NOT a synthetic block.
const GENESIS_HEADER: &str = concat!(
    "01000000",
    "0000000000000000000000000000000000000000000000000000000000000000",
    "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a",
    "29ab5f49ffff001d1dac2b7c",
);

// Regtest-PoW-like but UNTRUSTED fabricated headers.
// First block contains synthetic_transactions.tsv row 1, second block has
// all three synthetic transactions, including an odd-leaf Merkle padding.
const SYNTHETIC_101: &str = concat!(
    "02000000",
    "1212121212121212121212121212121212121212121212121212121212121212",
    "a62ca75fab28cf66b31a2651b76321a511e58f1152192b48395ddd2e0ebc3a08",
    "00f15365ffff7f2000000000",
);
const SYNTHETIC_102: &str = concat!(
    "02000000",
    "f6c5fe7671b5c08bc0a40a5dadd59780c9b12016872a763d1a53eef7fa3db47b",
    "dbbe90fe71b386b1c3147ee2e55e4c2a65007b88d8d41d678f6b5b6c56a1432e",
    "01f15365ffff7f2000000000",
);
const SEGWIT_FAKE: &str = concat!(
    "02000000",
    "2121212121212121212121212121212121212121212121212121212121212121",
    "ff39cb552378ec67948ada9324e5d469c8fb3eaaeef50eab885a730b90ddfd00",
    "02f15365ffff7f2000000000",
);

fn raw(hex: &str) -> Vec<u8> {
    decode_hex(hex).unwrap()
}
fn header(hex: &str) -> [u8; 80] {
    raw(hex).try_into().unwrap()
}
fn synthetic_rows() -> Vec<Vec<u8>> {
    include_str!("../fixtures/synthetic_transactions.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| raw(line.split('\t').next_back().unwrap()))
        .collect()
}
fn genesis_tx() -> Vec<u8> {
    raw(include_str!("../fixtures/real_transactions.tsv")
        .lines()
        .find(|line| line.starts_with("bitcoin-genesis\t"))
        .unwrap()
        .split('\t')
        .next_back()
        .unwrap())
}
fn segwit_tx() -> Vec<u8> {
    raw(include_str!("../fixtures/real_transactions.tsv")
        .lines()
        .find(|line| line.starts_with("segwit-opreturn-example\t"))
        .unwrap()
        .split('\t')
        .next_back()
        .unwrap())
}
fn sibling(hex: &str) -> [u8; 32] {
    raw(hex).try_into().unwrap()
}
fn anchor() -> Tip {
    Tip {
        height: 100,
        id: BlockId([0x12; 32]),
    }
}
fn fake_segment() -> CheckedHeaderSegment {
    CheckedHeaderSegment::check_against_claimed_anchor(
        anchor(),
        &[header(SYNTHETIC_101), header(SYNTHETIC_102)],
        BitcoinPowProfile::RegtestLike,
    )
    .unwrap()
}
fn digest(byte: u8) -> Digest {
    Digest::parse(&format!("{byte:02x}").repeat(32)).unwrap()
}
fn chain() -> ChainId {
    ChainId([0x11; 32])
}
fn account(name: &str) -> AccountId {
    AccountId::try_from(name).unwrap()
}

#[test]
fn published_bitcoin_genesis_pow_header_and_coinbase_merkle_proof() {
    let genesis = CheckedHeader::check_bitcoin_mainnet_genesis(&header(GENESIS_HEADER)).unwrap();
    assert_eq!(genesis.tip().height, 0);
    assert_eq!(genesis.n_bits(), 0x1d00ffff);
    assert_eq!(genesis.parent(), BlockId([0; 32]));
    assert_eq!(
        genesis
            .tip()
            .id
            .0
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
    );
    let tx = genesis
        .check_transaction_inclusion(
            &genesis_tx(),
            &MerklePath {
                tx_index: 0,
                tx_count: 1,
                siblings_raw: vec![],
            },
        )
        .unwrap();
    assert_eq!(tx.height(), 0);
    assert_eq!(tx.tx_index(), 0);
    assert_eq!(tx.tx_count(), 1);
    assert_eq!(tx.block_id(), genesis.tip().id);
    assert_eq!(tx.legacy_type_hint(), None); // no TradeLayer claims in genesis.
    assert_eq!(
        tx.txid_display()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b",
    );
    assert!(CheckedHeader::check_bitcoin_mainnet_genesis(&header(SYNTHETIC_101),).is_err());
}

#[test]
fn fabricated_regtest_like_headers_bind_parent_hash_and_merkle_roots() {
    let seg = fake_segment();
    assert_eq!(seg.anchor(), anchor());
    assert_eq!(seg.headers().len(), 2);
    assert_eq!(seg.headers()[0].parent(), anchor().id);
    assert_eq!(seg.headers()[1].parent(), seg.headers()[0].tip().id);
    assert_eq!(seg.tip().height, 102);
    assert_eq!(
        seg.tip()
            .id
            .0
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "32b5e3e509902ab61ce6e0d96e1e53c5721be388eee22a836cd55ac4c0d5ac23"
    );
    assert!(matches!(
        CheckedHeaderSegment::check_against_claimed_anchor(
            anchor(),
            &[header(SYNTHETIC_101)],
            BitcoinPowProfile::MainnetLike
        ),
        Err(InclusionError::HeaderTooEasy)
    ));
    assert!(matches!(
        CheckedHeaderSegment::check_against_claimed_anchor(
            anchor(),
            &[header(SYNTHETIC_102)],
            BitcoinPowProfile::RegtestLike
        ),
        Err(InclusionError::InvalidHeaderChain)
    ));
    assert!(matches!(
        CheckedHeaderSegment::check_against_claimed_anchor(
            anchor(),
            &[],
            BitcoinPowProfile::RegtestLike
        ),
        Err(InclusionError::TooManyHeaders)
    ));
    assert_eq!(MAX_HEADER_SEGMENT, 500);
    let mut bad_bits = header(SYNTHETIC_101);
    bad_bits[75] = 0x21; // above the regtest pow limit
    assert!(CheckedHeader::check_header_at_claimed_height(
        101,
        &bad_bits,
        BitcoinPowProfile::RegtestLike,
    )
    .is_err());
}

#[test]
fn exact_three_transaction_merkle_paths_and_odd_padding_are_checked() {
    let seg = fake_segment();
    let rows = synthetic_rows();
    assert_eq!(rows.len(), 3);
    // Transaction zero: sibling 1, then odd-leaf duplicate pair 2||2.
    let proof0 = MerklePath {
        tx_index: 0,
        tx_count: 3,
        siblings_raw: vec![
            sibling("2e83799f01553f9ba87efb9493c667ea2fbf82062a521c1779f18d07607ae543"),
            sibling("5f041350e9b5584f60c507acbcfe609e8cac8c2eb509609d611b470c2fce6b42"),
        ],
    };
    let zero = seg
        .check_transaction_inclusion(102, &rows[0], &proof0)
        .unwrap();
    assert_eq!(
        zero.legacy_type_hint(),
        Some(TransactionType::try_from(2).unwrap())
    );
    assert_eq!(zero.tx_index(), 0);
    let proof2 = MerklePath {
        tx_index: 2,
        tx_count: 3,
        siblings_raw: vec![
            sibling("309bbc86d0ea1378afa55e2fee37fa257cc3e33953ef528412684f6787b67381"),
            sibling("34f4083fb6ef5e99a1477db6160cc4153bf329fb91f7313f2c5c76c077c99c7a"),
        ],
    };
    let third = seg
        .check_transaction_inclusion(102, &rows[2], &proof2)
        .unwrap();
    assert_eq!(third.tx_index(), 2);
    assert_eq!(third.legacy_type_hint(), None); // nested legacy frame; invalid syntax.

    // A real equal Merkle sibling, unlike odd padding, is ambiguous and
    // must be rejected even if an attacker can make a matching commitment.
    let mut broken = proof0.clone();
    broken.siblings_raw[0] = inspect_transaction(&rows[0]).unwrap().txid;
    assert!(matches!(
        seg.check_transaction_inclusion(102, &rows[0], &broken),
        Err(InclusionError::MutatedMerkleBranch)
    ));
    broken = proof0.clone();
    broken.siblings_raw[1][0] ^= 1;
    assert!(matches!(
        seg.check_transaction_inclusion(102, &rows[0], &broken),
        Err(InclusionError::MerkleRootMismatch)
    ));
    broken = proof2.clone();
    broken.siblings_raw[0][0] ^= 1;
    assert!(seg
        .check_transaction_inclusion(102, &rows[2], &broken)
        .is_err());
    broken = proof2.clone();
    broken.tx_index = 3;
    assert!(matches!(
        seg.check_transaction_inclusion(102, &rows[2], &broken),
        Err(InclusionError::InvalidProof)
    ));
    broken = proof2.clone();
    broken.siblings_raw.pop();
    assert!(matches!(
        seg.check_transaction_inclusion(102, &rows[2], &broken),
        Err(InclusionError::InvalidProof)
    ));
    assert!(seg
        .check_transaction_inclusion(103, &rows[0], &proof0)
        .is_err());
}

#[test]
fn segwit_merkle_leaf_uses_nonwitness_txid_not_wtxid() {
    let header = CheckedHeader::check_header_at_claimed_height(
        500,
        &header(SEGWIT_FAKE),
        BitcoinPowProfile::RegtestLike,
    )
    .unwrap();
    let raw_tx = segwit_tx();
    let inspected = inspect_transaction(&raw_tx).unwrap();
    assert!(inspected.has_witness);
    assert_ne!(inspected.txid, inspected.wtxid);
    let proof = MerklePath {
        tx_index: 0,
        tx_count: 1,
        siblings_raw: vec![],
    };
    let included = header.check_transaction_inclusion(&raw_tx, &proof).unwrap();
    assert_eq!(
        included
            .txid_display()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "00fddd900b735a88ab0ef5eeaa3efbc869d4e52493da8a9467ec782355cb39ff",
    );
}

struct SyntheticTipOnly;
impl TrustedProtocolAdapter for SyntheticTipOnly {
    fn verify_block(&self, _: &CandidateBlock) -> Result<(), String> {
        Ok(())
    }
    fn authenticate_and_decode(
        &self,
        _: ChainId,
        _: &BlockHeader,
        _: &[u8],
    ) -> Result<Option<AuthenticatedProtocolOperation>, String> {
        Err("no chain-authenticated transactions in synthetic TLPT test".to_owned())
    }
}

fn synthetic_tlpt(seg: &CheckedHeaderSegment) -> Vec<TransitionStatement> {
    let mut state =
        UnifiedChainState::new_unverified(chain(), anchor(), account("admin"), 4).unwrap();
    let mut result = Vec::new();
    for header in seg.headers() {
        let prior = state.clone();
        state
            .apply_block(
                &CandidateBlock {
                    chain: chain(),
                    header: BlockHeader {
                        height: header.tip().height,
                        id: header.tip().id,
                        parent: header.parent(),
                    },
                    raw_transactions: Vec::new(),
                },
                &SyntheticTipOnly,
            )
            .unwrap();
        result
            .push(TransitionStatement::from_reversible_states_unverified(&prior, &state).unwrap());
    }
    result
}
fn one_receipt_batch(txid: Digest) -> AuditBatch {
    let mut receipt = Receipt {
        receipt_id: digest(0),
        txid,
        tx_type: TransactionType::try_from(2).unwrap(),
        height: 101,
        old_state_root: digest(0x91),
        new_state_root: digest(0x92),
        payload_hash: digest(0x93),
        witness_root: digest(0x94),
        support: SupportLevel::StateKernel,
        economic_mutation: true,
        valid: true,
    };
    receipt.receipt_id = receipt.calculate_receipt_id();
    let mut batch = AuditBatch {
        batch_id: digest(0),
        transition_root: digest(0),
        first_state_root: receipt.old_state_root,
        final_state_root: receipt.new_state_root,
        start_height: 101,
        end_height: 102,
        receipts: vec![receipt],
    };
    batch.transition_root = batch.calculate_transition_root();
    batch.batch_id = batch.calculate_batch_id();
    batch.inspect_strict().unwrap();
    batch
}

#[test]
fn zk_receipts_require_checked_merkle_evidence_instead_of_freeform_observations() {
    let seg = fake_segment();
    let rows = synthetic_rows();
    let included = seg
        .check_transaction_inclusion(
            101,
            &rows[0],
            &MerklePath {
                tx_index: 0,
                tx_count: 1,
                siblings_raw: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(included.legacy_type_hint().unwrap().as_u8(), 2);
    let txid = Digest::parse(
        &included
            .txid_display()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    )
    .unwrap();
    let batch = one_receipt_batch(txid);
    let tlpt = synthetic_tlpt(&seg);
    let expected = ExpectedChainRange {
        chain: chain(),
        anchor: seg.anchor(),
        final_tip: seg.tip(),
    };
    let report =
        reconcile_checked_bitcoin_inclusions_unverified(&batch, &tlpt, &seg, &[included], expected)
            .unwrap();
    assert_eq!(report.root_alignment.blocks.len(), 2);
    assert_eq!(report.root_alignment.receipt_count, 1);
    assert_eq!(report.root_alignment.blocks[0].receipt_count, 1);
    assert_eq!(report.root_alignment.blocks[1].receipt_count, 0);
    assert_ne!(report.inclusion_audit_digest, [0; 32]);
    assert_eq!(
        reconcile_checked_bitcoin_inclusions_unverified(
            &batch, &tlpt, &seg, &[included], expected,
        ).unwrap(),
        report,
    );

    let mut altered = batch.clone();
    altered.receipts[0].tx_type = TransactionType::try_from(1).unwrap();
    altered.receipts[0].receipt_id = altered.receipts[0].calculate_receipt_id();
    altered.transition_root = altered.calculate_transition_root();
    altered.batch_id = altered.calculate_batch_id();
    assert!(matches!(
        reconcile_checked_bitcoin_inclusions_unverified(
            &altered,
            &tlpt,
            &seg,
            &[included],
            expected,
        ),
        Err(BridgeError::ReceiptMismatch)
    ));
    assert!(matches!(
        reconcile_checked_bitcoin_inclusions_unverified(&batch, &tlpt, &seg, &[], expected,),
        Err(BridgeError::ReceiptMismatch)
    ));
    let foreign = ExpectedChainRange {
        chain: ChainId([1; 32]),
        ..expected
    };
    assert!(matches!(
        reconcile_checked_bitcoin_inclusions_unverified(&batch, &tlpt, &seg, &[included], foreign,),
        Err(BridgeError::WrongChain)
    ));
    // The known Bitcoin genesis coinbase can prove Merkle membership but
    // cannot be promoted to a TradeLayer receipt due to no TL carrier.
    let genesis = CheckedHeader::check_bitcoin_mainnet_genesis(&header(GENESIS_HEADER)).unwrap();
    let coinbase = genesis
        .check_transaction_inclusion(
            &genesis_tx(),
            &MerklePath {
                tx_index: 0,
                tx_count: 1,
                siblings_raw: vec![],
            },
        )
        .unwrap();
    assert_eq!(coinbase.legacy_type_hint(), None);
    assert!(matches!(
        reconcile_checked_bitcoin_inclusions_unverified(&batch, &tlpt, &seg, &[coinbase], expected,),
        Err(BridgeError::MissingBlock)
    ));
}
