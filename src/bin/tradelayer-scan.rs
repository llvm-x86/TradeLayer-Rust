//! Inspect a local TSV corpus. Completely offline; never writes ledger state.
use std::{env, fs, process};
use tradelayer_core::read_only::{id_hex, replay_tsv};

fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: tradelayer-scan <fixture-file.tsv>");
        process::exit(2);
    }
    let content = match fs::read_to_string(&args[1]) {
        Ok(content) => content,
        Err(err) => { eprintln!("cannot read fixture: {err}"); process::exit(1); }
    };
    let rows = match replay_tsv(&content) {
        Ok(rows) => rows,
        Err(err) => { eprintln!("scan failed: {err}"); process::exit(1); }
    };
    println!("label\tnetwork\treported_height\ttxid\twitness\top_return_outputs\tvout\tformat\ttype\tsyntax");
    for row in &rows {
        let height = row.reported_height.map_or_else(|| "-".into(), |n| n.to_string());
        let id = id_hex(&row.observation.txid);
        let witness = if row.observation.has_witness { "yes" } else { "no" };
        if row.observation.carriers.is_empty() {
            println!("{}\t{}\t{}\t{}\t{}\t{}\t-\t-\t-\t-", row.label, row.network_label, height, id, witness, row.observation.op_return_count);
        }
        for carrier in &row.observation.carriers {
            let kind = carrier.transaction_type.map_or_else(|| "-".into(), |n| n.to_string());
            println!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", row.label, row.network_label, height, id, witness, row.observation.op_return_count, carrier.vout, carrier.format.label(), kind, carrier.status);
        }
    }
}
