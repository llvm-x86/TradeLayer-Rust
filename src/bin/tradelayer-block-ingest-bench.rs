//! Stage8C: synthetic raw-block preindex benchmark, NO Bitcoin consensus.
//! One complete raw block is scanned using borrowed slices; reference path
//! constructs one owned Vec per transaction and uses the same CPU workers.
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::hint::black_box;
use std::time::{Duration, Instant};
use tradelayer_core::parallel_txindex::{inspect_unverified_block_parallel, ParallelIndexConfig};
use tradelayer_core::raw_block_ingest::{
    fetch_and_inspect_unverified_block, merkle_root_unverified, merkle_root_unverified_inplace,
    UnverifiedRawBlockView,
};
use tradelayer_core::read_only::{decode_hex, inspect_transaction};

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
fn raw_sample() -> Vec<u8> {
    decode_hex(include_str!("../../fixtures/raw_block_ingest_synthetic.hex").trim()).unwrap()
}
fn make_block(count: usize, witness_percent: usize) -> Vec<u8> {
    let raw = raw_sample();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let coinbase = view.raw_transaction(0).unwrap().to_vec();
    let legacy = view.raw_transaction(1).unwrap();
    let witness = view.raw_transaction(2).unwrap();
    let mut transactions = Vec::with_capacity(count);
    transactions.push(coinbase);
    for i in 1..count {
        let uses_witness = (i * 37) % 100 < witness_percent;
        let mut tx = if uses_witness {
            witness.to_vec()
        } else {
            legacy.to_vec()
        };
        let prevout = if uses_witness { 7 } else { 5 };
        tx[prevout..prevout + 8].copy_from_slice(&(i as u64).to_le_bytes());
        transactions.push(tx);
    }
    let txids = transactions
        .iter()
        .map(|raw| inspect_transaction(raw).unwrap().txid)
        .collect::<Vec<_>>();
    let mut block = raw[..80].to_vec();
    block[36..68].copy_from_slice(&merkle_root_unverified(&txids).unwrap());
    compact(&mut block, count);
    for tx in transactions {
        block.extend_from_slice(&tx);
    }
    block
}
fn median(mut durations: Vec<Duration>) -> Duration {
    durations.sort_unstable();
    durations[durations.len() / 2]
}
/// Measure the same source-order first/second duplicate rule without hashing.
fn duplicate_tree(ids: &[[u8; 32]]) -> Result<(), (usize, usize)> {
    let mut seen = BTreeMap::new();
    for (index, &txid) in ids.iter().enumerate() {
        if let Some(&first) = seen.get(&txid) {
            return Err((first, index));
        }
        seen.insert(txid, index);
    }
    Ok(())
}

