//! Offline CPU benchmark for a locally supplied Bitcoin Core getblock(hash,0)
//! hex file. No network, wallet, consensus authority, external crates or unsafe.
use std::env;
use std::fs;
use std::hint::black_box;
use std::time::{Duration, Instant};
use tradelayer_core::parallel_txindex::ParallelIndexConfig;
use tradelayer_core::raw_block_ingest::UnverifiedRawBlockView;
use tradelayer_core::read_only::{decode_hex, id_hex};

const MAX_HEX_FILE_BYTES: u64 = 8_100_000;

fn median(mut values: Vec<Duration>) -> f64 {
    values.sort_unstable();
    values[values.len() / 2].as_secs_f64() * 1_000.0
}

fn peak_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmHWM:")?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
    })
}

fn main() {
    let args: Vec<String> = env::args().collect();
    assert!(
        args.len() == 5,
        "usage: tradelayer-real-block-reduce-bench block.hex workers:1..16 repeats:1..20 tree|hash"
    );
    let path = &args[1];
    let workers: usize = args[2].parse().expect("integer worker count");
    let repeats: usize = args[3].parse().expect("integer repetitions");
    let variant = args[4].as_str();
    assert!((1..=16).contains(&workers));
    assert!((1..=20).contains(&repeats));
    assert!(variant == "tree" || variant == "hash");
    let len = fs::metadata(path).expect("local raw block hex file").len();
    assert!(
        len <= MAX_HEX_FILE_BYTES,
        "hex file exceeds research input bound"
    );
    let contents = fs::read_to_string(path).expect("UTF-8 hex file");
    let bytes = decode_hex(contents.trim()).expect("canonical raw-block hex");
    let view = UnverifiedRawBlockView::parse(&bytes, None)
        .expect("bounded raw block with valid transaction syntax");
    let config = ParallelIndexConfig {
        workers,
        window_transactions: 512,
    };
    let scan = || match variant {
        "tree" => view.inspect_parallel_one_team_tree_reference(config),
        "hash" => view.inspect_parallel_one_team(config),
        _ => unreachable!("validated variant"),
    };

    // Warm the selected read-only path before timing. The same amount of
    // retained state and transaction-byte parsing is used in both processes.
    let warm = scan().expect("local TXID/Merkle parity");
    assert_eq!(warm.txid_merkle_root_raw(), view.header_merkle_root_raw());
    let count = warm.ordered_preindex().transaction_count();
    let witnessed = warm
        .ordered_preindex()
        .ordered()
        .iter()
        .filter(|row| row.observation.has_witness)
        .count();
    let carriers = warm.ordered_preindex().candidate_positions().len();
    black_box(warm);

    let mut times = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let start = Instant::now();
        let result = scan().expect("repeated ordered preindex");
        let duration = start.elapsed();
        assert_eq!(result.ordered_preindex().transaction_count(), count);
        assert_eq!(result.txid_merkle_root_raw(), view.header_merkle_root_raw());
        assert_eq!(
            result.ordered_preindex().candidate_positions().len(),
            carriers
        );
        times.push(duration);
        black_box(result);
    }

    println!("benchmark=UNVERIFIED_OPERATOR_SUPPLIED_RAW_BLOCK");
    println!("variant={variant}");
    println!("source=LOCAL_BITCOIN_CORE_HEX_FILE");
    println!("block_hash={}", id_hex(&view.header_hash_raw()));
    println!("transaction_count={count}");
    println!("segwit_transactions={witnessed}");
    println!("candidate_transactions={carriers}");
    println!("raw_block_bytes={}", bytes.len());
    println!("block_weight={}", view.block_weight());
    println!("workers={workers}");
    println!("repetitions={repeats}");
    println!("prepared_only_median_ms={:.3}", median(times));
    match peak_rss_kib() {
        Some(value) => println!("process_peak_rss_kib={value}"),
        None => println!("process_peak_rss_kib=UNAVAILABLE"),
    }
    println!("ordered_source=CHECKED_LOCALLY");
    println!("txid_merkle=LOCAL_CHECK_ONLY");
    println!("full_bitcoin_consensus=UNVERIFIED");
    println!("tradelayer_state=UNVERIFIED");
}
