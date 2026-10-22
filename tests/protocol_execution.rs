//! Synthetic-only integration tests of ordered registry + ledger execution.
//! No signature ownership or L1 chain inclusion is established by this suite.

use tradelayer_core::chain::ChainId;
use tradelayer_core::execution::{ExecutionError, ProtocolOperation, ProtocolState, UnverifiedTx};
use tradelayer_core::ledger::{AccountId, LedgerError, PropertyId, PropertyKind, TxId};
use tradelayer_core::registry::{AdminTarget, RegistryAction, RegistryError};

fn a(value: &str) -> AccountId {
    AccountId::try_from(value).unwrap()
}
fn p(value: u32) -> PropertyId {
    PropertyId::try_from(value).unwrap()
}
fn txid(value: u8) -> TxId {
    TxId::from_bytes([value; 32])
}
fn chain() -> ChainId {
    ChainId([9; 32])
}
fn state() -> ProtocolState {
    ProtocolState::new_unverified(chain(), a("protocol-admin"))
}
fn tx(n: u8, actor: &str, height: u64, operation: ProtocolOperation) -> UnverifiedTx {
    UnverifiedTx {
        chain: chain(),
        txid: txid(n),
        claimed_actor: a(actor),
        claimed_height: height,
        operation,
    }
}
fn activate(types: &[u8]) -> ProtocolOperation {
    ProtocolOperation::Registry(RegistryAction::Activate {
        types: types.to_vec(),
        code_hash: [3; 32],
    })
}
fn issue(kind: PropertyKind, units: u128) -> ProtocolOperation {
    ProtocolOperation::Issue {
        id: p(5),
        kind,
        initial_atoms: units,
    }
}
fn send(from_amount: u128) -> ProtocolOperation {
    ProtocolOperation::Send {
        id: p(5),
        recipient: a("bob"),
        amount_atoms: from_amount,
    }
}
fn grant(to: &str, amount: u128) -> ProtocolOperation {
    ProtocolOperation::Grant {
        id: p(5),
        recipient: a(to),
        amount_atoms: amount,
    }
}
fn redeem(amount: u128) -> ProtocolOperation {
    ProtocolOperation::Redeem {
        id: p(5),
        amount_atoms: amount,
    }
}
fn bootstrap(types: &[u8]) -> ProtocolState {
    let mut s = state();
    s.apply_unverified(tx(1, "protocol-admin", 100, activate(types)))
        .unwrap();
    s
}

#[test]
fn cross_module_activation_issue_transfer_and_supply_conservation() {
    let mut s = bootstrap(&[0, 1, 2, 11, 12]);
    s.apply_batch_unverified([
        tx(2, "alice", 101, issue(PropertyKind::Managed, 100)),
        tx(3, "alice", 101, send(29)),
        tx(4, "alice", 102, grant("alice", 12)),
        tx(5, "alice", 103, redeem(3)),
    ])
    .unwrap();
    assert_eq!(s.ledger().available_atoms(p(5), &a("alice")), 80);
    assert_eq!(s.ledger().available_atoms(p(5), &a("bob")), 29);
    assert_eq!(s.ledger().property(p(5)).unwrap().total_supply_atoms, 109);
    s.ledger().verify_conservation().unwrap();
    assert_eq!(s.processed_count(), 5);
    assert_eq!(s.ledger().processed_count(), 4);
    assert_eq!(s.registry().processed_count(), 1);
}

#[test]
fn activation_is_a_strictly_earlier_block() {
    let mut s = state();
    let before = s.clone();
    assert_eq!(
        s.apply_unverified(tx(1, "alice", 100, issue(PropertyKind::Fixed, 100))),
        Err(ExecutionError::NotActivated(1))
    );
    assert_eq!(s, before);
    s.apply_unverified(tx(1, "protocol-admin", 100, activate(&[1, 2])))
        .unwrap();
    assert_eq!(
        s.apply_unverified(tx(2, "alice", 100, issue(PropertyKind::Fixed, 100))),
        Err(ExecutionError::SameBlockActivation(1))
    );
    assert_eq!(s.ledger().processed_count(), 0);
    s.apply_unverified(tx(2, "alice", 101, issue(PropertyKind::Fixed, 100)))
        .unwrap();
}

