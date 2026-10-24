//! Stage 25D TLCL v1: complete offline clearing/spot state checkpoint tests.
//! CRC is not authentication. All identities, prices and operations unverified.
use std::collections::BTreeSet;
use tradelayer_core::clearing_math::PnlRounding;
use tradelayer_core::clearing_sandbox::{
    ClearingError, ClearingSandbox, CollateralPolicy, MarkPolicy, MarkTerms, MatchedContractTrade,
};
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::spot::{SpotExchange, SpotIntent, UnverifiedSpotPlacement};

fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn a(text: &str) -> AccountId {
    AccountId::try_from(text).unwrap()
}
fn tx(n: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn permitted() -> BTreeSet<PropertyId> {
    BTreeSet::from([p(7), p(8)])
}
fn collateral_policy() -> CollateralPolicy {
    CollateralPolicy {
        initial_atoms_per_contract: 50_000_000,
        maintenance_atoms_per_contract: 30_000_000,
    }
}
fn mark_policy() -> MarkPolicy {
    MarkPolicy {
        height: 100,
        price_e8: 100_000_000,
        notional_e8: 100_000_000,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn terms() -> MarkTerms {
    MarkTerms {
        notional_e8: mark_policy().notional_e8,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn seeded_state() -> ClearingSandbox {
    let mut ledger = Ledger::new();
    ledger
        .apply_batch([
            (tx(1), Operation::CreateProperty {
                id: p(7), issuer: a("alice"), kind: PropertyKind::Fixed,
                initial_atoms: 1_000_000_000,
            }),
            (tx(2), Operation::CreateProperty {
                id: p(8), issuer: a("bob"), kind: PropertyKind::Fixed,
                initial_atoms: 1_000_000_000,
            }),
            (tx(3), Operation::Transfer {
                id: p(7), from: a("alice"), to: a("bob"),
                amount_atoms: 350_000_000,
            }),
            (tx(4), Operation::Transfer {
                id: p(8), from: a("bob"), to: a("alice"),
                amount_atoms: 200_000_000,
            }),
        ])
        .unwrap();
    let spot = SpotExchange::from_ledger_unverified(ledger, permitted()).unwrap();
    ClearingSandbox::from_spot_unverified(TradeLayerMainnet::Litecoin, 19, p(7), spot)
        .unwrap()
        .with_collateral_policy_unverified(collateral_policy())
        .unwrap()
        .with_mark_policy_unverified(mark_policy())
        .unwrap()
}
fn order(id: u64) -> UnverifiedSpotPlacement {
    UnverifiedSpotPlacement {
        txid: tx(id),
        actor: a("alice"),
        height: 101,
        intent: SpotIntent {
            offered: p(7), desired: p(8),
            offered_atoms: 60_000_000, expected_atoms: 60_000_000,
            post: false, stop: false,
        },
    }
}
fn active() -> ClearingSandbox {
    let mut s = seeded_state();
    s.deposit_margin_unverified(tx(10), a("alice"), 200_000_000).unwrap();
    s.deposit_margin_unverified(tx(11), a("bob"), 150_000_000).unwrap();
    s.apply_matched_trades_unverified(
        tx(12),
        &[MatchedContractTrade {
            long: a("alice"), short: a("bob"), contracts: 1,
        }],
    ).unwrap();
    s.enqueue_spot_order_unverified(order(13)).unwrap();
    s.process_spot_block_unverified(101).unwrap();
    s.settle_mark_to_market_unverified(tx(14), 101, 100_000_001).unwrap();
    s.settle_funding_unverified(tx(15), 100_000_000, 100_000).unwrap();
    s
}
fn restore(bytes: &[u8]) -> Result<ClearingSandbox, tradelayer_core::clearing_sandbox::ClearingSnapshotError> {
    ClearingSandbox::from_snapshot_bytes(
        bytes, TradeLayerMainnet::Litecoin, 19, p(7),
        &permitted(), Some(collateral_policy()), Some(terms()),
    )
}
fn repair_crc(bytes: &mut [u8]) {
    let end = bytes.len() - 4;
    let mut crc = !0u32;
    for &byte in &bytes[..end] {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    bytes[end..].copy_from_slice(&(!crc).to_be_bytes());
}

#[test]
fn combined_bytes_are_stable_and_recover_all_single_ledger_obligations() {
    let original = active();
    assert!(std::ptr::eq(original.ledger(), original.spot().ledger()));
    let encoded = original.snapshot_bytes().unwrap();
    assert_eq!(&encoded[..5], b"TLCL\x01");
    let loaded = restore(&encoded).unwrap();
    assert_eq!(loaded, original);
    assert_eq!(loaded.snapshot_bytes().unwrap(), encoded);
    assert!(std::ptr::eq(loaded.ledger(), loaded.spot().ledger()));
    assert_eq!(loaded.positions()[&a("alice")], 1);
    assert_eq!(loaded.positions()[&a("bob")], -1);
    assert_eq!(loaded.spot().open_count(), 1);
    assert_eq!(loaded.spot().pending_count(), 0);
    assert_eq!(loaded.spot().processed_through(), Some(101));
    assert_eq!(loaded.mark_policy().unwrap().height, 101);
    assert_eq!(loaded.margin_atoms(&a("alice")), 199_900_001);
    assert_eq!(loaded.margin_atoms(&a("bob")), 150_099_999);
    assert_eq!(loaded.verify(), Ok(()));
    assert_eq!(loaded.ledger().verify_conservation(), Ok(()));
    assert!(loaded.spot().snapshot_bytes().is_err());
    assert!(SpotExchange::from_snapshot_bytes(&encoded, &permitted()).is_err());
}

#[test]
fn replay_and_monotonic_mark_protection_survive_round_trip() {
    let original = active();
    let snapshot = original.snapshot_bytes().unwrap();
    let mut restored = restore(&snapshot).unwrap();
    let before = restored.clone();
    assert_eq!(
        restored.settle_funding_unverified(tx(15), 100_000_000, 100_000),
        Err(ClearingError::Replay(tx(15)))
    );
    assert_eq!(
        restored.settle_mark_to_market_unverified(tx(14), 102, 100_000_002),
        Err(ClearingError::Replay(tx(14)))
    );
    assert_eq!(
        restored.enqueue_spot_order_unverified(order(13)),
        Err(ClearingError::Replay(tx(13)))
    );
    assert_eq!(restored, before);
    restored.settle_mark_to_market_unverified(tx(16), 102, 100_000_002).unwrap();
    assert_eq!(restored.mark_policy().unwrap().height, 102);
    assert_eq!(restored.verify(), Ok(()));
}

#[test]
fn zero_balance_clearing_event_history_cannot_reenter_legacy_snapshot() {
    let mut state = seeded_state();
    state.settle_mark_to_market_unverified(tx(10), 101, 100_000_001).unwrap();
    assert_eq!(state.processed_count(), 1);
    assert_eq!(state.margin_atoms(&a("alice")), 0);
    assert!(state.spot().snapshot_bytes().is_err());
    let bytes = state.snapshot_bytes().unwrap();
    let restored = restore(&bytes).unwrap();
    assert_eq!(restored.snapshot_bytes().unwrap(), bytes);
    assert!(restored.spot().snapshot_bytes().is_err());
    assert!(ClearingSandbox::from_spot_unverified(
        TradeLayerMainnet::Litecoin, 19, p(7), restored.spot().clone()
    ).is_err());
}

#[test]
fn explicit_expected_identity_and_policy_mismatch_fail_closed() {
    let bytes = active().snapshot_bytes().unwrap();
    for wrong in [TradeLayerMainnet::Bitcoin, TradeLayerMainnet::Dogecoin] {
        assert!(ClearingSandbox::from_snapshot_bytes(
            &bytes, wrong, 19, p(7), &permitted(),
            Some(collateral_policy()), Some(terms())
        ).is_err());
    }
    assert!(ClearingSandbox::from_snapshot_bytes(
        &bytes, TradeLayerMainnet::Litecoin, 20, p(7), &permitted(),
        Some(collateral_policy()), Some(terms())
    ).is_err());
    assert!(ClearingSandbox::from_snapshot_bytes(
        &bytes, TradeLayerMainnet::Litecoin, 19, p(8), &permitted(),
        Some(collateral_policy()), Some(terms())
    ).is_err());
    assert!(ClearingSandbox::from_snapshot_bytes(
        &bytes, TradeLayerMainnet::Litecoin, 19, p(7), &BTreeSet::from([p(7)]),
        Some(collateral_policy()), Some(terms())
    ).is_err());
    assert!(ClearingSandbox::from_snapshot_bytes(
        &bytes, TradeLayerMainnet::Litecoin, 19, p(7), &permitted(),
        None, Some(terms())
    ).is_err());
    let terms_bad = MarkTerms { inverse: true, ..terms() };
    assert!(ClearingSandbox::from_snapshot_bytes(
        &bytes, TradeLayerMainnet::Litecoin, 19, p(7), &permitted(),
        Some(collateral_policy()), Some(terms_bad)
    ).is_err());
}

#[test]
fn corrupt_version_truncation_oversize_and_trailing_bytes_are_rejected() {
    let bytes = active().snapshot_bytes().unwrap();
    for len in 0..bytes.len().min(360) {
        assert!(restore(&bytes[..len]).is_err(), "truncated len {len}");
    }
    assert!(restore(&bytes[..bytes.len() - 1]).is_err());
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(restore(&extra).is_err());
    let mut tampered = bytes.clone();
    tampered[0] ^= 1;
    assert!(restore(&tampered).is_err());
    let mut revised_crc = bytes.clone();
    revised_crc[4] = 2;
    repair_crc(&mut revised_crc);
    assert!(restore(&revised_crc).is_err());
    let mut wrong_genesis = bytes.clone();
    wrong_genesis[6] ^= 1;
    repair_crc(&mut wrong_genesis);
    assert!(restore(&wrong_genesis).is_err());
    let mut illegal_flag = bytes.clone();
    // 5-byte magic + 1-byte network + 32-byte genesis + 32-byte chain
    // + 4-byte contract + 4-byte collateral = offset 78, policy flag.
    illegal_flag[78] = 3;
    repair_crc(&mut illegal_flag);
    assert!(restore(&illegal_flag).is_err());
    let excessive = vec![0; 64 * 1024 * 1024 + 1];
    assert!(restore(&excessive).is_err());
}

#[test]
fn TLCL_without_clearing_events_still_restores_original_spot_only_state() {
    let s = seeded_state();
    let bytes = s.snapshot_bytes().unwrap();
    let recovered = restore(&bytes).unwrap();
    assert_eq!(s, recovered);
    assert!(recovered.spot().snapshot_bytes().is_ok());
    assert_eq!(recovered.snapshot_bytes().unwrap(), bytes);
}

#[test]
fn exposed_state_is_read_only_and_does_not_make_a_checkpoint_an_authorization() {
    let s = active();
    let bytes = s.snapshot_bytes().unwrap();
    let loaded = restore(&bytes).unwrap();
    assert_eq!(loaded.ledger().verify_conservation(), Ok(()));
    assert_eq!(loaded.margin_atoms(&a("alice")) + loaded.margin_atoms(&a("bob")),
        s.margin_atoms(&a("alice")) + s.margin_atoms(&a("bob")));
    assert!(loaded.spot().snapshot_bytes().is_err());
    // No method here turns a decoded checkpoint into an authenticated
    // chain transaction, a proof of trade matching or insurance coverage.
}
