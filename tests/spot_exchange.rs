//! Stage 19 offline token-to-token matching research. Source authority is
//! legacy TradeLayer.js logic.js/orderbook.js/validity.js; intentional
//! economic-semantic differences are recorded in docs/SPOT_EXCHANGE.md.

use std::collections::BTreeSet;
use tradelayer_core::ledger::{
    AccountId, Ledger, LedgerError, Operation, PropertyId, PropertyKind, TxId,
};
use tradelayer_core::spot::{
    CancelSelector, SpotError, SpotExchange, SpotIntent, SpotPair, UnverifiedSpotCancellation,
    UnverifiedSpotPlacement,
};

fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn a(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn tx(n: u8) -> TxId {
    TxId::from_bytes([n; 32])
}

fn ledger() -> Ledger {
    let mut ledger = Ledger::new();
    ledger
        .apply(
            tx(1),
            Operation::CreateProperty {
                id: p(7),
                issuer: a("seller"),
                kind: PropertyKind::Fixed,
                initial_atoms: 1000,
            },
        )
        .unwrap();
    ledger
        .apply(
            tx(2),
            Operation::CreateProperty {
                id: p(8),
                issuer: a("buyer"),
                kind: PropertyKind::Managed,
                initial_atoms: 1000,
            },
        )
        .unwrap();
    ledger
}
fn exchange() -> SpotExchange {
    SpotExchange::from_ledger_unverified(ledger(), BTreeSet::from([p(7), p(8)])).unwrap()
}
fn place(id: u8, actor: &str, height: u64, offered: u32, desired: u32, amount: u64, wanted: u64) -> UnverifiedSpotPlacement {
    UnverifiedSpotPlacement {
        txid: tx(id),
        actor: a(actor),
        height,
        intent: SpotIntent {
            offered: p(offered),
            desired: p(desired),
            offered_atoms: amount,
            expected_atoms: wanted,
            post: false,
            stop: false,
        },
    }
}
fn cancel(id: u8, actor: &str, height: u64, offered: u32, desired: u32, selector: CancelSelector) -> UnverifiedSpotCancellation {
    UnverifiedSpotCancellation {
        txid: tx(id),
        actor: a(actor),
        height,
        offered: p(offered),
        desired: p(desired),
        is_contract: false,
        selector,
    }
}

#[test]
fn type5_maker_sell_partial_price_improvement_returns_unspent_quote() {
    let mut state = exchange();
    state
        .enqueue_type5_unverified(place(10, "seller", 10, 7, 8, 100, 200))
        .unwrap();
    assert_eq!(state.pending_count(), 1);
    assert_eq!(state.ledger().available_atoms(p(7), &a("seller")), 1000);
    assert_eq!(state.process_through_unverified(10).unwrap(), 1);
    assert_eq!(state.ledger().available_atoms(p(7), &a("seller")), 900);
    assert_eq!(state.open_count(), 1);
    assert!(state.fills().is_empty());
    state
        .enqueue_type5_unverified(place(11, "buyer", 11, 8, 7, 220, 100))
        .unwrap();
    assert_eq!(state.process_through_unverified(11).unwrap(), 1);
    assert_eq!(state.open_count(), 0);
    assert_eq!(state.fills().len(), 1);
    assert_eq!(state.fills()[0].base_atoms, 100);
    assert_eq!(state.fills()[0].quote_atoms, 200);
    assert_eq!(state.ledger().available_atoms(p(7), &a("buyer")), 100);
    assert_eq!(state.ledger().available_atoms(p(8), &a("seller")), 200);
    assert_eq!(state.ledger().available_atoms(p(8), &a("buyer")), 800);
    assert_eq!(state.ledger().available_atoms(p(7), &a("seller")), 900);
    state.ledger().verify_conservation().unwrap();
    state.verify_reserves().unwrap();
}

#[test]
fn type5_maker_buy_and_same_block_buy_prices_are_exact() {
    let mut state = exchange();
    state
        .enqueue_type5_unverified(place(10, "buyer", 10, 8, 7, 220, 100))
        .unwrap();
    state.process_through_unverified(10).unwrap();
    state
        .enqueue_type5_unverified(place(11, "seller", 11, 7, 8, 100, 200))
        .unwrap();
    state.process_through_unverified(11).unwrap();
    assert_eq!(state.fills()[0].quote_atoms, 220);
    assert_eq!(state.ledger().available_atoms(p(8), &a("seller")), 220);
    assert_eq!(state.ledger().available_atoms(p(8), &a("buyer")), 780);

    let mut left = exchange();
    let mut right = exchange();
    let sell = place(14, "seller", 10, 7, 8, 100, 200);
    let buy = place(15, "buyer", 10, 8, 7, 220, 100);
    for row in [sell.clone(), buy.clone()] {
        left.enqueue_type5_unverified(row).unwrap();
    }
    for row in [buy, sell] {
        right.enqueue_type5_unverified(row).unwrap();
    }
    left.process_through_unverified(10).unwrap();
    right.process_through_unverified(10).unwrap();
    assert_eq!(left, right);
    assert_eq!(left.fills()[0].quote_atoms, 220);
    assert_eq!(left.fills()[0].base_atoms, 100);
}

#[test]
fn type6_refunds_remaining_reserve_after_partial_fill() {
    let mut state = exchange();
    state
        .enqueue_type5_unverified(place(10, "seller", 10, 7, 8, 100, 200))
        .unwrap();
    state.process_through_unverified(10).unwrap();
    state
        .enqueue_type5_unverified(place(11, "buyer", 11, 8, 7, 110, 50))
        .unwrap();
    state.process_through_unverified(11).unwrap();
    assert_eq!(state.fills().len(), 1);
    assert_eq!(state.fills()[0].base_atoms, 50);
    assert_eq!(state.fills()[0].quote_atoms, 100);
    let pair = SpotPair::new(p(7), p(8)).unwrap();
    assert_eq!(state.open_orders(pair).len(), 1);
    assert_eq!(state.open_orders(pair)[0].remaining_offered, 50);
    assert_eq!(state.ledger().available_atoms(p(7), &a("seller")), 900);
    assert_eq!(state.ledger().available_atoms(p(8), &a("buyer")), 900);
    assert_eq!(
        state
            .cancel_type6_unverified(cancel(12, "seller", 12, 7, 8, CancelSelector::All))
            .unwrap(),
        1
    );
    assert_eq!(state.open_count(), 0);
    assert_eq!(state.ledger().available_atoms(p(7), &a("seller")), 950);
    assert_eq!(state.ledger().available_atoms(p(7), &a("buyer")), 50);
    assert_eq!(state.ledger().available_atoms(p(8), &a("seller")), 100);
    state.ledger().verify_conservation().unwrap();
}

#[test]
fn type6_full_txid_cancel_is_owner_scoped_and_does_not_truncate_ids() {
    let mut state = exchange();
    state.enqueue_type5_unverified(place(10, "seller", 10, 7, 8, 100, 200)).unwrap();
    state.enqueue_type5_unverified(place(11, "seller", 10, 7, 8, 150, 300)).unwrap();
    state.process_through_unverified(10).unwrap();
    let pair = SpotPair::new(p(7), p(8)).unwrap();
    assert_eq!(state.open_orders(pair).len(), 2);
    let saved = state.clone();
    assert_eq!(
        state.cancel_type6_unverified(cancel(12, "buyer", 11, 7, 8, CancelSelector::ByOrderTxId(tx(10)))),
        Err(SpotError::UnauthorizedCancellation)
    );
    assert_eq!(state, saved);
    assert_eq!(
        state.cancel_type6_unverified(cancel(13, "seller", 11, 7, 8, CancelSelector::ByOrderTxId(tx(10)))).unwrap(),
        1
    );
    assert_eq!(state.open_orders(pair).len(), 1);
    assert_eq!(state.open_orders(pair)[0].txid, tx(11));
    assert_eq!(state.ledger().available_atoms(p(7), &a("seller")), 850);
    assert!(matches!(
        state.cancel_type6_unverified(cancel(13, "seller", 11, 7, 8, CancelSelector::All)),
        Err(SpotError::Replay(_))
    ));
    assert_eq!(
        state.cancel_type6_unverified(cancel(14, "seller", 11, 7, 8, CancelSelector::All)).unwrap(),
        1
    );
    assert_eq!(state.ledger().available_atoms(p(7), &a("seller")), 1000);
}

#[test]
fn block_processing_is_atomic_if_second_order_cannot_be_reserved() {
    let mut state = exchange();
    state.enqueue_type5_unverified(place(10, "seller", 10, 7, 8, 800, 800)).unwrap();
    state.enqueue_type5_unverified(place(11, "seller", 10, 7, 8, 300, 300)).unwrap();
    let before = state.clone();
    assert!(matches!(
        state.process_through_unverified(10),
        Err(SpotError::Ledger(LedgerError::InsufficientAvailable { .. }))
    ));
    assert_eq!(state, before);
    assert_eq!(state.pending_count(), 2);
    assert_eq!(state.open_count(), 0);
    assert_eq!(state.ledger().available_atoms(p(7), &a("seller")), 1000);
}

#[test]
fn spoofed_cancel_replays_unsupported_orders_and_policy_are_rejected() {
    let mut state = exchange();
    assert_eq!(state.type3_utxo_trade_unverified(), Err(SpotError::AuthenticatedUtxoProofRequired));
    let mut rejected = place(10, "seller", 10, 7, 8, 100, 200);
    rejected.intent.stop = true;
    assert_eq!(state.enqueue_type5_unverified(rejected), Err(SpotError::UnsupportedFlag));
    let mut rejected = place(10, "seller", 10, 7, 8, 100, 200);
    rejected.intent.post = true;
    assert_eq!(state.enqueue_type5_unverified(rejected), Err(SpotError::UnsupportedFlag));
    assert_eq!(
        state.enqueue_type5_unverified(place(10, "seller", 10, 7, 7, 100, 200)),
        Err(SpotError::SameAsset)
    );
    assert_eq!(
        state.enqueue_type5_unverified(place(10, "seller", 10, 7, 8, 0, 200)),
        Err(SpotError::ZeroAmount)
    );
    assert_eq!(
        state.enqueue_type5_unverified(place(10, "~spot:evil", 10, 7, 8, 100, 200)),
        Err(SpotError::ReservedAccount)
    );
    let mut forbidden = place(10, "seller", 10, 7, 8, 100, 200);
    forbidden.intent.desired = p(3);
    assert_eq!(state.enqueue_type5_unverified(forbidden), Err(SpotError::RestrictedProperty));
    assert_eq!(
        SpotExchange::from_ledger_unverified(ledger(), BTreeSet::from([p(3), p(7)])),
        Err(SpotError::RestrictedProperty)
    );
    state.enqueue_type5_unverified(place(10, "seller", 10, 7, 8, 100, 200)).unwrap();
    assert_eq!(
        state.enqueue_type5_unverified(place(10, "seller", 10, 7, 8, 100, 200)),
        Err(SpotError::Replay(tx(10)))
    );
    state.process_through_unverified(10).unwrap();
    assert_eq!(
        state.cancel_type6_unverified(cancel(12, "buyer", 11, 7, 8, CancelSelector::All)),
        Err(SpotError::NoMatchingOrder)
    );
    assert_eq!(
        state.cancel_type6_unverified(cancel(12, "seller", 10, 7, 8, CancelSelector::All)),
        Err(SpotError::HeightRegression)
    );
    assert_eq!(
        state.enqueue_type5_unverified(place(11, "seller", 10, 7, 8, 100, 200)),
        Err(SpotError::HeightRegression)
    );
    let mut contract = cancel(12, "seller", 11, 7, 8, CancelSelector::All);
    contract.is_contract = true;
    assert_eq!(state.cancel_type6_unverified(contract), Err(SpotError::UnsupportedContractCancel));
    assert_eq!(state.open_count(), 1);
    state.verify_reserves().unwrap();
}

#[test]
fn self_cross_keeps_both_escrows_and_price_dust_rejects_atomically() {
    let mut l = ledger();
    l.apply(tx(3), Operation::Transfer { id: p(8), from: a("buyer"), to: a("seller"), amount_atoms: 100 }).unwrap();
    let mut state=SpotExchange::from_ledger_unverified(l, BTreeSet::from([p(7),p(8)])).unwrap();
    state.enqueue_type5_unverified(place(10, "seller", 10, 7, 8, 10, 20)).unwrap();
    state.enqueue_type5_unverified(place(11, "seller", 10, 8, 7, 20, 10)).unwrap();
    state.process_through_unverified(10).unwrap();
    assert_eq!(state.open_count(),2);
    assert!(state.fills().is_empty());
    state.verify_reserves().unwrap();

    let mut dust = exchange();
    dust.enqueue_type5_unverified(place(20, "seller", 10, 7, 8, 3, 1)).unwrap();
    dust.process_through_unverified(10).unwrap();
    dust.enqueue_type5_unverified(place(21, "buyer", 11, 8, 7, 1, 2)).unwrap();
    dust.process_through_unverified(11).unwrap();
    assert_eq!(dust.fills().len(),1);
    assert_eq!(dust.fills()[0].base_atoms,2);
    assert_eq!(dust.fills()[0].quote_atoms,1);
    dust.enqueue_type5_unverified(place(22, "buyer", 12, 8, 7, 1, 2)).unwrap();
    let before = dust.clone();
    assert_eq!(dust.process_through_unverified(12), Err(SpotError::UnrepresentableFill));
    assert_eq!(dust, before);
}

#[test]
fn no_supply_creation_and_two_asset_sums_hold_after_many_orders() {
    let mut state=exchange();
    for (id,amount) in [(10,30),(11,40),(12,50)] {
        state.enqueue_type5_unverified(place(id, "seller", 10, 7, 8, amount, amount*2)).unwrap();
    }
    state.process_through_unverified(10).unwrap();
    state.enqueue_type5_unverified(place(20, "buyer", 11, 8, 7, 300, 120)).unwrap();
    state.process_through_unverified(11).unwrap();
    assert_eq!(state.fills().len(),3);
    assert_eq!(state.open_count(),0);
    assert_eq!(state.ledger().available_atoms(p(7), &a("seller")),880);
    assert_eq!(state.ledger().available_atoms(p(7), &a("buyer")),120);
    assert_eq!(state.ledger().available_atoms(p(8), &a("seller")),240);
    assert_eq!(state.ledger().available_atoms(p(8), &a("buyer")),760);
    assert_eq!(state.ledger().property(p(7)).unwrap().total_supply_atoms,1000);
    assert_eq!(state.ledger().property(p(8)).unwrap().total_supply_atoms,1000);
    state.ledger().verify_conservation().unwrap();
}
