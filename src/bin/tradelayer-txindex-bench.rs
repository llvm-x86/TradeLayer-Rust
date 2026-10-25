//! Repeatable, LOCAL-ONLY benchmark of independent pre-execution TXID hash
//! and TL-carrier parsing. No network, wallet, full node, or state mutation.
//!
//! cargo run --release --frozen --bin tradelayer-txindex-bench -- 8000 8
//! Prints real measured results; does not assert or promise a speedup.
//! The printed synthetic TXIDs have NO Bitcoin/Litecoin chain provenance.

use std::env;
use std::process;
use std::time::{Duration, Instant};
use tradelayer_core::txindex_parallel::{
    scan_transactions_parallel_unverified, ParallelScanConfig, MAX_PARALLEL_WORKERS,
};
const MAX_SAMPLE:usize=10_000;
fn compact(n:usize,out:&mut Vec<u8>) {
    if n<=252 {out.push(n as u8);}
    else if n<=u16::MAX as usize {
        out.push(253);
        out.extend_from_slice(&(n as u16).to_le_bytes());
    } else {
        out.push(254);
        out.extend_from_slice(&(n as u32).to_le_bytes());
    }
}
fn fake(i:usize) -> Vec<u8> {
    let mut tx=vec![2,0,0,0,1];
    let mut prev=[0x88;32];
    prev[0..8].copy_from_slice(&(i as u64).to_le_bytes());
    tx.extend_from_slice(&prev);
    tx.extend_from_slice(&0u32.to_le_bytes());
    let script_len=(i%9)*180;
    compact(script_len,&mut tx);
    tx.extend(std::iter::repeat_n(0x51,script_len));
    tx.extend_from_slice(&0xffff_fffeu32.to_le_bytes());
    tx.push(1);
    tx.extend_from_slice(&0u64.to_le_bytes());
    let script = if i%200==3 {
        let mut s=vec![0x6a,15];
        s.extend_from_slice(b"tl20;alice;5;2");
        s
    } else {
        let mut s=vec![0x00,0x14];
        s.extend(std::iter::repeat_n(0x11,20));
        s
    };
    compact(script.len(),&mut tx);
    tx.extend_from_slice(&script);
    tx.extend_from_slice(&0u32.to_le_bytes());
    tx
}
fn median(mut samples:Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len()/2]
}
fn arg(args:&[String],at:usize,default:usize) -> usize {
    match args.get(at) {
        None=>default,
        Some(raw)=>raw.parse::<usize>().unwrap_or_else(|_|{
            eprintln!("invalid integer argument: {raw}");
            process::exit(2);
        })
    }
}
fn main() {
    let args=env::args().collect::<Vec<_>>();
    if args.len()>3 {
        eprintln!("usage: tradelayer-txindex-bench [1..=10000 transactions] [1..=16 workers]");
        process::exit(2);
    }
    let n=arg(&args,1,8000);
    let threads=arg(&args,2,4);
    if n==0 || n>MAX_SAMPLE || threads==0 || threads>MAX_PARALLEL_WORKERS {
        eprintln!("out of benchmark bounds (n<=10000 and workers 1..=16)");
        process::exit(2);
    }
    let txs=(0..n).map(fake).collect::<Vec<_>>();
    let serial=ParallelScanConfig::serial();
    let parallel=ParallelScanConfig {workers:threads,chunk_size:16};
    // Cold initialization is NOT included in timings. This is not a
    // reliable production throughput claim; run on the target CPU.
    let known=scan_transactions_parallel_unverified(&txs,serial)
        .expect("synthetic serial fixture");
    let warm=scan_transactions_parallel_unverified(&txs,parallel)
        .expect("synthetic parallel fixture");
    assert_eq!(known.ordered(),warm.ordered());
    let mut serial_times=Vec::new();
    let mut parallel_times=Vec::new();
    for trial in 0..5 {
        // Alternate which path runs first to reduce systematic cache bias.
        for config in if trial%2==0 {[serial,parallel]} else {[parallel,serial]} {
            let begin=Instant::now();
            let actual=scan_transactions_parallel_unverified(&txs,config)
                .expect("synthetic scanner");
            let elapsed=begin.elapsed();
            assert_eq!(actual.ordered(),known.ordered());
            if config.workers==1 {serial_times.push(elapsed);}
            else {parallel_times.push(elapsed);}
        }
    }
    let seq=median(serial_times);
    let par=median(parallel_times);
    let sec=seq.as_secs_f64();
    let pc=par.as_secs_f64();
    let count=n as f64;
    println!("transactions={n}");
    println!("total_wire_bytes={}",known.statistics().total_raw_bytes);
    println!("tl_candidate_count={}",known.statistics().carrier_candidate_transactions);
    println!("workers_requested={threads}");
    println!("workers_used={}",warm.statistics().workers_used);
    println!("serial_median_ms={:.3}",sec*1000.);
    println!("parallel_median_ms={:.3}",pc*1000.);
    println!("serial_txs_per_second={:.0}",count/sec.max(1e-9));
    println!("parallel_txs_per_second={:.0}",count/pc.max(1e-9));
    println!("observed_speedup_x={:.3}",sec/pc.max(1e-9));
    println!("consensus_admission=UNVERIFIED");
    println!("state_application=NOT_PERFORMED");
}
