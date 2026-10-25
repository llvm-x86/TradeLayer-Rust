//! Activation.js parity: offline 36-type source registry projection.
//! These tests DO NOT grant chain permission or initialize native balances.

use tradelayer_core::legacy_activation_audit::{
    legacy_admin_address, legacy_is_sender_genesis_admin, ClaimedActivationPosition,
    ClaimedLegacyActivation, LegacyActivationError, UnverifiedLegacyActivationRegistry,
    LEGACY_ACTIVATION_NAMES, LEGACY_ACTIVATION_TYPE_COUNT, MAX_ACTIVATION_BATCH,
    PINNED_GENESIS_ADMIN_LITECOIN_TESTNET,
};
use tradelayer_core::network_profile::TradeLayerMainnet;

fn at(height: u64, tx_index: u32, block_byte: u8) -> ClaimedActivationPosition {
    ClaimedActivationPosition {
        height,
        tx_index,
        block_id: [block_byte; 32],
    }
}

fn e(
    net: TradeLayerMainnet,
    ty: u8,
    height: u64,
    tx_index: u32,
    claimed_sender: &'static str,
    code_hash: Option<&'static str>,
) -> ClaimedLegacyActivation<'static> {
    ClaimedLegacyActivation {
        network: net,
        at: at(height, tx_index, if height == 0 { 1 } else { height as u8 }),
        transaction_type: ty,
        claimed_sender,
        claimed_code_hash: code_hash,
    }
}

const LTC: TradeLayerMainnet = TradeLayerMainnet::Litecoin;
const BTC: TradeLayerMainnet = TradeLayerMainnet::Bitcoin;
const DOGE: TradeLayerMainnet = TradeLayerMainnet::Dogecoin;
const ADMIN: &str = "MTmoypkhRQoJ172ZqxcsVumPZfJ8KCrQCB";

#[test]
fn exact_36_legacy_activation_names_preserve_dispatcher_conflicts() {
    let frozen = include_str!("../fixtures/legacy_activation_js_names.tsv")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (id, name) = line.split_once('\t').unwrap();
            (id.parse::<usize>().unwrap(), name)
        })
        .collect::<Vec<_>>();
    assert_eq!(frozen.len(), LEGACY_ACTIVATION_TYPE_COUNT);
    for (index, &(id, name)) in frozen.iter().enumerate() {
        assert_eq!(id, index);
        assert_eq!(name, LEGACY_ACTIVATION_NAMES[id]);
        assert_eq!(
            UnverifiedLegacyActivationRegistry::name(id as u8),
            Some(name)
        );
    }
    assert_eq!(UnverifiedLegacyActivationRegistry::name(36), None);
    // Do NOT remap these to protocol::TRANSACTION_CATALOG automatically.
    assert_eq!(LEGACY_ACTIVATION_NAMES[31], "Batch Move Zk Rollup");
    assert_eq!(LEGACY_ACTIVATION_NAMES[32], "Publish New Tx");
    assert_ne!(
        LEGACY_ACTIVATION_NAMES[31],
        tradelayer_core::protocol::lookup_transaction(31)
            .unwrap()
            .name
    );
    assert_ne!(
        LEGACY_ACTIVATION_NAMES[32],
        tradelayer_core::protocol::lookup_transaction(32)
            .unwrap()
            .name
    );
}

#[test]
fn six_pinned_admin_profiles_and_hardcoded_ltc_genesis_admin() {
    let checks = [
        (BTC, false, "bc1qktknrnx2jcchjht9anz0uy8ae02xryxq2vxeem"),
        (BTC, true, "tb1q8f84erfegxhaylmvpfll9m5rgwymqy4akjnnvq"),
        (LTC, false, ADMIN),
        (LTC, true, "tltc1qa0kd2d39nmeph3hvcx8ytv65ztcywg5sazhtw8"),
        (DOGE, false, "DLSfu9qvEggkeXAgCAwBBw5BVLvMCtkewz"),
        (DOGE, true, "nop27JQWbGr95ySHXZMzCg8XXxYzbCBZAW"),
    ];
    for (network, testnet, admin) in checks {
        assert_eq!(legacy_admin_address(network, testnet), admin);
        let registry = UnverifiedLegacyActivationRegistry::new(network, testnet);
        assert_eq!(registry.network(), network);
        assert_eq!(registry.testnet(), testnet);
        assert_eq!(registry.source_admin(), admin);
        assert_eq!(registry.active_count(), 0);
    }
    assert!(legacy_is_sender_genesis_admin(
        PINNED_GENESIS_ADMIN_LITECOIN_TESTNET
    ));
    // Pinned source oddly hardcodes LTC testnet even when chain is BTC/DOGE.
    assert!(!legacy_is_sender_genesis_admin(ADMIN));
}

