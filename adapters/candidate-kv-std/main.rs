//! Standalone, dependency-free experimental immutable-segment candidate store.
//!
//! Compile directly with rustc. This is deliberately OUTSIDE Cargo's frozen
//! TradeLayer consensus package; no third-party crates or unsafe are used.
//! It is an UNVERIFIED cache, never a TrustedChainAdapter or Bitcoin node.
//!
//! Storage protocol (Linux):
//!  1. Validate complete B/E and C/END input files.
//!  2. Acquire a fail-closed create_new writer lock.
//!  3. Fsync immutable per-block segments, then fsync segment directory.
//!  4. Fsync a new manifest, atomically rename it to CURRENT, fsync root.
//!  5. Crash before CURRENT publication leaves previous generation readable.
//!
//! Abandoned segments are retained intentionally. A process killed while
//! holding LOCK leaves a stale lock; never remove it automatically.

#![forbid(unsafe_code)]

#[allow(dead_code)]
#[path = "../../src/hash256.rs"]
mod hash256;

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAGIC: &str = "TLCAND-MANIFEST-1";
const SEG_MAGIC: &str = "TLCAND-SEGMENT-1";
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BLOCK_BYTES: usize = 4_000_000;
const MAX_BLOCKS: usize = 256;
const MAX_CARRIERS: usize = 20_000;
const MAX_TX_COUNT: u64 = 1_000_000;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
struct Block {
    height: u32,
    hash: String,
    parent: String,
    tx_count: u32,
    segment: String,
    segment_digest: String,
}

#[derive(Clone, Debug)]
struct Manifest {
    network: String,
    anchor_height: i64,
    anchor_hash: String,
    generation: u64,
    blocks: Vec<Block>,
}

#[derive(Clone, Debug)]
struct Incoming {
    height: u32,
    hash: String,
    parent: String,
    tx_count: u32,
    carriers: Vec<String>,
}

fn io_error(label: &str, e: std::io::Error) -> String {
    format!("{label}: {e}")
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn valid_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| hex_digit(b).is_some())
}

