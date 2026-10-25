//! Offline runtime benchmark of serial and bounded parallel read-only
//! txid/OP_RETURN indexing, using synthetic Bitcoin-style byte arrays.
//! No real node, UTXOs, signatures, claims of chain validity or writes.
use std::env;
use std::hint::black_box;
use std::process;
use std::time::{Duration, Instant};
use tradelayer_core::parallel_txindex::{
    inspect_unverified_block_parallel, inspect_unverified_block_serial, ParallelIndexConfig,
    MAX_BLOCK_INPUT_BYTES, MAX_PARALLEL_WORKERS,
};

fn compact(dst: &mut Vec<u8>, n: usize) {
    if n < 253 {
        dst.push(n as u8);
    } else {
        dst.push(253);
        dst.extend_from_slice(&(n as u16).to_le_bytes());
    }
}
fn synthetic_tx(position: usize, pad: usize, segwit: bool) -> Vec<u8> {
    let mut raw = Vec::with_capacity(pad + 120);
    raw.extend_from_slice(&2u32.to_le_bytes());
    if segwit {
        raw.extend_from_slice(&[0x00, 0x01]); // SegWit marker and flag
    }
    raw.push(1);
    let mut parent = [0u8; 32];
    parent[..8].copy_from_slice(&(position as u64).to_le_bytes());
    parent[8] = 0x77;
    raw.extend_from_slice(&parent);
    raw.extend_from_slice(&0u32.to_le_bytes());
    compact(&mut raw, pad);
    for n in 0..pad {
        raw.push((position as u8).wrapping_add((n as u8).wrapping_mul(17)));
    }
    raw.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
    raw.push(1);
    raw.extend_from_slice(&0u64.to_le_bytes());
    let script = if position.is_multiple_of(97) {
        &b"\x6a\x0etl20;alice;5;2"[..]
    } else {
        &b"\x51\x21\x02\x00\x00\x00\x00"[..]
    };
    compact(&mut raw, script.len());
    raw.extend_from_slice(script);
    if segwit {
        raw.extend_from_slice(&[1, 2, 0x30, 0x01]); // one nonempty witness item
    }
    raw.extend_from_slice(&0u32.to_le_bytes());
    raw
}
fn median(mut values: Vec<Duration>) -> Duration {
    values.sort_unstable();
    values[values.len() / 2]
}
fn usage() -> ! {
    eprintln!(
        "usage: tradelayer-index-bench [transactions:1..20000] [scriptSig_bytes:0..2048] [workers:1..16] [window:1..512] [repeats:1..10] [segwit_percent:0..100]"
    );
    process::exit(2)
}
fn argument<T: std::str::FromStr>(args: &[String], index: usize, default: T) -> T {
    match args.get(index) {
        None => default,
        Some(raw) => raw.parse().ok().unwrap_or_else(|| usage()),
    }
}

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() > 7 {
        usage();
    }
    let count: usize = argument(&args, 1, 2500);
    let pad: usize = argument(&args, 2, 512);
    let workers: usize = argument(&args, 3, ParallelIndexConfig::default().workers);
    let window: usize = argument(&args, 4, 256);
    let repeats: usize = argument(&args, 5, 3);
    let segwit_percent: usize = argument(&args, 6, 0);
    if !(1..=20_000).contains(&count)
        || pad > 2048
        || !(1..=10).contains(&repeats)
        || !(1..=MAX_PARALLEL_WORKERS).contains(&workers)
        || segwit_percent > 100
    {
        usage();
    }
    let config = ParallelIndexConfig {
        workers,
        window_transactions: window,
    };
    if config.validate().is_err() {
        usage();
    }
    let raws: Vec<_> = (0..count)
        .map(|i| synthetic_tx(i, pad + i % 7, (i * 37) % 100 < segwit_percent))
        .collect();
    let total_bytes: usize = raws.iter().map(Vec::len).sum();
    if total_bytes > MAX_BLOCK_INPUT_BYTES {
        usage();
    }
    // Two warmups; exclude cold startup/shim overhead from measurements.
    let baseline = inspect_unverified_block_serial(&raws).expect("synthetic serial");
    let first_parallel =
        inspect_unverified_block_parallel(&raws, config).expect("synthetic parallel");
    assert_eq!(baseline, first_parallel, "parallel changed indexed results");
    black_box(&baseline);
    black_box(&first_parallel);
    let mut serial_times = Vec::new();
    let mut parallel_times = Vec::new();
    for _ in 0..repeats {
        let start = Instant::now();
        let serial = inspect_unverified_block_serial(&raws).expect("synthetic serial repeat");
        serial_times.push(start.elapsed());
        black_box(serial);
        let start = Instant::now();
        let parallel =
            inspect_unverified_block_parallel(&raws, config).expect("synthetic parallel repeat");
        parallel_times.push(start.elapsed());
        assert_eq!(parallel, baseline, "nondeterministic block source results");
        black_box(parallel);
    }
    let serial = median(serial_times).as_secs_f64().max(1e-9);
    let parallel = median(parallel_times).as_secs_f64().max(1e-9);
    println!("benchmark=UNVERIFIED_SYNTHETIC_TXID_SCAN");
    println!("transaction_count={count}");
    println!("segwit_percent={segwit_percent}");
    println!("witness_transactions={}", baseline.ordered().iter().filter(|tx| tx.observation.has_witness).count());
    println!("raw_bytes={total_bytes}");
    println!("candidate_count={}", baseline.candidate_positions().len());
    println!("workers={workers}");
    println!("window_transactions={window}");
    println!("repeats={repeats}");
    println!("serial_median_ms={:.3}", serial * 1_000.0);
    println!("parallel_median_ms={:.3}", parallel * 1_000.0);
    println!("measured_speedup={:.3}", serial / parallel);
    println!("parallel_txs_per_sec={:.0}", count as f64 / parallel);
    println!("ordered_equivalence=PASS");
    println!("full_block_consensus=UNVERIFIED");
    println!("historical_signer_and_ledger=UNVERIFIED");
}
