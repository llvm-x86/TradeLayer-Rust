//! Stage 24B offline funding math tests, with no chain or ledger access.
use std::collections::BTreeMap;
use tradelayer_core::funding_audit::{
    audit_funding, FundingAuditError, FundingMode, FundingPosition, MAX_AUDIT_POSITIONS,
};

const FIXTURES: &str = include_str!("../fixtures/funding_audit_vectors.tsv");

fn parse_positions(raw: &str) -> Vec<FundingPosition> {
    raw.split(',')
        .map(|item| {
            let parts: Vec<_> = item.split(':').collect();
            assert_eq!(parts.len(), 3);
            FundingPosition {
                account: parts[0].to_string(),
                contracts: parts[1].parse().unwrap(),
                available_atoms: parts[2].parse().unwrap(),
            }
        })
        .collect()
}

fn deltas(audit: &tradelayer_core::funding_audit::FundingProjection) -> String {
    audit.deltas
        .iter()
        .map(|row| format!("{}:{}", row.account, row.delta_atoms))
        .collect::<Vec<_>>()
        .join(",")
}

fn previews(audit: &tradelayer_core::funding_audit::FundingProjection) -> String {
    let balances = audit.simulated_available_atoms.as_ref().unwrap();
    audit.deltas
        .iter()
        .zip(balances)
        .map(|(row, amount)| format!("{}:{amount}", row.account))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn pinned_synthetic_bigint_oracle_matches_both_projections() {
    let mut count = 0;
    for row in FIXTURES.lines() {
        if row.is_empty() || row.starts_with('#') {
            continue;
        }
        let columns: Vec<_> = row.split('\t').collect();
        assert_eq!(columns.len(), 10, "malformed fixture: {row}");
        let name = columns[0];
        let notional = columns[1].parse().unwrap();
        let rate = columns[2].parse().unwrap();
        let positions = parse_positions(columns[3]);
        let snapshot = positions.clone();
        let legacy = audit_funding(&positions, notional, rate, FundingMode::LegacySigned).unwrap();
        assert_eq!(legacy.nominal_atoms.to_string(), columns[4], "{name}");
        assert_eq!(deltas(&legacy), columns[5], "{name}");
        assert_eq!(legacy.net_atoms.to_string(), columns[6], "{name}");
        assert!(legacy.simulated_available_atoms.is_none());

        let proposed = audit_funding(
            &positions,
            notional,
            rate,
            FundingMode::ConservingProposal,
        );
        if let Some(problem) = columns[7].strip_prefix("ERR:") {
            let error = proposed.unwrap_err();
            match problem {
                "MissingCounterparty" => assert_eq!(error, FundingAuditError::MissingCounterparty),
                "InsufficientAvailable" => assert!(matches!(error, FundingAuditError::InsufficientAvailable(_))),
                _ => panic!("unexpected fixture error: {problem}"),
            }
        } else {
            let checked = proposed.unwrap();
            assert_eq!(deltas(&checked), columns[7], "{name}");
            assert_eq!(checked.net_atoms.to_string(), columns[8], "{name}");
            assert_eq!(previews(&checked), columns[9], "{name}");
            let balance_before: u128 = positions.iter().map(|p| p.available_atoms).sum();
            let balance_after: u128 = checked
                .simulated_available_atoms
                .as_ref()
                .unwrap()
                .iter()
                .sum();
            assert_eq!(balance_before, balance_after, "{name}");
        }
        // This audit is pure: the original snapshot and replay markers are
        // never mutated, including on an error path.
        assert_eq!(positions, snapshot);
        count += 1;
    }
    assert_eq!(count, 14);
}

#[test]
fn legacy_negative_rate_reverses_payers_and_can_create_or_burn_atoms() {
    let pair = parse_positions("long:1:100000000,short:-1:100000000");
    let legacy = audit_funding(&pair, 100_000_000, -100_000, FundingMode::LegacySigned).unwrap();
    let proposed = audit_funding(&pair, 100_000_000, -100_000, FundingMode::ConservingProposal).unwrap();
    assert_eq!(deltas(&legacy), "long:-100000,short:100000");
    assert_eq!(deltas(&proposed), "long:100000,short:-100000");

    let positions = parse_positions("shortA:-1:100,shortB:-1:100,shortC:-2:100,longX:4:100");
    let legacy = audit_funding(&positions, 510, -100_000, FundingMode::LegacySigned).unwrap();
    let repaired = audit_funding(&positions, 510, -100_000, FundingMode::ConservingProposal).unwrap();
    assert_eq!(legacy.net_atoms, 1);
    assert_eq!(repaired.net_atoms, 0);
    assert_eq!(repaired.nominal_atoms, 3);
}

#[test]
fn conserving_allocation_has_stable_ascii_tie_breaks_across_input_orders() {
    let a = parse_positions("payer:2:1000,shortZ:-1:1000,shortA:-1:1000");
    let b = parse_positions("shortZ:-1:1000,payer:2:1000,shortA:-1:1000");
    let pa = audit_funding(&a, 510, 100_000, FundingMode::ConservingProposal).unwrap();
    let pb = audit_funding(&b, 510, 100_000, FundingMode::ConservingProposal).unwrap();
    let map = |plan: &tradelayer_core::funding_audit::FundingProjection| -> BTreeMap<String, i128> {
        plan.deltas
            .iter()
            .map(|row| (row.account.clone(), row.delta_atoms))
            .collect()
    };
    assert_eq!(map(&pa), map(&pb));
    assert_eq!(map(&pa)["shortA"], 1);
    assert_eq!(map(&pa)["shortZ"], 0);
}

#[test]
fn every_signed_rate_conserves_proposed_balances_on_bounded_grid() {
    for payers in 1i64..=7 {
        for receivers in 1i64..=7 {
            for rate in [-125_000, -100_000, -1, 0, 1, 100_000, 125_000] {
                let p = vec![
                    FundingPosition {
                        account: "alice".into(),
                        contracts: payers,
                        available_atoms: 1_000_000,
                    },
                    FundingPosition {
                        account: "bob".into(),
                        contracts: -receivers,
                        available_atoms: 1_000_000,
                    },
                ];
                let proposed =
                    audit_funding(&p, 9_001, rate, FundingMode::ConservingProposal).unwrap();
                assert_eq!(proposed.net_atoms, 0);
                let before: u128 = p.iter().map(|item| item.available_atoms).sum();
                let after: u128 = proposed
                    .simulated_available_atoms
                    .unwrap()
                    .iter()
                    .sum();
                assert_eq!(before, after);
                if rate < 0 && proposed.nominal_atoms > 0 {
                    assert!(proposed.deltas[1].delta_atoms < 0);
                    assert!(proposed.deltas[0].delta_atoms > 0);
                }
            }
        }
    }
}

#[test]
fn adversarial_inputs_fail_closed_without_side_effects() {
    assert_eq!(
        audit_funding(
            &parse_positions("alice:1:5,alice:-1:5"),
            100,
            10_000,
            FundingMode::ConservingProposal,
        ),
        Err(FundingAuditError::DuplicateAccount)
    );
    assert_eq!(
        audit_funding(
            &parse_positions("long:1:1000"),
            100_000_000,
            100_000,
            FundingMode::ConservingProposal,
        ),
        Err(FundingAuditError::MissingCounterparty)
    );
    assert!(matches!(
        audit_funding(
            &parse_positions("long:1:0,short:-1:100000"),
            100_000_000,
            100_000,
            FundingMode::ConservingProposal,
        ),
        Err(FundingAuditError::InsufficientAvailable(_))
    ));
    let too_many = vec![
        FundingPosition {
            account: "same".into(),
            contracts: 1,
            available_atoms: 0,
        };
        MAX_AUDIT_POSITIONS + 1
    ];
    assert_eq!(
        audit_funding(&too_many, 1, 1, FundingMode::LegacySigned),
        Err(FundingAuditError::TooManyPositions)
    );
    let extremes = parse_positions("long:9223372036854775807:0,short:-9223372036854775807:0");
    assert_eq!(
        audit_funding(&extremes, u64::MAX, i64::MAX, FundingMode::ConservingProposal),
        Err(FundingAuditError::ArithmeticOverflow)
    );
}
