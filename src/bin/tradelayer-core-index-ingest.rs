//! Read-only, bounded Bitcoin Core RPC stream consumer for TradeLayer candidates.
//!
//! Input frames come from scripts/bitcoin_core_rpc_stream.py. A completed
//! stream ends with E<TAB>BLOCKS|SCRIPT<TAB>record_count. This process produces
//! syntactic OP_RETURN observations, never authenticated Bitcoin or TL state.
//! Core PR bitcoin/bitcoin#36629 indexes EXACT scripts, not all TL payloads.
use std::io::{self, BufRead};
use std::time::Instant;
use tradelayer_core::parallel_txindex::ParallelIndexConfig;
use tradelayer_core::raw_block_ingest::{UnverifiedRawBlockView, MAX_UNVERIFIED_RAW_BLOCK_BYTES};
use tradelayer_core::read_only::{id_hex, inspect_transaction, TransactionObservation};

const MAX_FRAME_BYTES: usize = MAX_UNVERIFIED_RAW_BLOCK_BYTES * 2 + 512;
const MAX_SCRIPT_TX_BYTES: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Blocks,
    Script,
}
impl Mode {
    const fn label(self) -> &'static str {
        match self {
            Self::Blocks => "BLOCKS",
            Self::Script => "SCRIPT",
        }
    }
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn hex_bytes(input: &str, maximum: usize) -> Result<Vec<u8>, String> {
    if !input.len().is_multiple_of(2) || input.len() / 2 > maximum {
        return Err("invalid or oversized hexadecimal input".into());
    }
    let mut bytes = Vec::with_capacity(input.len() / 2);
    for pair in input.as_bytes().chunks_exact(2) {
        let high = nibble(pair[0]).ok_or("invalid hexadecimal digit")?;
        let low = nibble(pair[1]).ok_or("invalid hexadecimal digit")?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hash_from_display(input: &str) -> Result<[u8; 32], String> {
    if input.len() != 64 || input.bytes().any(|b| nibble(b).is_none()) {
        return Err("expected a 64-character hash".into());
    }
    let mut bytes: [u8; 32] = hex_bytes(input, 32)?
        .try_into()
        .map_err(|_| "invalid hash length")?;
    bytes.reverse();
    Ok(bytes)
}

/// Reads one bounded line without allocating its entire unchecked length.
fn read_bounded_line<R: BufRead>(input: &mut R) -> Result<Option<String>, String> {
    let mut data = Vec::new();
    loop {
        let (consumed, ended, eof) = {
            let available = input.fill_buf().map_err(|err| err.to_string())?;
            let eof = available.is_empty();
            let end = available.iter().position(|byte| *byte == b'\n');
            let take = end.map_or(available.len(), |position| position + 1);
            if data.len().saturating_add(take) > MAX_FRAME_BYTES {
                return Err("RPC stream frame exceeds bounded raw block size".into());
            }
            data.extend_from_slice(&available[..take]);
            (take, end.is_some(), eof)
        };
        input.consume(consumed);
        if eof || ended {
            break;
        }
    }
    if data.is_empty() {
        return Ok(None);
    }
    if data.ends_with(b"\n") {
        data.pop();
    }
    if data.ends_with(b"\r") {
        data.pop();
    }
    String::from_utf8(data)
        .map(Some)
        .map_err(|_| "RPC stream must be UTF-8".into())
}

fn print_candidates(
    kind: &str,
    height: &str,
    blockhash: &str,
    position: &str,
    tx: &TransactionObservation,
    raw_transaction: &[u8],
) -> usize {
    let rawhex = raw_transaction
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    for carrier in &tx.carriers {
        println!(
            "C\t{kind}\t{height}\t{blockhash}\t{position}\t{}\t{}\t{}\t{}\t{}\t{rawhex}",
            id_hex(&tx.txid),
            carrier.vout,
            carrier.format.label(),
            carrier
                .transaction_type
                .map_or_else(|| "-".into(), |n| n.to_string()),
            carrier.status
        );
    }
    tx.carriers.len()
}

fn argument_config() -> Result<(ParallelIndexConfig, bool), String> {
    let mut config = ParallelIndexConfig::default();
    let mut one_team = false;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--workers" => {
                config.workers = arguments
                    .next()
                    .ok_or("--workers requires a value")?
                    .parse()
                    .map_err(|_| "invalid worker count")?;
            }
            "--one-team" => one_team = true,
            _ => {
                return Err(
                    "usage: tradelayer-core-index-ingest [--workers 1..16] [--one-team]".into(),
                )
            }
        }
    }
    config.validate().map_err(|err| format!("{err:?}"))?;
    Ok((config, one_team))
}