#[test]
fn replay_is_global_across_registry_and_ledger() {
    let mut s = bootstrap(&[1, 2, 7]);
    let before = s.clone();
    assert_eq!(
        s.apply_unverified(tx(1, "alice", 101, issue(PropertyKind::Fixed, 20))),
        Err(ExecutionError::Replay(txid(1)))
    );
    assert_eq!(s, before);
    s.apply_unverified(tx(2, "alice", 101, issue(PropertyKind::Fixed, 20)))
        .unwrap();
    let before = s.clone();
    assert_eq!(
        s.apply_unverified(tx(
            2,
            "alice",
            102,
            ProtocolOperation::Registry(RegistryAction::CreateWhitelist {
                name: "KYC".into(),
                url: String::new(),
                description: String::new(),
                backup: a("backup"),
            })
        )),
        Err(ExecutionError::Replay(txid(2)))
    );
    assert_eq!(s, before);
}

#[test]
fn mixed_batch_failure_restores_both_modules_and_global_replay() {
    let mut s = bootstrap(&[1, 2, 7]);
    let before = s.clone();
    let create_list = ProtocolOperation::Registry(RegistryAction::CreateWhitelist {
        name: "KYC".into(),
        url: String::new(),
        description: String::new(),
        backup: a("backup"),
    });
    let batch = [
        tx(2, "owner", 101, create_list.clone()),
        tx(3, "alice", 101, issue(PropertyKind::Fixed, 100)),
        tx(4, "bob", 101, send(50)), // Unauthorized origin has no funds.
    ];
    assert!(matches!(
        s.apply_batch_unverified(batch),
        Err(ExecutionError::Ledger(
            LedgerError::InsufficientAvailable { .. }
        ))
    ));
    assert_eq!(s, before);
    s.apply_batch_unverified([
        tx(2, "owner", 101, create_list),
        tx(3, "alice", 101, issue(PropertyKind::Fixed, 100)),
        tx(4, "alice", 102, send(50)),
    ])
    .unwrap();
    assert_eq!(s.registry().whitelist_count(), 1);
    assert_eq!(s.ledger().available_atoms(p(5), &a("bob")), 50);
}

#[test]
fn fixed_supply_rejects_grants_and_redemptions() {
    let mut s = bootstrap(&[1, 11, 12]);
    s.apply_unverified(tx(2, "alice", 101, issue(PropertyKind::Fixed, 20)))
        .unwrap();
    let before = s.clone();
    assert_eq!(
        s.apply_unverified(tx(3, "alice", 102, grant("alice", 5))),
        Err(ExecutionError::Ledger(LedgerError::FixedSupply(p(5))))
    );
    assert_eq!(
        s.apply_unverified(tx(3, "alice", 102, redeem(1))),
        Err(ExecutionError::Ledger(LedgerError::FixedSupply(p(5))))
    );
    assert_eq!(s, before);
}

#[test]
fn issuer_permissions_and_balance_accounting_are_retained() {
    let mut s = bootstrap(&[1, 2, 11, 12]);
    s.apply_unverified(tx(2, "alice", 101, issue(PropertyKind::Managed, 12)))
        .unwrap();
    assert!(matches!(
        s.apply_unverified(tx(3, "mallory", 102, grant("mallory", 5))),
        Err(ExecutionError::Ledger(LedgerError::UnauthorizedIssuer(_)))
    ));
    assert!(matches!(
        s.apply_unverified(tx(3, "mallory", 102, redeem(5))),
        Err(ExecutionError::Ledger(LedgerError::UnauthorizedIssuer(_)))
    ));
    s.apply_unverified(tx(3, "alice", 102, send(5))).unwrap();
    assert!(matches!(
        s.apply_unverified(tx(4, "alice", 103, redeem(8))),
        Err(ExecutionError::Ledger(
            LedgerError::InsufficientAvailable { .. }
        ))
    ));
    assert_eq!(s.ledger().property(p(5)).unwrap().total_supply_atoms, 12);
}

