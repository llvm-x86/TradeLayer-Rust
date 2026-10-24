//! Stage 25B: offline co-ownership of spot reserves and clearing margin.
//! One SpotExchange owns the sole Ledger. No chain authentication or settlement.
use std::collections::BTreeSet;
use tradelayer_core::clearing_sandbox::{
    ClearingError, ClearingSandbox, MatchedContractTrade,
};
use tradelayer_core::ledger::{
    AccountId, Ledger, LedgerError, Operation, PropertyId, PropertyKind, TxId,
};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::spot::{
    SpotError, SpotExchange, SpotIntent, UnverifiedSpotPlacement,
};

fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn a(name: &str) -> AccountId {
    AccountId::try_from(name).unwrap()
}
fn tx(n: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn funded() -> Ledger {
    let mut ledger = Ledger::new();
    ledger.apply_batch([
        (tx(1), Operation::CreateProperty {
            id: p(7), issuer: a("alice"), kind: PropertyKind::Fixed, initial_atoms: 1_000_000_000,
        }),
        (tx(2), Operation::CreateProperty {
            id: p(8), issuer: a("bob"), kind: PropertyKind::Fixed, initial_atoms: 1_000_000_000,
        }),
        (tx(3), Operation::Transfer {
            id: p(7), from: a("alice"), to: a("bob"), amount_atoms: 200_000_000,
        }),
        (tx(4), Operation::Transfer {
            id: p(8), from: a("bob"), to: a("alice"), amount_atoms: 100_000_000,
        }),
    ]).unwrap();
    ledger
}
fn joined() -> ClearingSandbox {
    let exchange = SpotExchange::from_ledger_unverified(
        funded(),
        BTreeSet::from([p(7),p(8)]),
    ).unwrap();
    ClearingSandbox::from_spot_unverified(
        TradeLayerMainnet::Litecoin,19,p(7),exchange,
    ).unwrap()
}
fn order(n:u64, actor:&str, height:u64, offer:u32, want:u32, atoms:u64) -> UnverifiedSpotPlacement {
    UnverifiedSpotPlacement {
        txid:tx(n), actor:a(actor),height,
        intent:SpotIntent {
            offered:p(offer),desired:p(want),
            offered_atoms:atoms,expected_atoms:atoms,post:false,stop:false,
        },
    }
}

#[test]
fn mixed_issue_send_spot_margin_trade_funding_share_one_ledger() {
    let mut state = joined();
    assert!(std::ptr::eq(state.ledger(),state.spot().ledger()));
    state.deposit_margin_unverified(tx(10),a("alice"),150_000_000).unwrap();
    state.deposit_margin_unverified(tx(11),a("bob"),100_000_000).unwrap();
    state.apply_matched_trades_unverified(
        tx(12), &[MatchedContractTrade {long:a("alice"),short:a("bob"),contracts:1}],
    ).unwrap();
    state.enqueue_spot_order_unverified(order(13,"alice",10,7,8,200_000_000)).unwrap();
    state.enqueue_spot_order_unverified(order(14,"bob",10,8,7,200_000_000)).unwrap();
    assert_eq!(state.process_spot_block_unverified(10).unwrap(),2);
    assert_eq!(state.spot().fills().len(),1);
    assert_eq!(state.spot().open_count(),0);
    assert_eq!(state.ledger().available_atoms(p(7),&a("alice")),450_000_000);
    assert_eq!(state.ledger().available_atoms(p(7),&a("bob")),300_000_000);
    assert_eq!(state.ledger().available_atoms(p(8),&a("alice")),300_000_000);
    assert_eq!(state.ledger().available_atoms(p(8),&a("bob")),700_000_000);
    let receipt=state.settle_funding_unverified(tx(15),100_000_000,100_000).unwrap();
    assert_eq!(receipt.nominal_atoms,100_000);
    assert_eq!(state.margin_atoms(&a("alice")),149_900_000);
    assert_eq!(state.margin_atoms(&a("bob")),100_100_000);
    assert_eq!(state.ledger().property(p(7)).unwrap().total_supply_atoms,1_000_000_000);
    assert_eq!(state.ledger().property(p(8)).unwrap().total_supply_atoms,1_000_000_000);
    assert_eq!(state.verify(),Ok(()));
    assert!(std::ptr::eq(state.ledger(),state.spot().ledger()));
}

#[test]
fn unmatched_spot_order_and_open_margin_cannot_spend_one_another_reserves() {
    let mut state=joined();
    state.deposit_margin_unverified(tx(10),a("alice"),150_000_000).unwrap();
    state.deposit_margin_unverified(tx(11),a("bob"),100_000_000).unwrap();
    state.enqueue_spot_order_unverified(order(12,"alice",10,7,8,300_000_000)).unwrap();
    state.process_spot_block_unverified(10).unwrap();
    assert_eq!(state.spot().open_count(),1);
    let before=state.clone();
    let escrow=state.margin_escrow_for_audit(&a("alice")).unwrap().clone();
    assert_eq!(
        state.apply_ordinary_operation_unverified(
            tx(15), Operation::Transfer {
                id:p(7),from:escrow,to:a("bob"),amount_atoms:1,
            }
        ),
        Err(ClearingError::Spot(SpotError::ReservedAccount))
    );
    assert_eq!(state,before);
    assert_eq!(
        state.enqueue_spot_order_unverified(order(16,escrow.as_str(),11,7,8,1)),
        Err(ClearingError::Spot(SpotError::ReservedAccount))
    );
    assert_eq!(state,before);
    // Without the margin metadata, the historical TLSP snapshot cannot
    // recover this joint state, so it MUST reject rather than orphan escrow.
    assert!(state.spot().snapshot_bytes().is_err());
    assert_eq!(state.verify(),Ok(()));
    let total=state.ledger().available_atoms(p(7),&a("alice"))
        + state.margin_atoms(&a("alice"));
    assert_eq!(total,500_000_000);
}

#[test]
fn late_spot_reserve_failure_restores_margin_and_pending_orders() {
    let mut state=joined();
    state.deposit_margin_unverified(tx(10),a("alice"),750_000_000).unwrap();
    let after_deposit=state.clone();
    state.enqueue_spot_order_unverified(order(11,"alice",10,7,8,100_000_000)).unwrap();
    let queued=state.clone();
    assert!(matches!(
        state.process_spot_block_unverified(10),
        Err(ClearingError::Spot(SpotError::Ledger(LedgerError::InsufficientAvailable{..})))
    ));
    assert_eq!(state,queued);
    assert_eq!(state.margin_atoms(&a("alice")),750_000_000);
    assert_eq!(state.ledger().available_atoms(p(7),&a("alice")),50_000_000);
    assert_eq!(state.spot().pending_count(),1);
    assert_eq!(after_deposit.spot().pending_count(),0);
}

#[test]
fn clearing_and_spot_replay_ids_are_disjoint() {
    let mut state=joined();
    state.deposit_margin_unverified(tx(10),a("alice"),100_000_000).unwrap();
    assert_eq!(
        state.enqueue_spot_order_unverified(order(10,"alice",10,7,8,1)),
        Err(ClearingError::Replay(tx(10)))
    );
    state.enqueue_spot_order_unverified(order(11,"alice",10,7,8,1)).unwrap();
    let before=state.clone();
    assert_eq!(
        state.deposit_margin_unverified(tx(11),a("bob"),100),
        Err(ClearingError::Replay(tx(11)))
    );
    assert_eq!(state,before);
}

#[test]
fn ordinary_token_operation_cannot_eat_margin_or_spot_escrow() {
    let mut state=joined();
    state.deposit_margin_unverified(tx(10),a("alice"),100_000_000).unwrap();
    state.enqueue_spot_order_unverified(order(11,"alice",10,7,8,200_000_000)).unwrap();
    state.process_spot_block_unverified(10).unwrap();
    let margin_escrow=state.margin_escrow_for_audit(&a("alice")).unwrap().clone();
    let snapshot=state.clone();
    for from in [margin_escrow,a("~spot:invalid")] {
        assert_eq!(
            state.apply_ordinary_operation_unverified(
                tx(12), Operation::Transfer {
                    id:p(7),from,to:a("mallory"),amount_atoms:1
                },
            ),
            Err(ClearingError::Spot(SpotError::ReservedAccount))
        );
        assert_eq!(state,snapshot);
    }
    state.apply_ordinary_operation_unverified(
        tx(12), Operation::Transfer {
            id:p(8),from:a("bob"),to:a("alice"),amount_atoms:10,
        },
    ).unwrap();
    assert_eq!(state.ledger().available_atoms(p(8),&a("alice")),100_000_010);
    assert_eq!(state.ledger().verify_conservation(),Ok(()));
}
