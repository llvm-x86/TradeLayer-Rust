//! Stage8C: zero-copy raw block ingestion, SHA256d/Merkle observation and
//! deterministic bounded parallel preindex. No accepted Bitcoin consensus.
use std::sync::atomic::{AtomicUsize, Ordering};
use tradelayer_core::parallel_txindex::{
    inspect_unverified_block_parallel, inspect_unverified_block_slices_serial, ParallelIndexConfig,
    ParallelIndexError,
};
use tradelayer_core::raw_block_ingest::{
    fetch_and_inspect_unverified_block, merkle_root_unverified, RawBlockIngestError,
    RawBlockTrustError, UnverifiedRawBlockView, MAX_UNVERIFIED_RAW_BLOCK_BYTES,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction, ScanError};

fn fixture() -> Vec<u8> {
    decode_hex(include_str!("../fixtures/raw_block_ingest_synthetic.hex").trim()).unwrap()
}
fn raw_id(hex: &str) -> [u8; 32] {
    decode_hex(hex).unwrap().try_into().unwrap()
}
fn cfg(workers: usize, window: usize) -> ParallelIndexConfig {
    ParallelIndexConfig {
        workers,
        window_transactions: window,
    }
}
fn compact(buf: &mut Vec<u8>, number: usize) {
    if number < 253 {
        buf.push(number as u8);
    } else if number <= u16::MAX as usize {
        buf.push(0xfd);
        buf.extend_from_slice(&(number as u16).to_le_bytes());
    } else {
        buf.push(0xfe);
        buf.extend_from_slice(&(number as u32).to_le_bytes());
    }
}
fn build_block(mut transactions: Vec<Vec<u8>>) -> Vec<u8> {
    let template = fixture();
    let mut header = template[..80].to_vec();
    let txids = transactions
        .iter()
        .map(|raw| inspect_transaction(raw).unwrap().txid)
        .collect::<Vec<_>>();
    let root = merkle_root_unverified(&txids).unwrap();
    header[36..68].copy_from_slice(&root);
    compact(&mut header, transactions.len());
    for tx in transactions.drain(..) {
        header.extend_from_slice(&tx);
    }
    header
}
fn synthetic_block(count: usize) -> Vec<u8> {
    let raw = fixture();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let coinbase = view.raw_transaction(0).unwrap().to_vec();
    let legacy = view.raw_transaction(1).unwrap();
    let witness = view.raw_transaction(2).unwrap();
    let mut txs = Vec::with_capacity(count);
    txs.push(coinbase);
    for index in 1..count {
        let is_witness = index % 2 == 0;
        let mut tx = if is_witness {
            witness.to_vec()
        } else {
            legacy.to_vec()
        };
        let prevout = if is_witness { 7 } else { 5 };
        tx[prevout..prevout + 8].copy_from_slice(&(index as u64).to_le_bytes());
        txs.push(tx);
    }
    build_block(txs)
}

#[test]
fn independent_four_transaction_hashes_merkle_weight_and_borrowed_pointers() {
    let raw = fixture();
    let expected_hash = raw_id("09667b766e1bccf918412b65f62966d7a2803613592fed34ba9d27ac608eb0c0");
    let expected_merkle =
        raw_id("760e6d8636fd5304a65989b461a27a88f48545bc1cc6e2b4c2016a638d2e8a9c");
    let view = UnverifiedRawBlockView::parse(&raw, Some(expected_hash)).unwrap();
    assert_eq!(view.source_block_bytes().as_ptr(), raw.as_ptr());
    assert_eq!(view.transaction_count(), 4);
    assert_eq!(view.header_hash_raw(), expected_hash);
    assert_eq!(view.header_merkle_root_raw(), expected_merkle);
    assert_eq!(view.stripped_block_bytes(), 357);
    assert_eq!(view.block_weight(), 1441);
    assert_eq!(view.transaction_ranges()[0].start, 81);
    let mut last = 81;
    for (index, span) in view.transaction_ranges().iter().enumerate() {
        assert_eq!(span.start, last);
        assert!(span.end > span.start);
        let borrowed = view.raw_transaction(index).unwrap();
        assert_eq!(borrowed.as_ptr(), raw[span.start..].as_ptr());
        assert_eq!(borrowed.len(), span.end - span.start);
        last = span.end;
    }
    assert_eq!(last, raw.len());
    assert!(view.raw_transaction(4).is_none());
    let serial = view.inspect_serial().unwrap();
    let parallel = view.inspect_parallel(cfg(4, 2)).unwrap();
    assert_eq!(parallel, serial);
    assert_eq!(serial.header_hash_raw(), expected_hash);
    assert_eq!(serial.txid_merkle_root_raw(), expected_merkle);
    assert_eq!(serial.ordered_preindex().transaction_count(), 4);
    assert_eq!(serial.ordered_preindex().candidate_positions(), &[1, 2]);
    assert_eq!(
        serial.ordered_preindex().ordered()[0].observation.txid,
        raw_id("8647c745029df431acac0ff4f8a91ff00d6194ef87a705946f784af9f213bffa")
    );
    assert_eq!(
        serial.ordered_preindex().ordered()[2].observation.txid,
        raw_id("f878d360c2a1b812ef5aa5b2706f0f56aa34dba00b4f40fe07000571f00208b1")
    );
    assert_eq!(
        serial.ordered_preindex().ordered()[2].observation.wtxid,
        raw_id("087f19b8125e0a640993f6add2ee73bf2f2189edb5e2f6f4d9a64e36b4f553df")
    );
    assert_eq!(
        view.require_full_bitcoin_consensus(),
        Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified)
    );
    assert_eq!(
        serial.require_authenticated_block(),
        Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified)
    );
    assert_eq!(
        serial.require_script_and_utxo_consensus(),
        Err(RawBlockTrustError::ScriptAndUtxoStateUnverified)
    );
    assert_eq!(
        serial.require_tradelayer_state_transition(),
        Err(RawBlockTrustError::TradeLayerStateAndSignedRevisionUnverified)
    );
}