#[test]
fn initially_all_36_types_inactive_even_if_type_known() {
    let registry = UnverifiedLegacyActivationRegistry::new(LTC, false);
    for ty in 0..36 {
        assert!(!registry.source_is_tx_type_active(ty));
        assert_eq!(registry.source_get_activation_block(ty), None);
        assert_eq!(registry.source_check_activation_block(ty), None);
        assert_eq!(registry.source_code_hash(ty), None);
        assert_eq!(registry.slot(ty).unwrap().active(), false);
    }
    assert_eq!(registry.slot(36), None);
    assert_eq!(registry.source_check_activation_block(36), None);
    assert_eq!(registry.active_count(), 0);
    assert_eq!(registry.last_claimed(), None);
    assert_eq!(registry.claimed_event_count(), 0);
    assert!(!registry.source_are_activations_above_threshold());
    assert_eq!(
        registry.require_authenticated_legacy_activation(),
        Err(LegacyActivationError::HistoricalActivationAuthorityUnavailable)
    );
    assert_eq!(
        registry.require_trusted_protocol_activation(),
        Err(LegacyActivationError::TrustedProtocolActivationUnavailable)
    );
}

#[test]
fn source_reactivation_overwrites_height_but_preserves_hash_if_absent() {
    let mut registry = UnverifiedLegacyActivationRegistry::new(LTC, false);
    registry
        .apply_claimed_batch(&[e(LTC, 7, 10, 1, ADMIN, Some("abc123"))])
        .unwrap();
    assert!(registry.source_is_tx_type_active(7));
    assert_eq!(registry.source_get_activation_block(7), Some(10));
    assert_eq!(registry.source_code_hash(7), Some("abc123"));

    // Historical JS explicitly allows repeating the same active type.
    registry
        .apply_claimed_batch(&[e(LTC, 7, 11, 1, ADMIN, None)])
        .unwrap();
    assert_eq!(registry.source_get_activation_block(7), Some(11));
    assert_eq!(registry.source_code_hash(7), Some("abc123"));

    registry
        .apply_claimed_batch(&[e(LTC, 7, 12, 1, ADMIN, Some(""))])
        .unwrap();
    assert_eq!(registry.source_get_activation_block(7), Some(12));
    assert_eq!(registry.source_code_hash(7), Some("abc123"));
    registry
        .apply_claimed_batch(&[e(LTC, 7, 13, 1, ADMIN, Some("def456"))])
        .unwrap();
    assert_eq!(registry.source_get_activation_block(7), Some(13));
    assert_eq!(registry.source_code_hash(7), Some("def456"));
    assert_eq!(registry.active_count(), 1);
    assert_eq!(registry.claimed_event_count(), 4);
}

#[test]
fn source_get_block_null_for_zero_while_check_block_returns_zero() {
    let mut registry = UnverifiedLegacyActivationRegistry::new(LTC, false);
    registry
        .apply_claimed_batch(&[e(LTC, 9, 0, 1, ADMIN, Some("0"))])
        .unwrap();
    assert!(registry.source_is_tx_type_active(9));
    assert_eq!(registry.source_get_activation_block(9), None);
    assert_eq!(registry.source_check_activation_block(9), Some(0));
    assert_eq!(registry.source_code_hash(9), Some("0"));
}

#[test]
fn strictly_above_ninety_percent_requires_33_of_36_flags() {
    let mut registry = UnverifiedLegacyActivationRegistry::new(LTC, false);
    let events = (1..=32)
        .map(|ty| e(LTC, ty, 20, ty as u32, ADMIN, None))
        .collect::<Vec<_>>();
    registry.apply_claimed_batch(&events).unwrap();
    assert_eq!(registry.active_count(), 32);
    assert!(!registry.source_are_activations_above_threshold());
    registry
        .apply_claimed_batch(&[e(LTC, 33, 20, 33, ADMIN, None)])
        .unwrap();
    assert_eq!(registry.active_count(), 33);
    assert!(registry.source_are_activations_above_threshold());
    assert!(!registry.source_is_tx_type_active(0));
    assert_eq!(
        registry.require_trusted_protocol_activation(),
        Err(LegacyActivationError::TrustedProtocolActivationUnavailable)
    );
}

#[test]
fn genesis_type_zero_denied_atomically_including_mixed_batches() {
    let mut registry = UnverifiedLegacyActivationRegistry::new(BTC, false);
    let sender = legacy_admin_address(BTC, false);
    let original = registry.clone();
    let batch = [
        e(BTC, 2, 21, 0, sender, None),
        e(BTC, 0, 21, 1, sender, None),
    ];
    assert_eq!(
        registry.apply_claimed_batch(&batch),
        Err(LegacyActivationError::UnverifiedGenesisTokenInitialization)
    );
    assert_eq!(registry, original);
    assert!(!registry.source_is_tx_type_active(0));
    assert_eq!(registry.claimed_event_count(), 0);
}

