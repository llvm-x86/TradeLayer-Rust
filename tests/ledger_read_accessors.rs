//! Read accessors used by a future RPC layer. They only expose what the
//! ledger already stores; none of them can change state.

use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};

fn p(number: u32) -> PropertyId {
    PropertyId::try_from(number).unwrap()
}

fn a(address: &str) -> AccountId {
    AccountId::try_from(address).unwrap()
}

fn tx(number: u64) -> TxId {
    let mut bytes = [0_u8; 32];
    bytes[24..].copy_from_slice(&number.to_be_bytes());
    TxId::from_bytes(bytes)
}

fn seeded() -> Ledger {
    let mut ledger = Ledger::new();
    let ops = [
        Operation::CreateProperty {
            id: p(7),
            issuer: a("alice"),
            kind: PropertyKind::Fixed,
            initial_atoms: 1_000,
        },
        Operation::CreateProperty {
            id: p(2),
            issuer: a("bob"),
            kind: PropertyKind::Fixed,
            initial_atoms: 500,
        },
        Operation::Transfer {
            id: p(7),
            from: a("alice"),
            to: a("carol"),
            amount_atoms: 250,
        },
        Operation::Transfer {
            id: p(2),
            from: a("bob"),
            to: a("carol"),
            amount_atoms: 5,
        },
    ];
    for (i, op) in ops.into_iter().enumerate() {
        ledger.apply(tx(i as u64 + 1), op).unwrap();
    }
    ledger
}

#[test]
fn properties_are_listed_in_ascending_id_order() {
    let ledger = seeded();
    let ids: Vec<u32> = ledger.properties().map(|(id, _)| id.get()).collect();
    assert_eq!(ids, vec![2, 7]);
    assert_eq!(ledger.property_count(), 2);
}

#[test]
fn an_account_lists_exactly_its_own_balances() {
    let ledger = seeded();
    let carol = ledger.available_balances_of(&a("carol"));
    assert_eq!(carol, vec![(p(2), 5), (p(7), 250)]);
    for (id, atoms) in &carol {
        assert_eq!(ledger.available_atoms(*id, &a("carol")), *atoms);
    }
    assert!(ledger.available_balances_of(&a("nobody")).is_empty());
}

#[test]
fn a_similar_account_name_is_not_confused_with_the_account() {
    let ledger = seeded();
    // "carol" must not match "carol2" or "caro".
    assert!(ledger.available_balances_of(&a("caro")).is_empty());
    assert!(ledger.available_balances_of(&a("carol2")).is_empty());
}

#[test]
fn holders_are_one_property_only_and_sum_to_supply() {
    let ledger = seeded();
    let holders = ledger.holders_of(p(7));
    let names: Vec<&str> = holders.iter().map(|(o, _)| o.as_str()).collect();
    assert_eq!(names, vec!["alice", "carol"]);
    let total: u128 = holders.iter().map(|(_, n)| *n).sum();
    assert_eq!(total, ledger.property(p(7)).unwrap().total_supply_atoms);
    assert!(ledger.holders_of(p(99)).is_empty());
}

#[test]
fn accessors_agree_with_a_snapshot_round_trip() {
    let ledger = seeded();
    let restored = Ledger::from_snapshot_bytes(&ledger.snapshot_bytes().unwrap()).unwrap();
    assert_eq!(
        ledger.available_balances_of(&a("carol")),
        restored.available_balances_of(&a("carol"))
    );
    assert_eq!(ledger.holders_of(p(2)), restored.holders_of(p(2)));
}