#[test]
fn one_mock_bulk_rpc_request_is_enough_for_entire_four_transaction_preindex() {
    let raw = fixture();
    let expected = raw_id("09667b766e1bccf918412b65f62966d7a2803613592fed34ba9d27ac608eb0c0");
    let calls = AtomicUsize::new(0);
    let output = fetch_and_inspect_unverified_block(expected, cfg(4, 64), |hash| {
        assert_eq!(hash, expected);
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(raw.clone())
    })
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(output.ordered_preindex().transaction_count(), 4);
    assert_eq!(
        output.require_authenticated_block(),
        Err(RawBlockTrustError::ActiveChainAndFullConsensusUnverified)
    );
    let wrong = fetch_and_inspect_unverified_block([3; 32], cfg(2, 64), |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(raw.clone())
    });
    assert_eq!(wrong, Err(RawBlockIngestError::RequestedHeaderHashMismatch));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        fetch_and_inspect_unverified_block(expected, cfg(2, 64), |_| Err(
            "offline fake RPC refused".into()
        )),
        Err(RawBlockIngestError::FetchFailed(
            "offline fake RPC refused".into()
        ))
    );
}

#[test]
fn canonical_count_boundary_errors_are_explicit_and_transaction_indexed() {
    let raw = fixture();
    assert!(matches!(
        UnverifiedRawBlockView::parse(&raw[..40], None),
        Err(RawBlockIngestError::TruncatedBlockHeader)
    ));
    assert_eq!(
        UnverifiedRawBlockView::parse(&raw[..80], None).err(),
        Some(RawBlockIngestError::MissingCompactSize(
            ScanError::Truncated
        ))
    );
    let mut zero = raw.clone();
    zero[80] = 0;
    assert_eq!(
        UnverifiedRawBlockView::parse(&zero, None).err(),
        Some(RawBlockIngestError::ZeroTransactions)
    );
    let mut excessive = raw.clone();
    excessive.splice(80..81, [0xfe, 0x41, 0x0d, 0x03, 0x00]); // 200001
    assert_eq!(
        UnverifiedRawBlockView::parse(&excessive, None).err(),
        Some(RawBlockIngestError::TooManyTransactions)
    );
    let mut noncanonical = raw.clone();
    noncanonical.splice(80..81, [0xfd, 0x04, 0x00]);
    assert_eq!(
        UnverifiedRawBlockView::parse(&noncanonical, None).err(),
        Some(RawBlockIngestError::MissingCompactSize(
            ScanError::NonCanonicalCompactSize
        ))
    );
    let mut short_count = raw.clone();
    short_count[80] = 3;
    assert_eq!(
        UnverifiedRawBlockView::parse(&short_count, None).err(),
        Some(RawBlockIngestError::TrailingRawBlockData)
    );
    let mut long_count = raw.clone();
    long_count[80] = 5;
    assert_eq!(
        UnverifiedRawBlockView::parse(&long_count, None).err(),
        Some(RawBlockIngestError::TransactionBoundary {
            index: 4,
            source: ScanError::Truncated,
        })
    );
    let spans = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let mut truncated = raw[..spans.transaction_ranges()[2].start + 15].to_vec();
    truncated[80] = 3;
    assert_eq!(
        UnverifiedRawBlockView::parse(&truncated, None).err(),
        Some(RawBlockIngestError::TransactionBoundary {
            index: 2,
            source: ScanError::Truncated,
        })
    );
    let mut wrong_segwit = raw.clone();
    wrong_segwit[spans.transaction_ranges()[2].start + 5] = 0x02;
    assert!(matches!(
        UnverifiedRawBlockView::parse(&wrong_segwit, None),
        Err(RawBlockIngestError::TransactionBoundary { index: 2, .. })
    ));
    let mut bad_scriptsig = raw.clone();
    let input_count_position = spans.transaction_ranges()[1].start + 4;
    bad_scriptsig.splice(input_count_position..input_count_position + 1, [0xfd, 1, 0]);
    assert_eq!(
        UnverifiedRawBlockView::parse(&bad_scriptsig, None).err(),
        Some(RawBlockIngestError::TransactionBoundary {
            index: 1,
            source: ScanError::NonCanonicalCompactSize,
        })
    );
    let mut too_large = raw;
    too_large.resize(MAX_UNVERIFIED_RAW_BLOCK_BYTES + 1, 0);
    assert_eq!(
        UnverifiedRawBlockView::parse(&too_large, None).err(),
        Some(RawBlockIngestError::OversizedRawBlock)
    );
}