#[test]
fn simultaneous_registry_activity_and_balances_fork_deterministically() {
    let mut s = bootstrap(&[1, 2, 7, 8, 9, 13, 14]);
    s.apply_unverified(tx(2, "alice", 101, issue(PropertyKind::Fixed, 100)))
        .unwrap();
    let branch = s.clone();
    s.apply_batch_unverified([
        tx(3, "alice", 102, send(7)),
        tx(
            4,
            "bob",
            102,
            ProtocolOperation::Registry(RegistryAction::CreateOracle {
                ticker: "BTCUSD".into(),
                url: String::new(),
                backup: a("backup"),
                lag: 1,
            }),
        ),
    ])
    .unwrap();
    assert_eq!(s.ledger().available_atoms(p(5), &a("bob")), 7);
    assert_eq!(s.registry().oracle_count(), 1);
    assert_eq!(branch.registry().oracle_count(), 0);
    assert_eq!(branch.ledger().available_atoms(p(5), &a("bob")), 0);
    let mut rebuilt = branch;
    rebuilt
        .apply_batch_unverified([
            tx(3, "alice", 102, send(7)),
            tx(
                4,
                "bob",
                102,
                ProtocolOperation::Registry(RegistryAction::CreateOracle {
                    ticker: "BTCUSD".into(),
                    url: String::new(),
                    backup: a("backup"),
                    lag: 1,
                }),
            ),
        ])
        .unwrap();
    assert_eq!(s, rebuilt);
}

#[test]
fn cross_chain_spoof_height_regression_and_empty_batches() {
    let mut s = bootstrap(&[1]);
    let pristine = s.clone();
    let mut foreign = tx(2, "alice", 101, issue(PropertyKind::Fixed, 3));
    foreign.chain = ChainId([1; 32]);
    assert_eq!(s.apply_unverified(foreign), Err(ExecutionError::WrongChain));
    assert_eq!(
        s.apply_batch_unverified(Vec::new()),
        Err(ExecutionError::EmptyBatch)
    );
    assert_eq!(
        s.apply_unverified(tx(2, "alice", 0, issue(PropertyKind::Fixed, 3))),
        Err(ExecutionError::InvalidHeight)
    );
    assert_eq!(s, pristine);
    s.apply_unverified(tx(2, "alice", 101, issue(PropertyKind::Fixed, 3)))
        .unwrap();
    assert_eq!(
        s.apply_unverified(tx(3, "alice", 100, issue(PropertyKind::Fixed, 3))),
        Err(ExecutionError::HeightRegression)
    );
}

#[test]
fn token_admin_rotation_is_explicitly_rejected_through_coordinator() {
    let mut s = bootstrap(&[1, 8]);
    s.apply_unverified(tx(2, "alice", 101, issue(PropertyKind::Managed, 1)))
        .unwrap();
    let before = s.clone();
    assert_eq!(
        s.apply_unverified(tx(
            3,
            "alice",
            102,
            ProtocolOperation::Registry(RegistryAction::UpdateAdmin {
                target: AdminTarget::Token(5),
                new_address: a("mallory"),
                update_backup: false,
            })
        )),
        Err(ExecutionError::Registry(
            RegistryError::UnsupportedTokenAdmin
        ))
    );
    assert_eq!(s, before);
}

#[test]
fn type34_collision_stays_unactivatable() {
    let mut s = state();
    assert_eq!(
        s.apply_unverified(tx(1, "protocol-admin", 100, activate(&[34]))),
        Err(ExecutionError::Registry(RegistryError::ConflictingType34))
    );
    assert_eq!(s.processed_count(), 0);
}

#[test]
fn registry_activation_in_same_batch_requires_later_height() {
    let mut s = state();
    let pre = s.clone();
    let ops = [
        tx(1, "protocol-admin", 100, activate(&[1])),
        tx(2, "alice", 100, issue(PropertyKind::Fixed, 2)),
    ];
    assert_eq!(
        s.apply_batch_unverified(ops),
        Err(ExecutionError::SameBlockActivation(1))
    );
    assert_eq!(s, pre);
    s.apply_batch_unverified([
        tx(1, "protocol-admin", 100, activate(&[1])),
        tx(2, "alice", 101, issue(PropertyKind::Fixed, 2)),
    ])
    .unwrap();
    assert_eq!(s.processed_count(), 2);
}
