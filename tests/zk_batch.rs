//! Cross-repo fixture generated from zkTL's committed checkpoint sample.
use tradelayer_core::zk_batch::{parse_bridge_tsv, ClaimError, MAX_RECEIPTS};

const REAL_ZKTL_FIXTURE: &str = include_str!("../fixtures/zkTL_sample_batch.tsv");

#[test]
fn cross_repo_canonical_commitments_agree() {
    let claim = parse_bridge_tsv(REAL_ZKTL_FIXTURE).expect("parsed source-derived claims");
    claim.inspect_strict().expect("metadata hash agreement");
    assert_eq!(claim.receipts.len(), 26);
    assert_eq!(claim.start_height, 132_900);
    assert_eq!(claim.end_height, 132_907);
    assert_eq!(claim.batch_id.hex(), "ffa178c4297787ed947e6a2b79816f6f2755f0cb5e249e32ea75817dc514187d");
}

#[test]
fn mutated_receipt_and_batch_are_rejected() {
    let mut claim = parse_bridge_tsv(REAL_ZKTL_FIXTURE).unwrap();
    claim.receipts[0].economic_mutation = false;
    assert_eq!(claim.inspect_strict(), Err(ClaimError::Mismatch("receiptId")));
    let mut claim = parse_bridge_tsv(REAL_ZKTL_FIXTURE).unwrap();
    claim.batch_id = claim.receipts[0].receipt_id;
    assert_eq!(claim.inspect_strict(), Err(ClaimError::Mismatch("batchId")));
}

#[test]
fn never_accept_audit_only_or_colliding_type() {
    let mut claim = parse_bridge_tsv(REAL_ZKTL_FIXTURE).unwrap();
    claim.receipts[0].support = tradelayer_core::zk_batch::SupportLevel::AuditReplay;
    assert_eq!(claim.inspect_strict(), Err(ClaimError::AuditOnly));
    let mut claim = parse_bridge_tsv(REAL_ZKTL_FIXTURE).unwrap();
    claim.receipts[0].tx_type = tradelayer_core::TransactionType::try_from(34).unwrap();
    assert_eq!(claim.inspect_strict(), Err(ClaimError::ConflictingTx34));
}

#[test]
fn duplicate_transaction_and_root_discontinuity_are_rejected() {
    let mut claim = parse_bridge_tsv(REAL_ZKTL_FIXTURE).unwrap();
    claim.receipts[1].txid = claim.receipts[0].txid;
    assert_eq!(claim.inspect_strict(), Err(ClaimError::Invalid("duplicate transaction ID")));
    let mut claim = parse_bridge_tsv(REAL_ZKTL_FIXTURE).unwrap();
    claim.receipts[1].old_state_root = claim.first_state_root;
    assert_eq!(claim.inspect_strict(), Err(ClaimError::Mismatch("root chain")));
}

#[test]
fn malformed_rows_and_type_36_are_rejected() {
    assert!(parse_bridge_tsv("").is_err());
    let mutation = REAL_ZKTL_FIXTURE.replacen("\t0\t132900\t", "\t36\t132900\t", 1);
    assert!(parse_bridge_tsv(&mutation).is_err());
    assert!(parse_bridge_tsv(&"x".repeat(2_000_001)).is_err());
    assert_eq!(MAX_RECEIPTS, 10_000);
}