#[test]
fn header_hash_merkle_mutation_and_duplicate_txids_fail_closed() {
    let raw = fixture();
    let expected = raw_id("09667b766e1bccf918412b65f62966d7a2803613592fed34ba9d27ac608eb0c0");
    let mut corrupt = raw.clone();
    corrupt[36] ^= 1; // Merkle root in header no longer matches actual TXIDs
    assert_eq!(
        UnverifiedRawBlockView::parse(&corrupt, Some(expected)).err(),
        Some(RawBlockIngestError::RequestedHeaderHashMismatch)
    );
    let view = UnverifiedRawBlockView::parse(&corrupt, None).unwrap();
    assert_eq!(
        view.inspect_parallel(cfg(4, 4)),
        Err(RawBlockIngestError::HeaderMerkleRootMismatch)
    );
    let original = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let tx0 = original.raw_transaction(0).unwrap().to_vec();
    let tx1 = original.raw_transaction(1).unwrap().to_vec();
    let duplicate = {
        let mut d = raw[..80].to_vec();
        d.push(3);
        d.extend_from_slice(&tx0);
        d.extend_from_slice(&tx1);
        d.extend_from_slice(&tx1);
        d
    };
    assert_eq!(
        UnverifiedRawBlockView::parse(&duplicate, None)
            .unwrap()
            .inspect_parallel(cfg(4, 64)),
        Err(RawBlockIngestError::TransactionPreindex(
            ParallelIndexError::DuplicateTxid {
                first: 1,
                second: 2
            }
        ))
    );
    assert_eq!(
        merkle_root_unverified(&[[1; 32], [1; 32]]),
        Err(RawBlockIngestError::InvalidTxidMerkleTree)
    );
}

fn oversized_weight_transaction(index: u32) -> Vec<u8> {
    let mut tx = Vec::new();
    tx.extend_from_slice(&2u32.to_le_bytes());
    tx.push(1);
    let mut parent = [0u8; 32];
    parent[..4].copy_from_slice(&index.to_le_bytes());
    tx.extend_from_slice(&parent);
    tx.extend_from_slice(&0u32.to_le_bytes());
    compact(&mut tx, 240_000);
    tx.extend(std::iter::repeat_n(0x42u8, 240_000));
    tx.extend_from_slice(&0xffff_fffeu32.to_le_bytes());
    tx.push(1);
    tx.extend_from_slice(&0u64.to_le_bytes());
    tx.extend_from_slice(&[1, 0x51]);
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx
}

#[test]
fn block_weight_caps_base_bytes_not_only_serialized_bytes() {
    let original = fixture();
    let mut block = original[..80].to_vec();
    compact(&mut block, 5);
    for i in 0..5 {
        block.extend_from_slice(&oversized_weight_transaction(i));
    }
    assert!(block.len() < MAX_UNVERIFIED_RAW_BLOCK_BYTES);
    assert_eq!(
        UnverifiedRawBlockView::parse(&block, None).err(),
        Some(RawBlockIngestError::BlockWeightExceeded)
    );
}

#[test]
fn ten_thousand_mixed_transactions_exactly_match_old_owned_vec_parallel_path() {
    let raw = synthetic_block(10_000);
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    assert_eq!(view.transaction_count(), 10_000);
    assert!(view.block_weight() <= 4_000_000);
    let borrowed = view
        .transaction_ranges()
        .iter()
        .map(|r| &raw[r.clone()])
        .collect::<Vec<_>>();
    let owned = borrowed.iter().map(|t| t.to_vec()).collect::<Vec<_>>();
    let old = inspect_unverified_block_parallel(&owned, cfg(4, 256)).unwrap();
    let slice_reference = inspect_unverified_block_slices_serial(&borrowed).unwrap();
    assert_eq!(old, slice_reference);
    assert!(old.candidate_positions().len() > 4000);
    for (workers, window) in [(1, 1), (2, 64), (4, 256), (8, 512), (16, 512)] {
        let new_result = view.inspect_parallel(cfg(workers, window)).unwrap();
        assert_eq!(*new_result.ordered_preindex(), old);
        assert_eq!(
            new_result.txid_merkle_root_raw(),
            view.header_merkle_root_raw()
        );
        assert_eq!(
            new_result.require_tradelayer_state_transition(),
            Err(RawBlockTrustError::TradeLayerStateAndSignedRevisionUnverified)
        );
    }
}