fn hex_string(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn hex_decode(value: &str, max_bytes: usize) -> Result<Vec<u8>> {
    if value.is_empty() || value.len() % 2 != 0 || value.len() / 2 > max_bytes {
        return Err("invalid or oversized hex input".into());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let hi = hex_digit(pair[0]).ok_or("hex must be lowercase ASCII")?;
        let lo = hex_digit(pair[1]).ok_or("hex must be lowercase ASCII")?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn number(s: &str, max: u64) -> Result<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit())
        || (s.len() > 1 && s.starts_with('0'))
    {
        return Err("noncanonical decimal number".into());
    }
    let n: u64 = s.parse().map_err(|_| "decimal overflow")?;
    if n > max {
        return Err("numeric resource bound exceeded".into());
    }
    Ok(n)
}

fn reverse_hex(bytes: &[u8]) -> String {
    let mut reversed = bytes.to_vec();
    reversed.reverse();
    hex_string(&reversed)
}

fn digest(bytes: &[u8]) -> String {
    hex_string(&hash256::double_sha256(bytes))
}

fn read_text(path: &Path, limit: u64) -> Result<String> {
    let size = fs::metadata(path).map_err(|e| io_error("stat input", e))?.len();
    if size > limit {
        return Err("input exceeds bounded file size".into());
    }
    let raw = fs::read(path).map_err(|e| io_error("read input", e))?;
    if raw.len() as u64 > limit || raw.is_empty() || *raw.last().unwrap_or(&0) != b'\n'
        || raw.contains(&b'\r') || raw.contains(&0)
    {
        return Err("truncated, noncanonical or oversized text file".into());
    }
    String::from_utf8(raw).map_err(|_| "input is not canonical UTF-8".into())
}

fn fields(line: &str) -> Vec<&str> {
    line.split('\t').collect()
}

fn compact_count(raw: &[u8]) -> Result<u32> {
    if raw.len() < 81 {
        return Err("truncated raw block header/transaction count".into());
    }
    let first = raw[80];
    let count: u64 = if first < 253 {
        u64::from(first)
    } else {
        let width = match first {
            253 => 2,
            254 => 4,
            _ => 8,
        };
        if raw.len() < 81 + width {
            return Err("truncated block CompactSize".into());
        }
        let mut arr = [0u8; 8];
        arr[..width].copy_from_slice(&raw[81..81 + width]);
        let n = u64::from_le_bytes(arr);
        if n < 253 || (first == 254 && n <= u16::MAX as u64)
            || (first == 255 && n <= u32::MAX as u64)
        {
            return Err("noncanonical block CompactSize".into());
        }
        n
    };
    if count == 0 || count > MAX_TX_COUNT {
        return Err("invalid transaction count".into());
    }
    u32::try_from(count).map_err(|_| "transaction count overflow".into())
}

fn parse_block_frames(path: &Path) -> Result<(Vec<Incoming>, u64)> {
    let text = read_text(path, MAX_FILE_BYTES)?;
    let mut blocks: Vec<Incoming> = Vec::new();
    let mut total_txs = 0u64;
    let mut done = false;
    for line in text.lines() {
        if done {
            return Err("trailing raw block frame after completion".into());
        }
        let f = fields(line);
        match f.as_slice() {
            ["B", height_s, hash, raw_hex] => {
                let height = number(height_s, u32::MAX as u64)? as u32;
                if !valid_hash(hash) {
                    return Err("bad raw block hash".into());
                }
                let raw = hex_decode(raw_hex, MAX_BLOCK_BYTES)?;
                if raw.len() < 81 {
                    return Err("missing raw block header".into());
                }
                let calculated = reverse_hex(&hash256::double_sha256(&raw[..80]));
                if calculated != *hash {
                    return Err("raw block header hash mismatch".into());
                }
                let parent = reverse_hex(&raw[4..36]);
                let count = compact_count(&raw)?;
                if let Some(last) = blocks.last() {
                    if height != last.height.checked_add(1).ok_or("height overflow")?
                        || parent != last.hash
                    {
                        return Err("raw blocks are not consecutive parent-linked heights".into());
                    }
                }
                total_txs = total_txs.checked_add(u64::from(count))
                    .ok_or("transaction count overflow")?;
                if blocks.len() >= MAX_BLOCKS || total_txs > MAX_TX_COUNT {
                    return Err("raw block batch exceeds resource cap".into());
                }
                blocks.push(Incoming {
                    height,
                    hash: hash.to_string(),
                    parent,
                    tx_count: count,
                    carriers: Vec::new(),
                });
            }
            ["E", "BLOCKS", count] => {
                if blocks.is_empty() || number(count, MAX_BLOCKS as u64)? != blocks.len() as u64 {
                    return Err("invalid E block completion marker".into());
                }
                done = true;
            }
            _ => return Err("unexpected raw block frame".into()),
        }
    }
    if !done {
        return Err("raw block stream lacks E completion".into());
    }
    Ok((blocks, total_txs))
}

fn parse_candidates(path: &Path, blocks: &mut [Incoming], tx_sum: u64) -> Result<()> {
    let text = read_text(path, MAX_FILE_BYTES)?;
    let first_height = blocks.first().ok_or("empty raw block batch")?.height;
    let mut last_key: Option<(u32, u32, u32)> = None;
    let mut last_tx: Option<(u32, u32, String, String)> = None;
    let mut seen = 0usize;
    let mut done = false;
    for line in text.lines() {
        if done {
            return Err("trailing candidate frame after completion".into());
        }
        let f = fields(line);
        match f.as_slice() {
            ["C", "BLOCK", height_s, hash, pos_s, txid, vout_s, fmt, typ, status, raw] => {
                let height = number(height_s, u32::MAX as u64)? as u32;
                let idx = height.checked_sub(first_height).ok_or("candidate height below batch")?
                    as usize;
                let block = blocks.get_mut(idx).ok_or("candidate height outside batch")?;
                if hash != &block.hash || !valid_hash(txid) {
                    return Err("candidate block or TXID hash mismatch".into());
                }
                let pos = number(pos_s, u64::from(block.tx_count - 1))? as u32;
                let vout = number(vout_s, 100_000)? as u32;
                let bytes = hex_decode(raw, 1_000_000)?;
                if bytes.len() < 2 {
                    return Err("candidate transaction too short".into());
                }
                for part in [fmt, typ, status] {
                    if part.is_empty() || part.len() > 100
                        || !part.bytes().all(|b| {
                            b.is_ascii_alphanumeric()
                                || matches!(b, b'_' | b'-' | b'.' | b':')
                        })
                    {
                        return Err("invalid candidate classification".into());
                    }
                }
                let key = (height, pos, vout);
                if last_key.is_some_and(|previous| key <= previous) {
                    return Err("candidate keys not strictly ordered by height, position, vout".into());
                }
                if let Some((prior_height, prior_pos, prior_id, prior_raw)) = &last_tx {
                    if *prior_height == height && *prior_pos == pos
                        && (prior_id != txid || prior_raw != raw)
                    {
                        return Err("same-position candidate has conflicting TX bytes".into());
                    }
                }
                last_tx = Some((height, pos, txid.to_string(), raw.to_string()));
                last_key = Some(key);
                block.carriers.push(line.to_string());
                seen += 1;
                if seen > MAX_CARRIERS {
                    return Err("candidate resource cap exceeded".into());
                }
            }
            ["END", "COMPLETE_UNVERIFIED", "BLOCKS", count, txcount, candidates] => {
                if number(count, MAX_BLOCKS as u64)? != blocks.len() as u64
                    || number(txcount, MAX_TX_COUNT)? != tx_sum
                    || number(candidates, MAX_CARRIERS as u64)? != seen as u64
                {
                    return Err("candidate END completion markers disagree with raw blocks".into());
                }
                done = true;
            }
            _ => return Err("unexpected candidate frame".into()),
        }
    }
    if !done {
        return Err("candidate stream lacks END completion".into());
    }
    Ok(())
}

fn segment_name(height: u32, digest: &str) -> String {
    format!("seg-{height:010}-{digest}.dat")
}

fn valid_segment_name(name: &str, height: u32, digest: &str) -> bool {
    valid_hash(digest) && name == segment_name(height, digest)
}

fn render_manifest(m: &Manifest) -> String {
    let mut s = format!(
        "{MAGIC}\nNETWORK\t{}\nANCHOR\t{}\t{}\nGEN\t{}\n",
        m.network, m.anchor_height, m.anchor_hash, m.generation
    );
    for b in &m.blocks {
        s.push_str(&format!(
            "B\t{}\t{}\t{}\t{}\t{}\t{}\n",
            b.height, b.hash, b.parent, b.tx_count, b.segment, b.segment_digest
        ));
    }
    s.push_str(&format!("END\t{}\n", m.blocks.len()));
    s
}

fn check_manifest(m: &Manifest) -> Result<()> {
    if !matches!(m.network.as_str(), "main" | "test" | "testnet4" | "signet" | "regtest")
        || !valid_hash(&m.anchor_hash) || m.anchor_height < -1
        || m.anchor_height > u32::MAX as i64
    {
        return Err("invalid manifest identity or anchor".into());
    }
    if m.anchor_height == -1 && m.anchor_hash != "0".repeat(64) {
        return Err("pre-genesis anchor must be all zero".into());
    }
    let mut last_height = m.anchor_height;
    let mut last_hash: &str = &m.anchor_hash;
    for b in &m.blocks {
        if i64::from(b.height) != last_height + 1 || b.parent != last_hash
            || !valid_hash(&b.hash) || !valid_hash(&b.parent)
            || !valid_segment_name(&b.segment, b.height, &b.segment_digest)
            || !valid_hash(&b.segment_digest) || b.tx_count == 0
        {
            return Err("manifest has gap, inconsistent parent, or malformed segment".into());
        }
        last_height = i64::from(b.height);
        last_hash = &b.hash;
    }
    Ok(())
}

fn parse_manifest(s: &str) -> Result<Manifest> {
    if !s.ends_with('\n') || s.contains('\r') {
        return Err("noncanonical manifest".into());
    }
    let mut lines = s.lines();
    if lines.next() != Some(MAGIC) {
        return Err("unrecognized manifest version".into());
    }
    let network_line = lines.next().ok_or("missing NETWORK")?;
    let n = fields(network_line);
    let ["NETWORK", network] = n.as_slice() else {
        return Err("invalid NETWORK field".into());
    };
    let anchor_line = lines.next().ok_or("missing ANCHOR")?;
    let a = fields(anchor_line);
    let ["ANCHOR", ah, hash] = a.as_slice() else {
        return Err("invalid ANCHOR field".into());
    };
    let anchor_height: i64 = ah.parse().map_err(|_| "invalid anchor height")?;
    let g = fields(lines.next().ok_or("missing GEN")?);
    let ["GEN", generation_s] = g.as_slice() else {
        return Err("invalid GEN".into());
    };
    let generation = number(generation_s, u64::MAX)?;
    let mut blocks = Vec::new();
    let mut ended = false;
    for line in lines {
        if ended {
            return Err("trailing manifest data".into());
        }
        let f = fields(line);
        match f.as_slice() {
            ["B", h, blockhash, parent, txcount, segment, segment_digest] => {
                blocks.push(Block {
                    height: number(h, u32::MAX as u64)? as u32,
                    hash: blockhash.to_string(),
                    parent: parent.to_string(),
                    tx_count: number(txcount, MAX_TX_COUNT)? as u32,
                    segment: segment.to_string(),
                    segment_digest: segment_digest.to_string(),
                });
                if blocks.len() > 2_000_000 {
                    return Err("manifest entry bound exceeded".into());
                }
            }
            ["END", count] => {
                if number(count, 2_000_000)? != blocks.len() as u64 {
                    return Err("manifest count mismatch".into());
                }
                ended = true;
            }
            _ => return Err("unknown manifest record".into()),
        }
    }
    if !ended {
        return Err("manifest lacks END".into());
    }
    let m = Manifest {
        network: network.to_string(),
        anchor_height,
        anchor_hash: hash.to_string(),
        generation,
        blocks,
    };
    check_manifest(&m)?;
    Ok(m)
}

fn load_manifest(dir: &Path) -> Result<Manifest> {
    let text = read_text(&dir.join("CURRENT"), 64 * 1024 * 1024)?;
    parse_manifest(&text)
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path).map_err(|e| io_error("open directory for sync", e))?
        .sync_all().map_err(|e| io_error("sync directory", e))
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true)
        .open(path).map_err(|e| io_error("create immutable segment/temp", e))?;
    file.write_all(bytes).map_err(|e| io_error("write immutable file", e))?;
    file.sync_all().map_err(|e| io_error("fsync immutable file", e))
}

