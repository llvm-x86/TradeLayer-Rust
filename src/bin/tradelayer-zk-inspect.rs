//! Inspect zkTL commitment metadata. This binary cannot verify STWO proofs.
use std::fs;
use tradelayer_core::zk_batch::parse_bridge_tsv;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: tradelayer-zk-inspect <exported-tlzk-batch.tsv>");
        std::process::exit(2);
    };
    let bytes = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("cannot read candidate: {error}");
            std::process::exit(2);
        }
    };
    match parse_bridge_tsv(&bytes).and_then(|batch| {
        batch.inspect_strict()?;
        Ok(batch)
    }) {
        Ok(batch) => println!(
            "batch={} receipts={} interval={}-{} metadata=consistent stwo_proof=UNVERIFIED economic_validity=UNVERIFIED",
            batch.batch_id.hex(), batch.receipts.len(), batch.start_height, batch.end_height
        ),
        Err(error) => {
            eprintln!("REJECTED: {error}");
            std::process::exit(1);
        }
    }
}
