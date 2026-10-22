//! Read-only local inspector: TLPT transitions + zkTL receipts + declared
//! L1 observations. ALL SOURCES UNVERIFIED; matching is not proof of anything.
//!
//! Usage:
//! tradelayer-root-reconcile <tlpt.tsv> <tlzk.tsv> <observed.tsv>
//!    <chain_hex> <anchor_height> <anchor_block_hex>
//!    <final_height> <final_block_hex>
use std::fs::File;
use std::io::Read;

use tradelayer_core::chain::{BlockId, ChainId, Tip};
use tradelayer_core::root_reconciliation::{
    parse_observations_tsv_unverified, parse_tlpt_tsv_unverified, reconcile_unverified,
    ExpectedChainRange, MAX_ROOT_BRIDGE_TEXT_BYTES,
};
use tradelayer_core::zk_batch::{parse_bridge_tsv, Digest, MAX_BATCH_BYTES};

fn load_bounded(path: &str, limit: usize) -> Result<String, String> {
    let file = File::open(path).map_err(|err| format!("{path}: {err}"))?;
    if file
        .metadata()
        .map_err(|err| format!("{path}: {err}"))?
        .len()
        > limit as u64
    {
        return Err(format!("{path}: too large"));
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("{path}: {err}"))?;
    if bytes.len() > limit {
        return Err(format!("{path}: too large"));
    }
    String::from_utf8(bytes).map_err(|err| format!("{path}: non-UTF8: {err}"))
}

fn canonical_height(s: &str) -> Result<u64, String> {
    if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|c| c.is_ascii_digit())
    {
        return Err("noncanonical decimal height".to_owned());
    }
    s.parse().map_err(|_| "height overflow".to_owned())
}

fn hash_32(value: &str) -> Result<[u8; 32], String> {
    Digest::parse(value)
        .map(Digest::as_bytes)
        .map_err(|err| format!("{err}"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 9 {
        return Err(concat!(
            "usage: tradelayer-root-reconcile <tlpt.tsv> <tlzk.tsv> <observed.tsv> ",
            "<chain_id_hex> <anchor_height> <anchor_hash_hex> ",
            "<final_height> <final_hash_hex>"
        )
        .to_owned());
    }
    let expected = ExpectedChainRange {
        chain: ChainId(hash_32(&args[4])?),
        anchor: Tip {
            height: canonical_height(&args[5])?,
            id: BlockId(hash_32(&args[6])?),
        },
        final_tip: Tip {
            height: canonical_height(&args[7])?,
            id: BlockId(hash_32(&args[8])?),
        },
    };
    let tlpt = load_bounded(&args[1], MAX_ROOT_BRIDGE_TEXT_BYTES)?;
    let zk = load_bounded(&args[2], MAX_BATCH_BYTES)?;
    let observations = load_bounded(&args[3], MAX_ROOT_BRIDGE_TEXT_BYTES)?;
    let statements = parse_tlpt_tsv_unverified(&tlpt).map_err(|err| format!("{err}"))?;
    let batch = parse_bridge_tsv(&zk).map_err(|err| format!("{err}"))?;
    let observed =
        parse_observations_tsv_unverified(&observations).map_err(|err| format!("{err}"))?;
    let result = reconcile_unverified(&batch, &statements, &observed, expected)
        .map_err(|err| format!("{err}"))?;
    println!("root_alignment=AUDIT_ONLY");
    println!("chain_consensus=UNVERIFIED");
    println!("transaction_inclusion=UNVERIFIED");
    println!("economic_validity=UNVERIFIED");
    println!("stwo_proof=UNVERIFIED");
    println!("root_namespaces=SEPARATE");
    println!("chain_id={}", hex(&expected.chain.0));
    println!("start_height={}", result.expected.anchor.height + 1);
    println!("end_height={}", result.expected.final_tip.height);
    println!("blocks={}", result.blocks.len());
    println!("receipts={}", result.receipt_count);
    println!("tlpt_before={}", hex(&result.rust_first_root));
    println!("tlpt_after={}", hex(&result.rust_last_root));
    println!("zktl_before={}", result.zktl_first_root.hex());
    println!("zktl_after={}", result.zktl_last_root.hex());
    println!("zktl_batch_id={}", result.zktl_batch_id.hex());
    println!("audit_digest={}", hex(&result.audit_digest));
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("REJECTED (audit only): {err}");
        std::process::exit(1);
    }
}