fn publish(dir: &Path, manifest: &Manifest) -> Result<()> {
    check_manifest(manifest)?;
    let temp = dir.join(format!("CURRENT.{}.tmp", std::process::id()));
    write_new(&temp, render_manifest(manifest).as_bytes())?;
    fs::rename(&temp, dir.join("CURRENT"))
        .map_err(|e| io_error("publish manifest pointer", e))?;
    sync_directory(dir)
}

struct WriterLock {
    path: PathBuf,
}
impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
fn writer_lock(dir: &Path) -> Result<WriterLock> {
    let path = dir.join("LOCK");
    let mut file = OpenOptions::new().write(true).create_new(true)
        .open(&path)
        .map_err(|_| "writer LOCK exists or cannot be created; inspect any stale writer lock manually")?;
    writeln!(file, "pid={}", std::process::id())
        .map_err(|e| io_error("write lock", e))?;
    file.sync_all().map_err(|e| io_error("sync lock", e))?;
    Ok(WriterLock { path })
}

fn initialize(dir: &Path, network: &str, anchor_height: i64, anchor_hash: &str) -> Result<()> {
    let m = Manifest {
        network: network.to_string(),
        anchor_height,
        anchor_hash: anchor_hash.to_string(),
        generation: 0,
        blocks: Vec::new(),
    };
    check_manifest(&m)?;
    fs::create_dir(dir).map_err(|e| io_error("create new store directory", e))?;
    fs::create_dir(dir.join("segments")).map_err(|e| io_error("create segment directory", e))?;
    sync_directory(dir)?;
    publish(dir, &m)
}