fn duplicate_hash(ids: &[[u8; 32]]) -> Result<(), (usize, usize)> {
    let mut seen = HashMap::with_capacity(ids.len());
    for (index, &txid) in ids.iter().enumerate() {
        match seen.entry(txid) {
            std::collections::hash_map::Entry::Occupied(found) => {
                return Err((*found.get(), index));
            }
            std::collections::hash_map::Entry::Vacant(vacant) => {
                vacant.insert(index);
            }
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() > 5 {
        panic!("usage: tradelayer-block-ingest-bench [tx_count:1..10000] [workers:1..16] [witness_percent:0..100] [repeats:1..5]");
    }
    let parse = |i: usize, default: usize| -> usize {
        args.get(i).map_or(default, |text| {
            text.parse().expect("valid integer argument")
        })
    };
    let count = parse(1, 4000);
    let workers = parse(2, 4);
    let witness_percent = parse(3, 50);
    let repeats = parse(4, 3);
    assert!((1..=10000).contains(&count));
    assert!((1..=16).contains(&workers));
    assert!(witness_percent <= 100);
    assert!((1..=5).contains(&repeats));
    let config = ParallelIndexConfig {
        workers,
        window_transactions: 512,
    };
    let raw = make_block(count, witness_percent);
    let view = UnverifiedRawBlockView::parse(&raw, None).expect("bounded raw block");
    let old_vec = view
        .transaction_ranges()
        .iter()
        .map(|span| raw[span.clone()].to_vec())
        .collect::<Vec<_>>();
    let old = inspect_unverified_block_parallel(&old_vec, config).unwrap();
    let new = view.inspect_parallel(config).unwrap();
    assert_eq!(old, *new.ordered_preindex());
    let one_team = view.inspect_parallel_one_team(config).unwrap();
    assert_eq!(one_team, new);
    let hash_seen = view.inspect_parallel_one_team_hash_seen(config).unwrap();
    let inplace_merkle = view.inspect_parallel_one_team_inplace_merkle(config).unwrap();
    let fast_reduce = view.inspect_parallel_one_team_fast_reduce(config).unwrap();
    assert_eq!(hash_seen, one_team);
    assert_eq!(inplace_merkle, one_team);
    assert_eq!(fast_reduce, one_team);
    let observed_txids = one_team
        .ordered_preindex()
        .ordered()
        .iter()
        .map(|row| row.observation.txid)
        .collect::<Vec<_>>();
    assert_eq!(duplicate_tree(&observed_txids), Ok(()));
    assert_eq!(duplicate_hash(&observed_txids), Ok(()));
    assert_eq!(
        merkle_root_unverified(&observed_txids),
        merkle_root_unverified_inplace(&observed_txids)
    );
    let mut owned_times = Vec::new();
    let mut full_owned_times = Vec::new();
    let mut prepared_times = Vec::new();
    let mut one_team_times = Vec::new();
    let mut hash_seen_times = Vec::new();
    let mut inplace_merkle_times = Vec::new();
    let mut fast_reduce_times = Vec::new();
    let mut tree_index_times = Vec::new();
    let mut hash_index_times = Vec::new();
    let mut merkle_reference_times = Vec::new();
    let mut merkle_inplace_times = Vec::new();
    let mut bulk_times = Vec::new();
    for _ in 0..repeats {
        let start = Instant::now();
        let owned: Vec<_> = view
            .transaction_ranges()
            .iter()
            .map(|r| raw[r.clone()].to_vec())
            .collect();
        let serial_by_slot = inspect_unverified_block_parallel(&owned, config).unwrap();
        owned_times.push(start.elapsed());
        black_box(&serial_by_slot);
        // Equal-work comparison: both paths clone the single fetched raw
        // block, check its header, discover TX byte ranges and calculate
        // the TXID Merkle. Only owned-copy/reparse versus prepared-fused
        // worker processing should differ meaningfully.
        let start = Instant::now();
        let full = raw.clone();
        let full_view = UnverifiedRawBlockView::parse(&full, Some(view.header_hash_raw())).unwrap();
        let full_owned = full_view
            .transaction_ranges()
            .iter()
            .map(|range| full[range.clone()].to_vec())
            .collect::<Vec<_>>();
        let full_result = inspect_unverified_block_parallel(&full_owned, config).unwrap();
        let old_root = merkle_root_unverified(
            &full_result
                .ordered()
                .iter()
                .map(|row| row.observation.txid)
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(old_root, full_view.header_merkle_root_raw());
        assert_eq!(full_result, serial_by_slot);
        full_owned_times.push(start.elapsed());
        black_box(full_result);
        let start = Instant::now();
        let prepared_only = view.inspect_parallel(config).unwrap();
        prepared_times.push(start.elapsed());
        assert_eq!(*prepared_only.ordered_preindex(), serial_by_slot);
        black_box(prepared_only);
        let start = Instant::now();
        let pooled = view.inspect_parallel_one_team(config).unwrap();
        one_team_times.push(start.elapsed());
        assert_eq!(*pooled.ordered_preindex(), serial_by_slot);
        black_box(pooled);
        let start = Instant::now();
        let hash_only = view.inspect_parallel_one_team_hash_seen(config).unwrap();
        hash_seen_times.push(start.elapsed());
        assert_eq!(*hash_only.ordered_preindex(), serial_by_slot);
        black_box(hash_only);
        let start = Instant::now();
        let merkle_only = view.inspect_parallel_one_team_inplace_merkle(config).unwrap();
        inplace_merkle_times.push(start.elapsed());
        assert_eq!(*merkle_only.ordered_preindex(), serial_by_slot);
        black_box(merkle_only);
        let start = Instant::now();
        let combined = view.inspect_parallel_one_team_fast_reduce(config).unwrap();
        fast_reduce_times.push(start.elapsed());
        assert_eq!(*combined.ordered_preindex(), serial_by_slot);
        black_box(combined);
        let start = Instant::now();
        let observed =
            fetch_and_inspect_unverified_block(view.header_hash_raw(), config, |_| Ok(raw.clone()))
                .unwrap();
        bulk_times.push(start.elapsed());
        assert_eq!(*observed.ordered_preindex(), serial_by_slot);
        black_box(observed);
    }
    // Isolate index and Merkle reducer costs from thread scheduling and
    // SHA256d transaction hashing. Use the exact same source-ordered IDs.
    for _ in 0..repeats * 7 {
        let start = Instant::now();
        let baseline_index = duplicate_tree(&observed_txids);
        tree_index_times.push(start.elapsed());
        assert_eq!(baseline_index, Ok(()));
        black_box(baseline_index);

        let start = Instant::now();
        let hashed_index = duplicate_hash(&observed_txids);
        hash_index_times.push(start.elapsed());
        assert_eq!(hashed_index, Ok(()));
        black_box(hashed_index);

        let start = Instant::now();
        let root = merkle_root_unverified(&observed_txids).unwrap();
        merkle_reference_times.push(start.elapsed());
        assert_eq!(root, view.header_merkle_root_raw());
        black_box(root);

        let start = Instant::now();
        let root = merkle_root_unverified_inplace(&observed_txids).unwrap();
        merkle_inplace_times.push(start.elapsed());
        assert_eq!(root, view.header_merkle_root_raw());
        black_box(root);
    }
    let baseline_ms = median(owned_times).as_secs_f64() * 1000.0;
    let full_owned_ms = median(full_owned_times).as_secs_f64() * 1000.0;
    let prepared_ms = median(prepared_times).as_secs_f64() * 1000.0;
    let one_team_ms = median(one_team_times).as_secs_f64() * 1000.0;
    let hash_seen_ms = median(hash_seen_times).as_secs_f64() * 1000.0;
    let inplace_merkle_ms = median(inplace_merkle_times).as_secs_f64() * 1000.0;
    let fast_reduce_ms = median(fast_reduce_times).as_secs_f64() * 1000.0;
    let tree_index_ms = median(tree_index_times).as_secs_f64() * 1000.0;
    let hash_index_ms = median(hash_index_times).as_secs_f64() * 1000.0;
    let merkle_reference_ms = median(merkle_reference_times).as_secs_f64() * 1000.0;
    let merkle_inplace_ms = median(merkle_inplace_times).as_secs_f64() * 1000.0;
    let bulk_ms = median(bulk_times).as_secs_f64() * 1000.0;
    let bulk_seconds = (bulk_ms / 1000.0).max(1e-9);
    println!("benchmark=UNVERIFIED_SYNTHETIC_BULK_BLOCK_INGEST");
    println!("transaction_count={count}");
    println!("witness_percent={witness_percent}");
    println!("workers={workers}");
    println!("raw_block_bytes={}", raw.len());
    println!("declared_weight={}", view.block_weight());
    println!("reference_owned_tx_vectors={count}");
    println!("new_raw_tx_byte_copies=0");
    println!("bulk_mock_rpc_calls_per_iteration=1");
    println!("legacy_per_tx_mock_rpc_calls_if_fetched_individually={count}");
    println!("owned_vector_parallel_median_ms={baseline_ms:.3}");
    println!("full_owned_block_pipeline_median_ms={full_owned_ms:.3}");
    println!("fused_prepared_only_median_ms={prepared_ms:.3}");
    println!("one_team_prepared_only_median_ms={one_team_ms:.3}");
    println!("one_team_hashed_prepared_only_median_ms={hash_seen_ms:.3}");
    println!("one_team_inplace_merkle_prepared_only_median_ms={inplace_merkle_ms:.3}");
    println!("one_team_fast_reduce_prepared_only_median_ms={fast_reduce_ms:.3}");
    println!(
        "fast_reduce_vs_one_team_ratio={:.3}",
        one_team_ms / fast_reduce_ms.max(1e-9)
    );
    println!("duplicate_btree_median_ms={tree_index_ms:.3}");
    println!("duplicate_hashmap_median_ms={hash_index_ms:.3}");
    println!(
        "duplicate_hashmap_vs_btree_ratio={:.3}",
        tree_index_ms / hash_index_ms.max(1e-9)
    );
    println!("merkle_allocating_median_ms={merkle_reference_ms:.3}");
    println!("merkle_inplace_median_ms={merkle_inplace_ms:.3}");
    println!(
        "merkle_inplace_vs_allocating_ratio={:.3}",
        merkle_reference_ms / merkle_inplace_ms.max(1e-9)
    );
    println!(
        "one_team_vs_windowed_ratio={:.3}",
        prepared_ms / one_team_ms.max(1e-9)
    );
    println!(
        "one_team_extra_result_slots={}",
        if workers > 1 { count } else { 0 }
    );
    println!("bulk_borrowed_parallel_median_ms={bulk_ms:.3}");
    println!(
        "full_pipeline_speed_ratio={:.3}",
        full_owned_ms / bulk_ms.max(1e-9)
    );
    println!("measured_ratio={:.3}", baseline_ms / bulk_ms.max(1e-9));
    println!(
        "bulk_preindex_txs_per_sec={:.0}",
        count as f64 / bulk_seconds
    );
    println!("ordered_equivalence=PASS");
    println!("txid_merkle=LOCAL_CHECK_ONLY");
    println!("full_block_consensus=UNVERIFIED");
    println!("tradelayer_state=UNVERIFIED");
}
