//! Source-characterization regression tests for legacy clearing call cadence.
use tradelayer_core::clearing_cadence::{
    funding_bucket, project_end_block, CadenceError, ReplayPath, TxObservation,
    MAX_OBSERVED_TRANSACTIONS,
};

const FIXTURES: &str = include_str!("../fixtures/clearing_cadence_vectors.tsv");

fn parse_txs(raw: &str) -> Vec<TxObservation> {
    if raw.is_empty() {
        return Vec::new();
    }
    raw.split(',')
        .map(|entry| {
            let fields: Vec<_> = entry.split(':').collect();
            assert_eq!(fields.len(), 3);
            TxObservation {
                tx_type: fields[0].parse().unwrap(),
                already_processed: fields[1] == "1",
                valid_before_logic: fields[2] == "1",
            }
        })
        .collect()
}

fn enabled(input: &str) -> bool {
    match input {
        "1" => true,
        "0" => false,
        _ => panic!("bad boolean vector: {input}"),
    }
}

#[test]
fn source_independent_node_vectors_match_realtime_and_catchup() {
    let mut rows = 0usize;
    for line in FIXTURES.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 10, "bad fixture: {line}");
        let path = match fields[1] {
            "catchup" => ReplayPath::CatchUp,
            "realtime" => ReplayPath::RealTime,
            _ => panic!("unknown replay path"),
        };
        let projection =
            project_end_block(path, fields[2].parse().unwrap(), &parse_txs(fields[3])).unwrap();
        let label = fields[0];
        assert_eq!(
            projection.funding_bucket,
            fields[4].parse().unwrap(),
            "{label}"
        );
        assert_eq!(
            projection.other_bucket,
            fields[5].parse().unwrap(),
            "{label}"
        );
        assert_eq!(projection.clearing_called, enabled(fields[6]), "{label}");
        assert_eq!(
            projection.funding_height_eligible,
            enabled(fields[7]),
            "{label}"
        );
        assert_eq!(projection.funding_attempted, enabled(fields[8]), "{label}");
        assert_eq!(
            projection.withdrawals_attempted,
            enabled(fields[9]),
            "{label}"
        );
        rows += 1;
    }
    assert_eq!(rows, 32);
}

#[test]
fn realtime_literal_m_buckets_type22_while_catchup_buckets_type20() {
    assert!(funding_bucket(ReplayPath::RealTime, 22));
    assert!(!funding_bucket(ReplayPath::RealTime, 20));
    assert!(!funding_bucket(ReplayPath::CatchUp, 22));
    assert!(funding_bucket(ReplayPath::CatchUp, 20));
    assert_eq!(u8::from_str_radix("m", 36).unwrap(), 22);
    assert_eq!(u8::from_str_radix("k", 36).unwrap(), 20);
}

#[test]
fn funding_only_or_invalid_other_trades_trigger_different_clearing() {
    for ty in [4, 22] {
        let txs = [TxObservation {
            tx_type: ty,
            already_processed: false,
            valid_before_logic: true,
        }];
        let backfill = project_end_block(ReplayPath::CatchUp, 240, &txs).unwrap();
        let realtime = project_end_block(ReplayPath::RealTime, 240, &txs).unwrap();
        assert!(backfill.funding_attempted);
        assert!(!realtime.funding_attempted);
        assert!(realtime.withdrawals_attempted);
    }
    let invalid = [TxObservation {
        tx_type: 18,
        already_processed: false,
        valid_before_logic: false,
    }];
    assert!(
        project_end_block(ReplayPath::CatchUp, 240, &invalid)
            .unwrap()
            .funding_attempted
    );
    assert!(
        !project_end_block(ReplayPath::RealTime, 240, &invalid)
            .unwrap()
            .funding_attempted
    );
}

#[test]
fn scheduled_empty_block_only_forces_catchup_at_ten_thousand_multiples() {
    for height in [24u64, 240, 480, 960, 9_984, 10_000, 30_000, 60_000] {
        let catchup = project_end_block(ReplayPath::CatchUp, height, &[]).unwrap();
        let realtime = project_end_block(ReplayPath::RealTime, height, &[]).unwrap();
        assert_eq!(catchup.clearing_called, height % 10_000 == 0);
        assert!(!realtime.clearing_called);
        assert_eq!(
            catchup.funding_attempted,
            height % 10_000 == 0 && height % 24 == 0
        );
        assert!(!realtime.funding_attempted);
        assert!(!catchup.withdrawals_attempted || catchup.clearing_called);
        assert!(realtime.withdrawals_attempted);
    }
}

#[test]
fn invalid_inputs_are_rejected_without_truncation() {
    assert_eq!(
        project_end_block(ReplayPath::CatchUp, 0, &[]),
        Err(CadenceError::InvalidHeight)
    );
    let unsupported = [TxObservation {
        tx_type: 36,
        already_processed: false,
        valid_before_logic: true,
    }];
    assert_eq!(
        project_end_block(ReplayPath::RealTime, 24, &unsupported),
        Err(CadenceError::UnknownTransactionType)
    );
    let excess = vec![
        TxObservation {
            tx_type: 4,
            already_processed: false,
            valid_before_logic: true,
        };
        MAX_OBSERVED_TRANSACTIONS + 1
    ];
    assert_eq!(
        project_end_block(ReplayPath::CatchUp, 24, &excess),
        Err(CadenceError::TooManyInputs)
    );
}