fn segment_bytes(b: &Incoming) -> Vec<u8> {
    let mut s = format!(
        "{SEG_MAGIC}\t{}\t{}\t{}\t{}\n",
        b.height, b.hash, b.parent, b.tx_count
    );
    for row in &b.carriers {
        s.push_str(row);
        s.push('\n');
    }
    s.push_str(&format!("END\t{}\n", b.carriers.len()));
    s.into_bytes()
}

fn read_segment(dir: &Path, b: &Block) -> Result<String> {
    if !valid_segment_name(&b.segment, b.height) {
        return Err("invalid manifest segment path".into());
    }
    let bytes = fs::read(dir.join("segments").join(&b.segment))
        .map_err(|e| io_error("read segment", e))?;
    if bytes.len() > MAX_FILE_BYTES as usize || digest(&bytes) != b.segment_digest {
        return Err("segment payload digest mismatch or resource overflow".into());
    }
    let content = String::from_utf8(bytes).map_err(|_| "segment not UTF-8")?;
    let mut iter = content.lines();
    let first = iter.next().ok_or("empty segment")?;
    let header = fields(first);
    let expected = format!(
        "{SEG_MAGIC}\t{}\t{}\t{}\t{}",
        b.height, b.hash, b.parent, b.tx_count
    );
    if first != expected || header.len() != 5 {
        return Err("segment header disagrees with manifest".into());
    }
    let mut count = 0usize;
    let mut ended = false;
    for line in iter {
        if ended {
            return Err("trailing segment records".into());
        }
        let f = fields(line);
        match f.as_slice() {
            ["C", "BLOCK", height_s, hash, ..] if *height_s == b.height.to_string()
                    && *hash == b.hash => count += 1,
            ["END", n] if number(n, MAX_CARRIERS as u64)? == count as u64 => ended = true,
            _ => return Err("malformed segment rows".into()),
        }
    }
    if !ended {
        return Err("segment missing END".into());
    }
    Ok(content)
}

