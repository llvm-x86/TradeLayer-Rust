//! Offline Bitcoin Core RPC frame contract, using the existing raw-block fixture.
use std::io::Write;
use std::process::{Command, Stdio};
use tradelayer_core::raw_block_ingest::UnverifiedRawBlockView;
use tradelayer_core::read_only::{decode_hex, id_hex, inspect_transaction};

fn run(input: &str) -> (bool, String, String) {
    let binary = env!("CARGO_BIN_EXE_tradelayer-core-index-ingest");
    let mut process = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = process.wait_with_output().unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn block_fixture() -> (String, String, String) {
    let block_hex = include_str!("../fixtures/raw_block_ingest_synthetic.hex")
        .trim()
        .to_string();
    let raw = decode_hex(&block_hex).unwrap();
    let view = UnverifiedRawBlockView::parse(&raw, None).unwrap();
    let blockhash = id_hex(&view.header_hash_raw());
    (block_hex, blockhash, view.transaction_count().to_string())
}

#[test]
fn bulk_block_stream_recovers_ordered_tl_candidates() {
    let (raw, blockhash, count) = block_fixture();
    let input = format!("B\t10\t{blockhash}\t{raw}\nE\tBLOCKS\t1\n");
    let (ok, out, err) = run(&input);
    assert!(ok, "{err}");
    assert!(out.contains("C\tBLOCK\t10\t"));
    assert!(out.contains("\t1\t") || out.contains("\t2\t"));
    assert!(out.contains(&format!("END\tCOMPLETE_UNVERIFIED\tBLOCKS\t1\t{count}\t2")));
    assert!(err.contains("source=UNVERIFIED"));
}

#[test]
fn wrong_block_hash_and_missing_end_are_rejected() {
    let (raw, _, _) = block_fixture();
    let wrong = format!("B\t0\t{}\t{raw}\nE\tBLOCKS\t1\n", "0".repeat(64));
    let (ok, _, error) = run(&wrong);
    assert!(!ok);
    assert!(error.contains("RequestedHeaderHashMismatch"));

    let (_, blockhash, _) = block_fixture();
    let (ok, _, error) = run(&format!("B\t0\t{blockhash}\t{raw}\n"));
    assert!(!ok);
    assert!(error.contains("missing E sentinel"));
}

#[test]
fn linked_blocks_and_count_are_enforced() {
    let (raw, blockhash, _) = block_fixture();
    let input = format!("B\t0\t{blockhash}\t{raw}\nB\t1\t{blockhash}\t{raw}\nE\tBLOCKS\t2\n");
    let (ok, _, error) = run(&input);
    assert!(!ok);
    assert!(error.contains("previous-hash linkage mismatch"));
    let input = format!("B\t0\t{blockhash}\t{raw}\nE\tBLOCKS\t2\n");
    let (ok, _, error) = run(&input);
    assert!(!ok);
    assert!(error.contains("count mismatch"));
}

#[test]
fn script_history_txid_and_vout_are_independently_checked() {
    let (raw, blockhash, _) = block_fixture();
    let block = decode_hex(&raw).unwrap();
    let view = UnverifiedRawBlockView::parse(&block, None).unwrap();
    let transaction = view.raw_transaction(1).unwrap();
    let txid = id_hex(&inspect_transaction(transaction).unwrap().txid);
    let txhex = transaction
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let input = format!("S\t{txid}\t{blockhash}\t{txhex}\t0\nE\tSCRIPT\t1\n");
    let (ok, out, error) = run(&input);
    assert!(ok, "{error}");
    assert!(out.contains("C\tSCRIPT\t-\t"));
    assert!(out.contains("END\tCOMPLETE_UNVERIFIED\tSCRIPT\t1\t1\t1"));
    let wrong = format!(
        "S\t{}\t{blockhash}\t{txhex}\t0\nE\tSCRIPT\t1\n",
        "0".repeat(64)
    );
    let (ok, _, error) = run(&wrong);
    assert!(!ok);
    assert!(error.contains("TXID mismatch"));
    let wrong = format!("S\t{txid}\t{blockhash}\t{txhex}\t0,0\nE\tSCRIPT\t1\n");
    let (ok, _, error) = run(&wrong);
    assert!(!ok);
    assert!(error.contains("duplicated matching output"));
}

#[test]
fn script_stream_with_zero_matches_is_an_explicit_complete_observation() {
    let (ok, out, err) = run("E\tSCRIPT\t0\n");
    assert!(ok, "{err}");
    assert!(out.contains("END\tCOMPLETE_UNVERIFIED\tSCRIPT\t0\t0\t0"));
}
