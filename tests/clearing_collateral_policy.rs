//! Synthetic, opt-in collateral floors. Economic ratio is caller supplied.
use std::collections::BTreeSet;
use tradelayer_core::clearing_sandbox::{
    ClearingError, ClearingSandbox, CollateralPolicy, MatchedContractTrade,
};
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::spot::{SpotExchange, SpotIntent, UnverifiedSpotPlacement};

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
fn joined() -> ClearingSandbox {
    let mut ledger = Ledger::new();
    ledger
        .apply_batch([
            (
                tx(1),
                Operation::CreateProperty {
                    id: p(7),
                    issuer: a("alice"),
                    kind: PropertyKind::Fixed,
                    initial_atoms: 1_000_000_000,
                },
            ),
            (
                tx(2),
                Operation::CreateProperty {
                    id: p(8),
                    issuer: a("bob"),
                    kind: PropertyKind::Fixed,
                    initial_atoms: 1_000_000_000,
                },
            ),
            (
                tx(3),
                Operation::Transfer {
                    id: p(7),
                    from: a("alice"),
                    to: a("bob"),
                    amount_atoms: 200_000_000,
                },
            ),
        ])
        .unwrap();
    let spot = SpotExchange::from_ledger_unverified(ledger, BTreeSet::from([p(7), p(8)])).unwrap();
    ClearingSandbox::from_spot_unverified(TradeLayerMainnet::Litecoin, 19, p(7), spot).unwrap()
}
fn one_trade(long: &str, short: &str) -> Vec<MatchedContractTrade> {
    vec![MatchedContractTrade {
        long: a(long),
        short: a(short),
        contracts: 1,
    }]
}

#[test]
fn invalid_or_reconfigurable_collateral_policy_is_denied() {
    let unconfigured = joined();
    assert_eq!(
        unconfigured
            .clone()
            .with_collateral_policy_unverified(CollateralPolicy {
                initial_atoms_per_contract: 100,
                maintenance_atoms_per_contract: 101,
            }),
        Err(ClearingError::InvalidCollateralPolicy)
    );
    assert_eq!(
        unconfigured
            .clone()
            .with_collateral_policy_unverified(CollateralPolicy {
                initial_atoms_per_contract: 0,
                maintenance_atoms_per_contract: 0,
            }),
        Err(ClearingError::InvalidCollateralPolicy)
    );
    let policy = CollateralPolicy {
        initial_atoms_per_contract: 80_000_000,
        maintenance_atoms_per_contract: 50_000_000,
    };
    let mut state = unconfigured
        .with_collateral_policy_unverified(policy)
        .unwrap();
    assert_eq!(state.collateral_policy(), Some(policy));
    state
        .deposit_margin_unverified(tx(10), a("alice"), 100_000_000)
        .unwrap();
    let before = state.clone();
    assert_eq!(
        state.clone().with_collateral_policy_unverified(policy),
        Err(ClearingError::CollateralPolicyFrozen)
    );
    assert_eq!(state, before);
}

#[test]
fn initial_margin_rejects_increased_exposure_and_keeps_original_ledger() {
    let mut state = joined()
        .with_collateral_policy_unverified(CollateralPolicy {
            initial_atoms_per_contract: 80_000_000,
            maintenance_atoms_per_contract: 50_000_000,
        })
        .unwrap();
    state
        .deposit_margin_unverified(tx(10), a("alice"), 100_000_000)
        .unwrap();
    state
        .deposit_margin_unverified(tx(11), a("bob"), 100_000_000)
        .unwrap();
    state
        .apply_matched_trades_unverified(tx(12), &one_trade("alice", "bob"))
        .unwrap();
    let snapshot = state.clone();
    assert!(matches!(
        state.apply_matched_trades_unverified(tx(13), &one_trade("alice", "bob")),
        Err(ClearingError::InsufficientMargin { .. })
    ));
    assert_eq!(state, snapshot);
    state
        .apply_matched_trades_unverified(tx(13), &one_trade("bob", "alice"))
        .unwrap();
    assert!(state.positions().is_empty());
    assert_eq!(state.verify(), Ok(()));
}

#[test]
fn insufficient_maintenance_after_funding_rolls_back_and_retry_is_permitted() {
    let mut state = joined()
        .with_collateral_policy_unverified(CollateralPolicy {
            initial_atoms_per_contract: 60_000_000,
            maintenance_atoms_per_contract: 60_000_000,
        })
        .unwrap();
    state
        .deposit_margin_unverified(tx(10), a("alice"), 60_000_001)
        .unwrap();
    state
        .deposit_margin_unverified(tx(11), a("bob"), 60_000_001)
        .unwrap();
    state
        .apply_matched_trades_unverified(tx(12), &one_trade("alice", "bob"))
        .unwrap();
    let before = state.clone();
    assert_eq!(
        state.settle_funding_unverified(tx(13), 100_000_000, 100_000),
        Err(ClearingError::InsufficientMargin {
            account: a("alice"),
            required_atoms: 60_000_000,
            available_atoms: 59_900_001,
        })
    );
    assert_eq!(state, before);
    state
        .deposit_margin_unverified(tx(14), a("alice"), 200_000)
        .unwrap();
    state
        .settle_funding_unverified(tx(13), 100_000_000, 100_000)
        .unwrap();
    assert_eq!(state.margin_atoms(&a("alice")), 60_100_001);
    assert_eq!(state.margin_atoms(&a("bob")), 60_100_001);
    assert_eq!(state.verify(), Ok(()));
}

#[test]
fn posted_spot_reserves_are_not_eligible_collateral_for_opening_contracts() {
    let mut state = joined()
        .with_collateral_policy_unverified(CollateralPolicy {
            initial_atoms_per_contract: 100_000_000,
            maintenance_atoms_per_contract: 80_000_000,
        })
        .unwrap();
    state
        .deposit_margin_unverified(tx(10), a("alice"), 90_000_000)
        .unwrap();
    state
        .deposit_margin_unverified(tx(11), a("bob"), 110_000_000)
        .unwrap();
    state
        .enqueue_spot_order_unverified(UnverifiedSpotPlacement {
            txid: tx(12),
            actor: a("alice"),
            height: 10,
            intent: SpotIntent {
                offered: p(7),
                desired: p(8),
                offered_atoms: 150_000_000,
                expected_atoms: 150_000_000,
                post: false,
                stop: false,
            },
        })
        .unwrap();
    state.process_spot_block_unverified(10).unwrap();
    let before = state.clone();
    assert_eq!(state.spot().open_count(), 1);
    assert_eq!(state.margin_atoms(&a("alice")), 90_000_000);
    assert!(matches!(
        state.apply_matched_trades_unverified(tx(13), &one_trade("alice", "bob")),
        Err(ClearingError::InsufficientMargin { .. })
    ));
    assert_eq!(state, before);
    state
        .deposit_margin_unverified(tx(14), a("alice"), 20_000_000)
        .unwrap();
    state
        .apply_matched_trades_unverified(tx(13), &one_trade("alice", "bob"))
        .unwrap();
    assert_eq!(state.margin_atoms(&a("alice")), 110_000_000);
    assert_eq!(state.spot().open_count(), 1);
    assert_eq!(state.ledger().verify_conservation(), Ok(()));
}