fn ingest(dir: &Path, network: &str, blocks_file: &Path, candidates_file: &Path) -> Result<(usize, usize)> {
    let (mut new_blocks, tx_sum) = parse_block_frames(blocks_file)?;
    parse_candidates(candidates_file, &mut new_blocks, tx_sum)?;
    let _guard = writer_lock(dir)?;
    let original = load_manifest(dir)?;
    if original.network != network {
        return Err("wrong network namespace for candidate store".into());
    }
    let first = new_blocks.first().ok_or("empty block batch")?;
    let mut kept = original.blocks.iter()
        .take_while(|b| b.height < first.height).cloned().collect::<Vec<_>>();
    let (parent_height, parent_hash) = match kept.last() {
        Some(b) => (i64::from(b.height), b.hash.as_str()),
        None => (original.anchor_height, original.anchor_hash.as_str()),
    };
    if i64::from(first.height) != parent_height + 1 || first.parent != parent_hash {
        return Err("raw block range does not attach to previously indexed chain or anchor".into());
    }
    let generation = original.generation.checked_add(1).ok_or("generation overflow")?;
    for b in &new_blocks {
        let data = segment_bytes(b);
        if data.len() > MAX_FILE_BYTES as usize {
            return Err("candidate block segment exceeds size cap".into());
        }
        let hash = digest(&data);
        let filename = segment_name(b.height, &hash);
        let path = dir.join("segments").join(&filename);
        if path.exists() {
            let existing = fs::read(&path).map_err(|e| io_error("read existing segment", e))?;
            if existing != data {
                return Err("existing immutable segment has conflicting bytes".into());
            }
        } else {
            write_new(&path, &data)?;
        }
        kept.push(Block {
            height: b.height,
            hash: b.hash.clone(),
            parent: b.parent.clone(),
            tx_count: b.tx_count,
            segment: filename,
            segment_digest: hash,
        });
    }
    sync_directory(&dir.join("segments"))?;
    // Test-only fault injection, never used as a production recovery mechanism.
    if env::var("TLCAND_TEST_FAILPOINT").ok().as_deref() == Some("after_segments") {
        std::process::abort();
    }
    let next = Manifest {
        network: original.network,
        anchor_height: original.anchor_height,
        anchor_hash: original.anchor_hash,
        generation,
        blocks: kept,
    };
    check_manifest(&next)?;
    publish(dir, &next)?;
    let carriers = new_blocks.iter().map(|b| b.carriers.len()).sum();
    Ok((new_blocks.len(), carriers))
}

