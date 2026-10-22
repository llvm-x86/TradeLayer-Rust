//! Export *diagnostic* legacy fields for differential tests with the JS decoder.
//! Do not use this output to authorize TradeLayer balance changes.
use std::{env, fs, process};
use tradelayer_core::read_only::{hex_bytes, id_hex, replay_tsv, CarrierFormat};
use tradelayer_core::wire::{parse_frame, parse_typed, ParsedTransaction};

fn export_values(tx: ParsedTransaction) -> (&'static str, String, String, String, String) {
    match tx {
        ParsedTransaction::Activate {
            types,
            code_hash_base36,
        } => (
            "activate",
            types
                .iter()
                .map(|t| t.as_u8().to_string())
                .collect::<Vec<_>>()
                .join(";"),
            code_hash_base36,
            "-".into(),
            "-".into(),
        ),
        ParsedTransaction::Issue {
            initial_tokens,
            ticker,
            whitelist_raw,
            managed,
            ..
        } => (
            "issue",
            initial_tokens.to_string(),
            ticker,
            whitelist_raw,
            if managed { "1" } else { "0" }.into(),
        ),
        ParsedTransaction::Send {
            recipient,
            property_id,
            amount_atoms,
        } => (
            "send",
            property_id.to_string(),
            amount_atoms.to_string(),
            recipient,
            "-".into(),
        ),
        ParsedTransaction::SendAll { recipient } => {
            ("all", recipient, "-".into(), "-".into(), "-".into())
        }
        ParsedTransaction::Grant {
            property_id,
            amount_atoms,
            recipient,
            ..
        } => (
            "grant",
            property_id.to_string(),
            amount_atoms.to_string(),
            recipient,
            "-".into(),
        ),
        ParsedTransaction::Redeem {
            property_id,
            amount_atoms,
            ..
        } => (
            "redeem",
            property_id.to_string(),
            amount_atoms.to_string(),
            "-".into(),
            "-".into(),
        ),
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: tradelayer-legacy-fields <raw_transaction_corpus.tsv>");
        process::exit(2);
    }
    let content = fs::read_to_string(&args[1]).unwrap_or_else(|err| {
        eprintln!("cannot read corpus: {err}");
        process::exit(1);
    });
    let rows = replay_tsv(&content).unwrap_or_else(|err| {
        eprintln!("invalid corpus: {err}");
        process::exit(1);
    });
    println!("# txid\tvout\ttype\tbody_hex\trust_kind\tfield1\tfield2\tfield3\tfield4");
    for row in rows {
        for carrier in row.observation.carriers {
            if carrier.format != CarrierFormat::LegacyAscii {
                continue;
            }
            let txid = id_hex(&row.observation.txid);
            let Some(input) = std::str::from_utf8(&carrier.payload).ok() else {
                continue;
            };
            let Ok(frame) = parse_frame(input) else {
                continue;
            };
            let (kind, one, two, three, four) = match parse_typed(input) {
                Ok(tx) => export_values(tx),
                Err(_) => ("invalid", "-".into(), "-".into(), "-".into(), "-".into()),
            };
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                txid,
                carrier.vout,
                frame.tx_type.as_u8(),
                hex_bytes(frame.body.as_bytes()),
                kind,
                one,
                two,
                three,
                four
            );
        }
    }
}
