//! Read-only Bitcoin *mainnet* header chainwork experiment from published
//! genesis. Validates SHA256d header PoW, mainnet difficulty, MTP and
//! cumulative work. DOES NOT prove active best chain, full-block validity,
//! UTXO/scripts, future-time eligibility, or economic correctness.
//!
//! Input: one lowercase 160-hex-byte 80-byte header per line.
//! Line 1 must be the genuine Bitcoin-mainnet genesis header.
//! Up to 500 subsequent headers per CLI invocation.
use std::fs::File;
use std::io::Read;
use std::path::Path;

use tradelayer_core::bitcoin_chainwork::MainnetWorkChain;
use tradelayer_core::read_only::decode_hex;

const MAX_TOTAL_LINES: usize = 501;
const MAX_FILE_BYTES: usize = MAX_TOTAL_LINES * 161;

fn canonical_header(text: &str) -> Result<[u8; 80], String> {
    if text.len() != 160
        || !text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("expected lowercase 160-character header hex".into());
    }
    decode_hex(text)
        .map_err(|err| format!("header: {err}"))?
        .try_into()
        .map_err(|_| "header length not 80 bytes".to_owned())
}
fn to_hex(input: &[u8]) -> String {
    input.iter().map(|value| format!("{value:02x}")).collect()
}
fn read_bounded(path: &Path) -> Result<String, String> {
    let f = File::open(path).map_err(|err| format!("{err}"))?;
    if f.metadata().map_err(|err| format!("{err}"))?.len() > MAX_FILE_BYTES as u64 {
        return Err("input too long".into());
    }
    let mut bytes = Vec::new();
    f.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("{err}"))?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("input too long".into());
    }
    String::from_utf8(bytes).map_err(|_| "input is not UTF-8".into())
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 2 {
        return Err("usage: tradelayer-bitcoin-work <genesis_then_headers.txt>".into());
    }
    let data = read_bounded(Path::new(&args[1]))?;
    let mut lines = data.lines();
    let genesis = canonical_header(lines.next().ok_or("missing genesis header")?)?;
    let mut chain = MainnetWorkChain::from_published_genesis(&genesis)
        .map_err(|err| format!("genesis rejected: {err}"))?;
    let mut headers = Vec::new();
    for line in lines {
        if headers.len() >= MAX_TOTAL_LINES - 1 {
            return Err("more than 500 continuation headers".into());
        }
        headers.push(canonical_header(line)?);
    }
    if !headers.is_empty() {
        chain.extend_checked(&headers)
            .map_err(|err| format!("header segment rejected: {err}"))?;
    }
    println!("header_validation=PARTIAL_BITCOIN_MAINNET");
    println!("anchor=PUBLISHED_BITCOIN_MAINNET_GENESIS");
    println!("sha256d_pow=CHECKED");
    println!("difficulty_transitions=CHECKED");
    println!("median_time_past=CHECKED");
    println!("best_active_chain=UNVERIFIED");
    println!("full_block_utxo_signatures=UNVERIFIED");
    println!("economic_validity=UNVERIFIED");
    println!("stwo_proof=UNVERIFIED");
    println!("last_height={}", chain.tip().height);
    println!("last_block_hash={}", to_hex(&chain.tip().id.0));
    println!("last_nbits={:08x}", chain.checkpoint().bits);
    println!("chainwork={}", to_hex(&chain.checkpoint().cumulative_work.to_be_bytes()));
    Ok(())
}
fn main() {
    if let Err(err) = run() {
        eprintln!("REJECTED (header-only): {err}");
        std::process::exit(1);
    }
}