#[test]
fn wrong_network_admin_order_hash_and_missing_block_id_fail_atomically() {
    let mut registry = UnverifiedLegacyActivationRegistry::new(LTC, false);
    let original = registry.clone();
    assert_eq!(
        registry.apply_claimed_batch(&[]),
        Err(LegacyActivationError::EmptyBatch)
    );
    let over = vec![e(LTC, 7, 10, 1, ADMIN, None); MAX_ACTIVATION_BATCH + 1];
    assert_eq!(
        registry.apply_claimed_batch(&over),
        Err(LegacyActivationError::TooManyEvents)
    );
    assert_eq!(
        registry.apply_claimed_batch(&[e(BTC, 7, 10, 1, ADMIN, None)]),
        Err(LegacyActivationError::WrongNetwork)
    );
    assert_eq!(
        registry.apply_claimed_batch(&[e(LTC, 7, 10, 1, "wrong", None)]),
        Err(LegacyActivationError::WrongClaimedAdmin)
    );
    assert_eq!(
        registry.apply_claimed_batch(&[e(LTC, 36, 10, 1, ADMIN, None)]),
        Err(LegacyActivationError::UnknownType(36))
    );
    assert_eq!(
        registry.apply_claimed_batch(&[e(LTC, 7, 10, 1, ADMIN, Some("bad\nhash"))]),
        Err(LegacyActivationError::InvalidCodeHash)
    );
    let oversized_hash = "x".repeat(257);
    let oversized = ClaimedLegacyActivation {
        network: LTC,
        at: at(10, 1, 10),
        transaction_type: 7,
        claimed_sender: ADMIN,
        claimed_code_hash: Some(&oversized_hash),
    };
    assert_eq!(
        registry.apply_claimed_batch(&[oversized]),
        Err(LegacyActivationError::InvalidCodeHash)
    );
    let mut missing = e(LTC, 7, 10, 1, ADMIN, None);
    missing.at.block_id = [0; 32];
    assert_eq!(
        registry.apply_claimed_batch(&[missing]),
        Err(LegacyActivationError::MissingBlockId)
    );
    assert_eq!(registry, original);
}

#[test]
fn same_height_fork_duplicate_position_and_rollback_projection_fail_closed() {
    let mut registry = UnverifiedLegacyActivationRegistry::new(LTC, false);
    let base = e(LTC, 7, 10, 1, ADMIN, Some("old"));
    registry.apply_claimed_batch(&[base]).unwrap();
    let snapshot = registry.clone();
    assert_eq!(
        registry.apply_claimed_batch(&[base]),
        Err(LegacyActivationError::OutOfOrderClaimedEvents)
    );
    let mut fork = e(LTC, 9, 10, 2, ADMIN, None);
    fork.at.block_id = [0xa5; 32];
    assert_eq!(
        registry.apply_claimed_batch(&[fork]),
        Err(LegacyActivationError::ConflictingClaimedBlockAtHeight)
    );
    let backward = e(LTC, 5, 9, 2, ADMIN, None);
    assert_eq!(
        registry.apply_claimed_batch(&[backward]),
        Err(LegacyActivationError::OutOfOrderClaimedEvents)
    );
    assert_eq!(registry, snapshot);
    let later = e(LTC, 9, 11, 1, ADMIN, Some("new"));
    registry.apply_claimed_batch(&[later]).unwrap();
    assert!(registry.source_is_tx_type_active(9));
    // Exact reversible diagnostic snapshot; caller chooses the rollback
    // point and no active-chain/best-work selection is claimed.
    registry = snapshot;
    assert!(!registry.source_is_tx_type_active(9));
    assert!(registry.source_is_tx_type_active(7));
    assert_eq!(registry.claimed_event_count(), 1);
}

#[test]
fn source_accepts_type34_flag_but_it_never_disambiguates_protocol_operation() {
    let mut registry = UnverifiedLegacyActivationRegistry::new(DOGE, false);
    let sender = legacy_admin_address(DOGE, false);
    registry
        .apply_claimed_batch(&[e(DOGE, 34, 7, 1, sender, None)])
        .unwrap();
    assert!(registry.source_is_tx_type_active(34));
    assert_eq!(
        registry.require_trusted_protocol_activation(),
        Err(LegacyActivationError::TrustedProtocolActivationUnavailable)
    );
}
