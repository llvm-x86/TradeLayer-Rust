//! Stage 25B: no TLSP/TLJP checkpoint or legacy joint migration may
//! discard an active OR historically completed clearing transaction.
use std::collections::BTreeSet;
use tradelayer_core::clearing_sandbox::{ClearingError, ClearingSandbox};
use tradelayer_core::joint::{JointError, JointProtocolState};
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::registry::Registry;
use tradelayer_core::spot::{SpotError, SpotExchange};

fn id(n: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn a(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn property() -> PropertyId {
    PropertyId::try_from(7).unwrap()
}
fn sandbox() -> ClearingSandbox {
    let mut ledger = Ledger::new();
    ledger
        .apply(
            id(1),
            Operation::CreateProperty {
                id: property(),
                issuer: a("alice"),
                kind: PropertyKind::Fixed,
                initial_atoms: 1_000_000,
            },
        )
        .unwrap();
    let spot = SpotExchange::from_ledger_unverified(ledger, BTreeSet::from([property()])).unwrap();
    ClearingSandbox::from_spot_unverified(TradeLayerMainnet::Litecoin, 19, property(), spot)
        .unwrap()
}
fn assert_joint_migration_denied(book: &ClearingSandbox) {
    assert_eq!(
        JointProtocolState::migrate_spot_components_unverified(
            TradeLayerMainnet::Litecoin,
            Registry::new(a("admin")),
            book.spot().clone(),
            10,
        ),
        Err(JointError::Spot(SpotError::ReservedAccount))
    );
}

#[test]
fn active_margin_cannot_be_serialized_as_spot_or_migrated_to_joint() {
    let mut book = sandbox();
    assert!(book.spot().snapshot_bytes().is_ok());
    book.deposit_margin_unverified(id(2), a("alice"), 1000)
        .unwrap();
    assert_eq!(book.margin_atoms(&a("alice")), 1000);
    assert!(book.spot().snapshot_bytes().is_err());
    book.process_spot_block_unverified(10).unwrap();
    assert_joint_migration_denied(&book);
    assert_eq!(
        ClearingSandbox::from_spot_unverified(
            TradeLayerMainnet::Litecoin,
            19,
            property(),
            book.spot().clone(),
        ),
        Err(ClearingError::EscrowCollision)
    );
}

#[test]
fn completely_withdrawn_margin_still_carries_unserializable_replay_history() {
    let mut book = sandbox();
    book.deposit_margin_unverified(id(2), a("alice"), 1000)
        .unwrap();
    book.withdraw_margin_unverified(id(3), a("alice"), 1000)
        .unwrap();
    assert_eq!(book.margin_atoms(&a("alice")), 0);
    assert!(book.margin_escrow_for_audit(&a("alice")).is_none());
    assert_eq!(
        book.ledger().available_atoms(property(), &a("alice")),
        1_000_000
    );
    assert_eq!(book.verify(), Ok(()));
    book.process_spot_block_unverified(10).unwrap();
    // The event history is not encoded in TLSP, even though all escrow
    // balances were refunded. The fence must survive this zero-balance case.
    assert!(book.spot().snapshot_bytes().is_err());
    assert_joint_migration_denied(&book);
    assert_eq!(
        ClearingSandbox::from_spot_unverified(
            TradeLayerMainnet::Litecoin,
            19,
            property(),
            book.spot().clone(),
        ),
        Err(ClearingError::EscrowCollision)
    );
}