fn run() -> Result<(), String> {
    let (config, one_team) = argument_config()?;
    let mut reader = io::stdin().lock();
    let start = Instant::now();
    let mut mode: Option<Mode> = None;
    let mut records = 0usize;
    let mut total_txs = 0usize;
    let mut candidates = 0usize;
    let mut last_height: Option<u32> = None;
    let mut last_blockhash: Option<[u8; 32]> = None;
    let mut complete = false;
    while let Some(line) = read_bounded_line(&mut reader)? {
        if complete {
            return Err("data after end-of-stream sentinel".into());
        }
        let columns = line.split('\t').collect::<Vec<_>>();
        match *columns.as_slice() {
            ["B", height, blockhash, rawhex] => {
                if mode.is_some_and(|m| m != Mode::Blocks) {
                    return Err("cannot mix BLOCKS and SCRIPT frames".into());
                }
                mode = Some(Mode::Blocks);
                let height = height.parse::<u32>().map_err(|_| "invalid block height")?;
                if last_height.is_some_and(|previous| previous.checked_add(1) != Some(height)) {
                    return Err("block stream must be contiguous and ascending".into());
                }
                let expected = hash_from_display(blockhash)?;
                let bytes = hex_bytes(rawhex, MAX_UNVERIFIED_RAW_BLOCK_BYTES)?;
                let view = UnverifiedRawBlockView::parse(&bytes, Some(expected))
                    .map_err(|err| format!("block {height}: {err:?}"))?;
                if last_blockhash.is_some_and(|previous| &bytes[4..36] != previous.as_slice()) {
                    return Err("block previous-hash linkage mismatch".into());
                }
                let report = if one_team {
                    view.inspect_parallel_one_team(config)
                } else {
                    view.inspect_parallel(config)
                }
                .map_err(|err| format!("block {height}: {err:?}"))?;
                for candidate in report.ordered_preindex().candidates_in_source_order() {
                    candidates += print_candidates(
                        "BLOCK",
                        &height.to_string(),
                        blockhash,
                        &candidate.position.to_string(),
                        &candidate.observation,
                        view.raw_transaction(candidate.position)
                            .ok_or("missing source transaction range")?,
                    );
                }
                total_txs = total_txs
                    .checked_add(view.transaction_count())
                    .ok_or("counter overflow")?;
                records += 1;
                last_height = Some(height);
                last_blockhash = Some(expected);
            }
            ["S", txid, blockhash, rawhex, matches] => {
                if mode.is_some_and(|m| m != Mode::Script) {
                    return Err("cannot mix BLOCKS and SCRIPT frames".into());
                }
                mode = Some(Mode::Script);
                if blockhash != "-" {
                    hash_from_display(blockhash)?;
                }
                let expected_txid = hash_from_display(txid)?;
                let raw = hex_bytes(rawhex, MAX_SCRIPT_TX_BYTES)?;
                let observed = inspect_transaction(&raw)
                    .map_err(|err| format!("script-history transaction {txid}: {err}"))?;
                if observed.txid != expected_txid {
                    return Err("script-history raw transaction TXID mismatch".into());
                }
                if matches.is_empty() {
                    return Err("script-history result lacks matching output indexes".into());
                }
                let mut previous = None;
                for output in matches.split(',') {
                    let index = output
                        .parse::<u32>()
                        .map_err(|_| "invalid matching output index")?;
                    if index >= observed.output_count || previous.is_some_and(|p| p >= index) {
                        return Err("invalid or duplicated matching output index".into());
                    }
                    previous = Some(index);
                }
                candidates += print_candidates("SCRIPT", "-", blockhash, "-", &observed, &raw);
                total_txs += 1;
                records += 1;
            }
            ["E", kind, count] => {
                let expected_mode = match kind {
                    "BLOCKS" => Mode::Blocks,
                    "SCRIPT" => Mode::Script,
                    _ => return Err("invalid stream end mode".into()),
                };
                if mode.is_some_and(|m| m != expected_mode) {
                    return Err("stream end mode mismatches records".into());
                }
                let count = count.parse::<usize>().map_err(|_| "invalid stream count")?;
                if count != records {
                    return Err("stream end count mismatch".into());
                }
                complete = true;
                mode = Some(expected_mode);
            }
            _ => return Err("invalid RPC stream frame".into()),
        }
    }
    if !complete {
        return Err("truncated RPC stream: missing E sentinel".into());
    }
    let seconds = start.elapsed().as_secs_f64().max(0.000_001);
    println!(
        "END\tCOMPLETE_UNVERIFIED\t{}\t{records}\t{total_txs}\t{candidates}",
        mode.expect("end sentinel required").label()
    );
    eprintln!(
        "records={records} txs={total_txs} candidates={candidates} elapsed_s={seconds:.3} observed_txs_per_sec={:.0} source=UNVERIFIED",
        total_txs as f64 / seconds
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("REJECTED RPC INDEX STREAM: {error}");
        std::process::exit(1);
    }
}
