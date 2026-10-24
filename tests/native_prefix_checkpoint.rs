//! Executable Stage 18 boundary tests for native-key sorted Vec storage.
//! Uses synthetic transaction IDs and unauthenticated actor identifiers.

use std::collections::BTreeMap;
use tradelayer_core::ledger::{
    AccountId, Ledger, Operation, PropertyId, PropertyKind, SupplyColumnSource, TxId,
};

fn account(name: &str) -> AccountId {
    AccountId::try_from(name).expect("valid printable ASCII account identifier")
}

fn tx(number: u64) -> TxId {
    let mut data = [0u8; 32];
    data[24..].copy_from_slice(&number.to_be_bytes());
    TxId::from_bytes(data)
}

fn available_rows(ledger: &Ledger, property: PropertyId) -> BTreeMap<String, u128> {
    ledger
        .supply_column_rows_unverified()
        .expect("valid accounting columns")
        .into_iter()
        .filter_map(|row| {
            if row.property != property {
                return None;
            }
            match row.source {
                SupplyColumnSource::Available(holder) => {
                    assert!(row.atoms > 0, "canonical store must prune zero balances");
                    Some((holder.as_str().to_owned(), row.atoms))
                }
                _ => None,
            }
        })
        .collect()
}

#[test]
fn every_account_prefix_length_is_sorted_pruned_and_snapshot_stable() {
    // All 90 accepted identifier lengths are exercised under the SAME
    // maximal property ID. Every shorter name prefixes longer names, so
    // this catches lexicographic/prefix mistakes missed by mixed-property
    // test oracles. Issuer sorts after all these prefix accounts.
    let id = PropertyId::try_from(u32::MAX).unwrap();
    let issuer = account("zIssuer");
    let mut state = Ledger::new();
    state
        .apply(
            tx(1),
            Operation::CreateProperty {
                id,
                issuer: issuer.clone(),
                kind: PropertyKind::Fixed,
                initial_atoms: 100_000,
            },
        )
        .unwrap();

    let mut reference = BTreeMap::from([("zIssuer".to_owned(), 100_000u128)]);
    for width in 1..=90usize {
        let name = "A".repeat(width);
        let quantity = width as u128 + 2;
        state
            .apply(
                tx(100 + width as u64),
                Operation::Transfer {
                    id,
                    from: issuer.clone(),
                    to: account(&name),
                    amount_atoms: quantity,
                },
            )
            .unwrap();
        *reference.get_mut("zIssuer").unwrap() -= quantity;
        reference.insert(name.clone(), quantity);
        assert_eq!(state.available_atoms(id, &account(&name)), quantity);
    }
    assert_eq!(available_rows(&state, id), reference);
    assert_eq!(state.verify_conservation(), Ok(()));
    let first_snapshot = state.snapshot_bytes().unwrap();
    let recovered = Ledger::from_snapshot_bytes(&first_snapshot).unwrap();
    assert_eq!(recovered, state);
    assert_eq!(recovered.snapshot_bytes().unwrap(), first_snapshot);

    // Remove half the support, including the first and next-to-longest
    // odd-width entries. Exercise zero pruning with an interleaved set of
    // proper-prefix names still present, then serialize again.
    for width in (1..=90usize).step_by(2) {
        let name = "A".repeat(width);
        let quantity = width as u128 + 2;
        state
            .apply(
                tx(1000 + width as u64),
                Operation::Transfer {
                    id,
                    from: account(&name),
                    to: issuer.clone(),
                    amount_atoms: quantity,
                },
            )
            .unwrap();
        *reference.get_mut("zIssuer").unwrap() += quantity;
        reference.remove(&name);
        assert_eq!(state.available_atoms(id, &account(&name)), 0);
        assert_eq!(available_rows(&state, id), reference);
    }
    assert_eq!(state.verify_conservation(), Ok(()));
    let final_snapshot = state.snapshot_bytes().unwrap();
    let restored = Ledger::from_snapshot_bytes(&final_snapshot).unwrap();
    assert_eq!(restored, state);
    assert_eq!(restored.snapshot_bytes().unwrap(), final_snapshot);
    assert_eq!(available_rows(&restored, id), reference);
}