fn verify(dir: &Path) -> Result<Manifest> {
    let m = load_manifest(dir)?;
    for b in &m.blocks {
        let _ = read_segment(dir, b)?;
    }
    Ok(m)
}

fn status(dir: &Path, full_verify: bool) -> Result<()> {
    let m = if full_verify { verify(dir)? } else { load_manifest(dir)? };
    let last = m.blocks.last();
    println!("schema={MAGIC}");
    println!("network={}", m.network);
    println!("trust=UNVERIFIED");
    println!("generation={}", m.generation);
    println!("blocks={}", m.blocks.len());
    println!("height={}", last.map_or(m.anchor_height, |b| i64::from(b.height)));
    println!("hash={}", last.map_or(m.anchor_hash.as_str(), |b| b.hash.as_str()));
    if full_verify {
        println!("segment_integrity=CHECKED");
    }
    Ok(())
}

fn get(dir: &Path, height: u32) -> Result<()> {
    let m = load_manifest(dir)?;
    let b = m.blocks.iter().find(|b| b.height == height).ok_or("height not indexed")?;
    let segment = read_segment(dir, b)?;
    for line in segment.lines() {
        if line.starts_with("C\tBLOCK\t") {
            println!("{line}");
        }
    }
    println!("END\tUNVERIFIED\t{height}");
    Ok(())
}

fn args_map(args: &[String]) -> Result<BTreeMap<String, String>> {
    if args.len() % 2 != 0 {
        return Err("each CLI flag requires a value".into());
    }
    let mut m = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        if !pair[0].starts_with("--") || m.insert(pair[0].clone(), pair[1].clone()).is_some() {
            return Err("invalid or repeated CLI option".into());
        }
    }
    Ok(m)
}

fn option<'a>(map: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str> {
    map.get(key).map(String::as_str).ok_or_else(|| format!("missing {key}"))
}

fn run() -> Result<()> {
    let args = env::args().collect::<Vec<_>>();
    let command = args.get(1).ok_or("usage: candidate-store {init|ingest|status|verify|get}")?;
    let options = args_map(&args[2..])?;
    let dir = Path::new(option(&options, "--dir")?);
    match command.as_str() {
        "init" => {
            let network = option(&options, "--network")?;
            let anchor_height: i64 = option(&options, "--anchor-height")?.parse()
                .map_err(|_| "invalid anchor height")?;
            initialize(dir, network, anchor_height, option(&options, "--anchor-hash")?)?;
            println!("initialized=UNVERIFIED");
        }
        "ingest" => {
            let network = option(&options, "--network")?;
            let blocks = Path::new(option(&options, "--blocks")?);
            let candidates = Path::new(option(&options, "--candidates")?);
            let (n, c) = ingest(dir, network, blocks, candidates)?;
            println!("committed=UNVERIFIED blocks={n} carriers={c}");
        }
        "status" => status(dir, false)?,
        "verify" => status(dir, true)?,
        "get" => {
            let h = number(option(&options, "--height")?, u32::MAX as u64)? as u32;
            get(dir, h)?;
        }
        _ => return Err("unknown CLI command".into()),
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("REJECTED UNVERIFIED CANDIDATE STORE: {e}");
        std::process::exit(1);
    }
}
