//! Pure transition tests. These use synthetic actors; no signature or block
//! verification occurs. They cannot establish validity on a live chain.

use tradelayer_core::ledger::{AccountId, TxId};
use tradelayer_core::registry::{
    AdminTarget, Registry, RegistryAction, RegistryError, RegistryTx, MAX_METADATA_BYTES,
};

fn a(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn id(n: u8) -> TxId {
    TxId::from_bytes([n; 32])
}
fn tx(n: u8, actor: &str, height: u64, action: RegistryAction) -> RegistryTx {
    RegistryTx {
        txid: id(n),
        actor: a(actor),
        height,
        action,
    }
}
fn activate(types: &[u8]) -> RegistryAction {
    RegistryAction::Activate {
        types: types.to_vec(),
        code_hash: [0xAB; 32],
    }
}
fn initial() -> Registry {
    Registry::new(a("genesis-admin"))
}
fn boot() -> Registry {
    let mut state = initial();
    state
        .apply(tx(
            1,
            "genesis-admin",
            100,
            activate(&[0, 7, 8, 9, 13, 14, 15]),
        ))
        .unwrap();
    state
}
fn create_list(name: &str) -> RegistryAction {
    RegistryAction::CreateWhitelist {
        name: name.into(),
        url: "https://example.com".into(),
        description: "test policy".into(),
        backup: a("backup"),
    }
}
fn create_oracle() -> RegistryAction {
    RegistryAction::CreateOracle {
        ticker: "LTCUSD".into(),
        url: "https://oracle.example".into(),
        backup: a("backup"),
        lag: 1,
    }
}
fn attest(list_id: u32, target: &str) -> RegistryAction {
    RegistryAction::Attest {
        list_id,
        subject: a(target),
        metadata: "KYC".into(),
    }
}
fn price(oracle_id: u32, quote: u128) -> RegistryAction {
    RegistryAction::PublishOracle {
        oracle_id,
        price_1e4: quote,
        high_1e4: Some(quote + 10),
        low_1e4: Some(quote.saturating_sub(10)),
        close_1e4: Some(quote),
    }
}

#[test]
fn activation_is_admin_only_and_requires_later_block() {
    let mut s = initial();
    let pristine = s.clone();
    assert_eq!(
        s.apply(tx(1, "intruder", 100, activate(&[7]))),
        Err(RegistryError::Unauthorized)
    );
    assert_eq!(s, pristine);
    s.apply(tx(1, "genesis-admin", 100, activate(&[0, 7])))
        .unwrap();
    assert_eq!(s.activation(7).unwrap().height, 100);
    assert_eq!(s.activation(7).unwrap().code_hash, [0xAB; 32]);
    assert_eq!(
        s.apply(tx(2, "owner", 100, create_list("KYC"))),
        Err(RegistryError::SameBlockActivation(7))
    );
    s.apply(tx(2, "owner", 101, create_list("KYC"))).unwrap();
    assert_eq!(s.whitelist_count(), 1);
    assert_eq!(s.whitelist(1).unwrap().admin, a("owner"));
}

#[test]
fn activation_rejects_type34_and_repeated_and_out_of_range() {
    let mut s = initial();
    let actions = [
        (activate(&[34]), RegistryError::ConflictingType34),
        (activate(&[7, 7]), RegistryError::DuplicateActivation(7)),
        (activate(&[36]), RegistryError::InvalidType(36)),
        (activate(&[]), RegistryError::EmptyActivation),
    ];
    for (i, (action, err)) in actions.into_iter().enumerate() {
        assert_eq!(
            s.apply(tx((i + 1) as u8, "genesis-admin", 100, action)),
            Err(err)
        );
        assert_eq!(s, initial());
    }
    s.apply(tx(9, "genesis-admin", 100, activate(&[7])))
        .unwrap();
    let before = s.clone();
    assert_eq!(
        s.apply(tx(10, "genesis-admin", 101, activate(&[7, 8]))),
        Err(RegistryError::AlreadyActivated(7))
    );
    assert_eq!(s, before);
}

#[test]
fn atomic_batch_rolls_back_and_does_not_poison_replay_markers() {
    let mut s = boot();
    let initial = s.clone();
    let batch = vec![
        tx(2, "owner", 101, create_list("KYC")),
        tx(3, "rogue", 101, attest(1, "bob")),
    ];
    assert_eq!(s.apply_batch(batch), Err(RegistryError::Unauthorized));
    assert_eq!(s, initial);
    assert_eq!(s.processed_count(), 1);
    s.apply_batch([
        tx(2, "owner", 101, create_list("KYC")),
        tx(3, "owner", 101, attest(1, "bob")),
    ])
    .unwrap();
    assert!(s.attestation(1, &a("bob")).unwrap().active);
    assert_eq!(s.processed_count(), 3);
}

#[test]
fn replay_and_height_regression_fail_without_mutation() {
    let mut s = boot();
    s.apply(tx(2, "owner", 101, create_list("KYC"))).unwrap();
    let before = s.clone();
    assert_eq!(
        s.apply(tx(2, "owner", 101, create_list("Other"))),
        Err(RegistryError::Replay(id(2)))
    );
    assert_eq!(
        s.apply(tx(3, "owner", 100, create_list("Other"))),
        Err(RegistryError::HeightRegression)
    );
    assert_eq!(s, before);
}

#[test]
fn whitelist_attestation_revocation_and_reissuance() {
    let mut s = boot();
    s.apply(tx(2, "owner", 101, create_list("KYC"))).unwrap();
    s.apply(tx(3, "owner", 101, attest(1, "bob"))).unwrap();
    assert_eq!(
        s.apply(tx(4, "owner", 102, attest(1, "bob"))),
        Err(RegistryError::ExistingAttestation)
    );
    s.apply(tx(
        4,
        "owner",
        102,
        RegistryAction::RevokeAttestation {
            list_id: 1,
            subject: a("bob"),
        },
    ))
    .unwrap();
    let old = s.attestation(1, &a("bob")).unwrap();
    assert_eq!(old.updated_height, 102);
    assert!(!old.active);
    s.apply(tx(5, "owner", 103, attest(1, "bob"))).unwrap();
    assert_eq!(s.attestation(1, &a("bob")).unwrap().issued_height, 103);
    assert!(s.attestation(1, &a("bob")).unwrap().active);
}

#[test]
fn primary_and_backup_can_rotate_admin_but_only_primary_attests() {
    let mut s = boot();
    s.apply(tx(2, "owner", 101, create_list("KYC"))).unwrap();
    assert_eq!(
        s.apply(tx(3, "backup", 102, attest(1, "bob"))),
        Err(RegistryError::Unauthorized)
    );
    s.apply(tx(
        3,
        "backup",
        102,
        RegistryAction::UpdateAdmin {
            target: AdminTarget::Whitelist(1),
            new_address: a("new-admin"),
            update_backup: false,
        },
    ))
    .unwrap();
    assert_eq!(s.whitelist(1).unwrap().admin, a("new-admin"));
    assert_eq!(
        s.apply(tx(4, "owner", 103, attest(1, "bob"))),
        Err(RegistryError::Unauthorized)
    );
    s.apply(tx(4, "new-admin", 103, attest(1, "bob"))).unwrap();
    s.apply(tx(
        5,
        "backup",
        104,
        RegistryAction::UpdateAdmin {
            target: AdminTarget::Whitelist(1),
            new_address: a("backup-2"),
            update_backup: true,
        },
    ))
    .unwrap();
    assert_eq!(s.whitelist(1).unwrap().backup, a("backup-2"));
}

#[test]
fn unsupported_legacy_self_cert_and_token_admin_fail_closed() {
    let mut s = boot();
    s.apply(tx(2, "owner", 101, create_list("KYC"))).unwrap();
    assert_eq!(
        s.apply(tx(3, "owner", 102, attest(0, "bob"))),
        Err(RegistryError::UnsupportedSelfCertification)
    );
    assert_eq!(
        s.apply(tx(
            3,
            "owner",
            102,
            RegistryAction::UpdateAdmin {
                target: AdminTarget::Token(2),
                new_address: a("new-admin"),
                update_backup: false,
            }
        )),
        Err(RegistryError::UnsupportedTokenAdmin)
    );
    assert_eq!(s.processed_count(), 2);
}

#[test]
fn oracle_lifecycle_uses_exact_integer_prices_and_rejects_post_close() {
    let mut s = boot();
    s.apply(tx(2, "oracle-owner", 101, create_oracle()))
        .unwrap();
    assert_eq!(s.oracle_count(), 1);
    let before = s.clone();
    assert_eq!(
        s.apply(tx(3, "mallory", 102, price(1, 1200000))),
        Err(RegistryError::Unauthorized)
    );
    assert_eq!(s, before);
    s.apply(tx(3, "oracle-owner", 102, price(1, 1200000)))
        .unwrap();
    assert_eq!(
        s.oracle(1).unwrap().latest.as_ref().unwrap().price_1e4,
        1200000
    );
    assert_eq!(
        s.oracle(1).unwrap().latest.as_ref().unwrap().low_1e4,
        Some(1199990)
    );
    s.apply(tx(
        4,
        "oracle-owner",
        103,
        RegistryAction::CloseOracle { oracle_id: 1 },
    ))
    .unwrap();
    assert!(s.oracle(1).unwrap().closed);
    assert_eq!(
        s.apply(tx(5, "oracle-owner", 104, price(1, 1300000))),
        Err(RegistryError::ClosedOracle(1))
    );
}

#[test]
fn oracle_backup_may_change_admin_and_cannot_publish_itself() {
    let mut s = boot();
    s.apply(tx(2, "owner", 101, create_oracle())).unwrap();
    assert_eq!(
        s.apply(tx(3, "backup", 102, price(1, 10000))),
        Err(RegistryError::Unauthorized)
    );
    s.apply(tx(
        3,
        "backup",
        102,
        RegistryAction::UpdateAdmin {
            target: AdminTarget::Oracle(1),
            new_address: a("new-owner"),
            update_backup: false,
        },
    ))
    .unwrap();
    s.apply(tx(4, "new-owner", 103, price(1, 10000))).unwrap();
    assert_eq!(s.oracle(1).unwrap().admin, a("new-owner"));
    assert_eq!(s.oracle(1).unwrap().latest.as_ref().unwrap().height, 103);
}

#[test]
fn malformed_metadata_and_prices_are_atomic_errors() {
    let mut s = boot();
    let before = s.clone();
    assert_eq!(
        s.apply(tx(
            2,
            "owner",
            101,
            RegistryAction::CreateWhitelist {
                name: "bad\nname".into(),
                url: String::new(),
                description: String::new(),
                backup: a("backup"),
            }
        )),
        Err(RegistryError::InvalidField("whitelist name"))
    );
    assert_eq!(s, before);
    s.apply(tx(2, "owner", 101, create_oracle())).unwrap();
    assert_eq!(
        s.apply(tx(3, "owner", 102, price(1, 0))),
        Err(RegistryError::InvalidPrice)
    );
    assert_eq!(
        s.apply(tx(
            3,
            "owner",
            102,
            RegistryAction::Attest {
                list_id: 1,
                subject: a("bob"),
                metadata: "a".repeat(MAX_METADATA_BYTES + 1),
            }
        )),
        Err(RegistryError::InvalidField("attestation metadata"))
    );
}

#[test]
fn deterministic_allocations_across_independent_runs_and_forks() {
    let mut left = boot();
    let mut right = boot();
    let ops = [
        tx(2, "owner", 101, create_list("A")),
        tx(3, "owner", 101, create_list("B")),
        tx(4, "alice", 102, create_oracle()),
        tx(5, "oracle-b", 102, create_oracle()),
    ];
    left.apply_batch(ops.clone()).unwrap();
    right.apply_batch(ops).unwrap();
    assert_eq!(left, right);
    assert_eq!(left.whitelist_count(), 2);
    assert_eq!(left.oracle_count(), 2);
    assert_eq!(left.oracle(2).unwrap().admin, a("oracle-b"));
    let fork = left.clone();
    left.apply(tx(6, "owner", 103, attest(2, "carol"))).unwrap();
    assert!(fork.attestation(2, &a("carol")).is_none());
    assert_eq!(fork.processed_count(), 5);
}

#[test]
fn unknown_registry_resources_reject_changes() {
    let mut s = boot();
    assert_eq!(
        s.apply(tx(
            2,
            "owner",
            101,
            RegistryAction::RevokeAttestation {
                list_id: 7,
                subject: a("bob")
            }
        )),
        Err(RegistryError::UnknownWhitelist(7))
    );
    assert_eq!(
        s.apply(tx(2, "owner", 101, price(1, 10000))),
        Err(RegistryError::UnknownOracle(1))
    );
    assert_eq!(
        s.apply(tx(
            2,
            "owner",
            101,
            RegistryAction::UpdateAdmin {
                target: AdminTarget::Whitelist(20),
                new_address: a("bob"),
                update_backup: false,
            }
        )),
        Err(RegistryError::UnknownWhitelist(20))
    );
}
