//! Stage 22: one spendable Ledger, one registry and one input replay set in
//! each fully independent Bitcoin/Litecoin/Dogecoin TradeLayer simulation.
//! All actors, blocks and txids in these fixtures are explicitly UNVERIFIED.

use std::collections::BTreeSet;
use tradelayer_core::chain::{BlockHeader, BlockId, Tip};
use tradelayer_core::joint::{JointError, JointOperation, UnverifiedJointTx};
use tradelayer_core::joint_chain::{JointChainError, JointChainState, UnverifiedJointBlock};
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_bound::NetworkBoundSpot;
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::registry::{Registry, RegistryAction, RegistryTx};
use tradelayer_core::spot::{
    CancelSelector, SpotExchange, SpotIntent, SpotPair, UnverifiedSpotPlacement,
};
use tradelayer_core::spot_chain::{UnverifiedSpotBlock, UnverifiedSpotOperation};

fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn a(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn tx(n: u8) -> TxId {
    TxId::from_bytes([n; 32])
}
fn anchor(network: TradeLayerMainnet) -> Tip {
    Tip {
        height: 0,
        id: network.genesis(),
    }
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
fn permitted() -> BTreeSet<PropertyId> {
    BTreeSet::from([p(7), p(8)])
}
fn new(network: TradeLayerMainnet) -> JointChainState {
    JointChainState::new_unverified(
        network,
        anchor(network),
        a("admin"),
        ledger(),
        permitted(),
        4,
    )
    .unwrap()
}
fn jt(n: u8, who: &str, operation: JointOperation) -> UnverifiedJointTx {
    UnverifiedJointTx {
        txid: tx(n),
        actor: a(who),
        operation,
    }
}
fn block(
    state: &JointChainState,
    marker: u8,
    transactions: Vec<UnverifiedJointTx>,
) -> UnverifiedJointBlock {
    UnverifiedJointBlock {
        network: state.network(),
        header: BlockHeader {
            height: state.tip().height + 1,
            id: BlockId([marker; 32]),
            parent: state.tip().id,
        },
        transactions,
    }
}
fn activate() -> UnverifiedJointTx {
    jt(
        10,
        "admin",
        JointOperation::Registry(RegistryAction::Activate {
            types: vec![1, 2, 5, 6, 11, 12],
            code_hash: [0x29; 32],
        }),
    )
}
fn place(sell: bool, n: u8, offered: u64, wanted: u64) -> UnverifiedJointTx {
    jt(
        n,
        if sell { "seller" } else { "buyer" },
        JointOperation::SpotPlace(SpotIntent {
            offered: if sell { p(7) } else { p(8) },
            desired: if sell { p(8) } else { p(7) },
            offered_atoms: offered,
            expected_atoms: wanted,
            post: false,
            stop: false,
        }),
    )
}
fn cancel(n: u8, who: &str) -> UnverifiedJointTx {
    jt(
        n,
        who,
        JointOperation::SpotCancel {
            offered: p(7),
            desired: p(8),
            is_contract: false,
            selector: CancelSelector::All,
        },
    )
}
fn start(network: TradeLayerMainnet) -> JointChainState {
    let mut state = new(network);
    state
        .apply_unverified_block(&block(&state, 20, vec![activate()]))
        .unwrap();
    state
}

#[test]
fn mixed_send_issue_spot_match_and_cancel_use_one_ledger() {
    let mut chain = start(TradeLayerMainnet::Litecoin);
    let b2 = block(
        &chain,
        21,
        vec![
            jt(
                11,
                "seller",
                JointOperation::Send {
                    id: p(7),
                    recipient: a("alice"),
                    amount_atoms: 100,
                },
            ),
            jt(
                12,
                "minter",
                JointOperation::Issue {
                    id: p(9),
                    kind: PropertyKind::Fixed,
                    initial_atoms: 50,
                },
            ),
        ],
    );
    chain.apply_unverified_block(&b2).unwrap();
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("seller")),
        900
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("alice")),
        100
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(9), &a("minter")),
        50
    );
    let before_market = chain.clone();

    // Buyer acquires half the resting seller's base units. Both transfers,
    // the previous send, and the issuer's newly created asset share a Ledger.
    let b3 = block(
        &chain,
        22,
        vec![place(true, 13, 200, 400), place(false, 14, 220, 100)],
    );
    chain.apply_unverified_block(&b3).unwrap();
    let market = chain.state().spot();
    assert_eq!(market.fills().len(), 1);
    assert_eq!(market.fills()[0].base_atoms, 100);
    assert_eq!(market.fills()[0].quote_atoms, 220);
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("seller")),
        700
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("alice")),
        100
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("buyer")),
        100
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(8), &a("seller")),
        220
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(8), &a("buyer")),
        780
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(9), &a("minter")),
        50
    );
    assert_eq!(market.open_count(), 1);
    assert_eq!(
        market.open_orders(SpotPair::new(p(7), p(8)).unwrap())[0].remaining_offered,
        100
    );
    chain.state().verify_components_unverified().unwrap();

    let snapshot = chain.checkpoint_bytes().unwrap();
    let recovered = JointChainState::from_checkpoint_bytes(
        &snapshot,
        TradeLayerMainnet::Litecoin,
        anchor(TradeLayerMainnet::Litecoin),
        &a("admin"),
        &permitted(),
        4,
    )
    .unwrap();
    assert_eq!(recovered, chain);
    assert_eq!(recovered.checkpoint_bytes().unwrap(), snapshot);

    // Cancel returns exactly the unspent 100 base units to the same ledger.
    chain
        .apply_unverified_block(&block(&chain, 23, vec![cancel(15, "seller")]))
        .unwrap();
    assert_eq!(chain.state().spot().open_count(), 0);
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("seller")),
        800
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("alice")),
        100
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("buyer")),
        100
    );
    assert_eq!(chain.state().processed_count(), 6);
    chain.state().ledger().verify_conservation().unwrap();

    chain.rollback_tip(chain.tip()).unwrap();
    assert_eq!(chain, recovered);
    chain.rollback_tip(chain.tip()).unwrap();
    assert_eq!(chain, before_market);
}

#[test]
fn ordinary_transfer_can_only_spend_available_balance_outside_spot_escrow() {
    let mut chain = start(TradeLayerMainnet::Bitcoin);
    chain
        .apply_unverified_block(&block(&chain, 21, vec![place(true, 11, 950, 1900)]))
        .unwrap();
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("seller")),
        50
    );
    let previous = chain.clone();
    assert!(matches!(
        chain.apply_unverified_block(&block(
            &chain,
            22,
            vec![jt(
                12,
                "seller",
                JointOperation::Send {
                    id: p(7),
                    recipient: a("attacker"),
                    amount_atoms: 100
                }
            )]
        )),
        Err(JointChainError::Joint(JointError::Spot(_)))
    ));
    assert_eq!(chain, previous);
    let spoof = jt(
        13,
        "~spot:evil",
        JointOperation::Send {
            id: p(7),
            recipient: a("attacker"),
            amount_atoms: 25,
        },
    );
    assert!(matches!(
        chain.apply_unverified_block(&block(&chain, 22, vec![spoof])),
        Err(JointChainError::Joint(JointError::Spot(_)))
    ));
    assert_eq!(chain, previous);
    chain.state().verify_components_unverified().unwrap();
}

#[test]
fn one_global_replay_set_rejects_reuse_across_registry_ledger_and_spot() {
    let mut chain = start(TradeLayerMainnet::Dogecoin);
    let initial = chain.clone();
    let collide_activation = place(true, 10, 10, 20);
    assert!(matches!(
        chain.apply_unverified_block(&block(&chain, 21, vec![collide_activation])),
        Err(JointChainError::Joint(JointError::Replay(_)))
    ));
    assert_eq!(chain, initial);
    chain
        .apply_unverified_block(&block(
            &chain,
            21,
            vec![jt(
                11,
                "seller",
                JointOperation::Send {
                    id: p(7),
                    recipient: a("alice"),
                    amount_atoms: 20,
                },
            )],
        ))
        .unwrap();
    let before = chain.clone();
    assert!(matches!(
        chain.apply_unverified_block(&block(&chain, 22, vec![place(true, 11, 10, 20)])),
        Err(JointChainError::Joint(JointError::Replay(_)))
    ));
    assert_eq!(chain, before);
    chain
        .apply_unverified_block(&block(&chain, 22, vec![place(true, 12, 10, 20)]))
        .unwrap();
    let saved = chain.clone();
    assert!(matches!(
        chain.apply_unverified_block(&block(
            &chain,
            23,
            vec![jt(
                12,
                "admin",
                JointOperation::Registry(RegistryAction::Activate {
                    types: vec![13],
                    code_hash: [7; 32]
                })
            ),]
        )),
        Err(JointChainError::Joint(JointError::Replay(_)))
    ));
    assert_eq!(chain, saved);
}

#[test]
fn rejected_settlement_type3_and_unported_channel_operations_roll_back_block() {
    let mut chain = start(TradeLayerMainnet::Litecoin);
    let before = chain.clone();
    for operation in [
        JointOperation::UtxoTradeType3,
        JointOperation::EconomicSettlement { ty: 23 },
        JointOperation::EconomicSettlement { ty: 31 },
        JointOperation::Unsupported { ty: 4 },
        JointOperation::Unsupported { ty: 21 },
    ] {
        let request = block(
            &chain,
            21,
            vec![
                jt(
                    11,
                    "seller",
                    JointOperation::Send {
                        id: p(7),
                        recipient: a("alice"),
                        amount_atoms: 100,
                    },
                ),
                jt(12, "seller", operation),
            ],
        );
        assert!(chain.apply_unverified_block(&request).is_err());
        assert_eq!(chain, before);
    }
}

#[test]
fn activation_in_same_block_is_insufficient_and_failure_is_atomic() {
    let mut chain = new(TradeLayerMainnet::Bitcoin);
    let before = chain.clone();
    let same = block(&chain, 20, vec![activate(), place(true, 11, 10, 20)]);
    assert!(matches!(
        chain.apply_unverified_block(&same),
        Err(JointChainError::Joint(JointError::ActivatedInSameBlock(5)))
    ));
    assert_eq!(chain, before);
    chain
        .apply_unverified_block(&block(&chain, 20, vec![activate()]))
        .unwrap();
    assert_eq!(chain.state().processed_count(), 1);
}

#[test]
fn mixed_batch_fails_atomically_if_late_order_reservation_exceeds_balance() {
    let mut chain = start(TradeLayerMainnet::Litecoin);
    let before = chain.clone();
    let b = block(
        &chain,
        21,
        vec![
            jt(
                11,
                "seller",
                JointOperation::Send {
                    id: p(7),
                    recipient: a("alice"),
                    amount_atoms: 100,
                },
            ),
            place(true, 12, 950, 1900),
        ],
    );
    assert!(chain.apply_unverified_block(&b).is_err());
    assert_eq!(chain, before);
    assert_eq!(chain.state().ledger().available_atoms(p(7), &a("alice")), 0);
    assert_eq!(chain.state().processed_count(), 1);
}

#[test]
fn three_mainnets_preserve_independent_property_and_transaction_histories() {
    let networks = [
        TradeLayerMainnet::Bitcoin,
        TradeLayerMainnet::Litecoin,
        TradeLayerMainnet::Dogecoin,
    ];
    let mut states = Vec::new();
    for (i, net) in networks.into_iter().enumerate() {
        let mut state = start(net);
        let amount = 20 + i as u128 * 10;
        state
            .apply_unverified_block(&block(
                &state,
                21,
                vec![jt(
                    11,
                    "seller",
                    JointOperation::Send {
                        id: p(7),
                        recipient: a("alice"),
                        amount_atoms: amount,
                    },
                )],
            ))
            .unwrap();
        assert_eq!(
            state.state().ledger().available_atoms(p(7), &a("alice")),
            amount
        );
        let bytes = state.checkpoint_bytes().unwrap();
        assert_eq!(
            JointChainState::from_checkpoint_bytes(
                &bytes,
                net,
                anchor(net),
                &a("admin"),
                &permitted(),
                4
            )
            .unwrap(),
            state
        );
        for other in networks {
            if other != net {
                assert!(JointChainState::from_checkpoint_bytes(
                    &bytes,
                    other,
                    anchor(other),
                    &a("admin"),
                    &permitted(),
                    4
                )
                .is_err());
            }
        }
        states.push(state);
    }
    assert_ne!(
        states[0].checkpoint_bytes().unwrap(),
        states[1].checkpoint_bytes().unwrap()
    );
    assert_ne!(
        states[1].checkpoint_bytes().unwrap(),
        states[2].checkpoint_bytes().unwrap()
    );
}

#[test]
fn fork_reorganization_restores_both_spot_and_registry_as_one_snapshot() {
    let mut chain = start(TradeLayerMainnet::Bitcoin);
    chain
        .apply_unverified_block(&block(&chain, 21, vec![place(true, 11, 100, 200)]))
        .unwrap();
    let parent = chain.clone();
    chain
        .apply_unverified_block(&block(&chain, 22, vec![place(false, 12, 220, 100)]))
        .unwrap();
    assert_eq!(chain.state().spot().fills().len(), 1);
    chain.rollback_tip(chain.tip()).unwrap();
    assert_eq!(chain, parent);
    chain
        .apply_unverified_block(&block(
            &chain,
            42,
            vec![jt(
                12,
                "seller",
                JointOperation::Send {
                    id: p(7),
                    recipient: a("alice"),
                    amount_atoms: 25,
                },
            )],
        ))
        .unwrap();
    assert_eq!(chain.state().spot().fills().len(), 0);
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("alice")),
        25
    );
    let round_trip = JointChainState::from_checkpoint_bytes(
        &chain.checkpoint_bytes().unwrap(),
        TradeLayerMainnet::Bitcoin,
        anchor(TradeLayerMainnet::Bitcoin),
        &a("admin"),
        &permitted(),
        4,
    )
    .unwrap();
    assert_eq!(round_trip, chain);
}

#[test]
fn older_spot_checkpoint_has_explicit_one_ledger_migration_path() {
    let network = TradeLayerMainnet::Litecoin;
    let spot = SpotExchange::from_ledger_unverified(ledger(), permitted()).unwrap();
    let mut old = NetworkBoundSpot::new_unverified(network, anchor(network), spot, 4).unwrap();
    old.apply_unverified_block(&UnverifiedSpotBlock {
        chain: network.chain_id(),
        header: BlockHeader {
            height: 1,
            parent: anchor(network).id,
            id: BlockId([42; 32]),
        },
        operations: vec![UnverifiedSpotOperation::PlaceType5(
            UnverifiedSpotPlacement {
                txid: tx(50),
                actor: a("seller"),
                height: 1,
                intent: SpotIntent {
                    offered: p(7),
                    desired: p(8),
                    offered_atoms: 100,
                    expected_atoms: 200,
                    stop: false,
                    post: false,
                },
            },
        )],
    })
    .unwrap();
    let mut registry = Registry::new(a("admin"));
    registry
        .apply(RegistryTx {
            txid: tx(10),
            actor: a("admin"),
            height: 1,
            action: RegistryAction::Activate {
                types: vec![2, 5, 6],
                code_hash: [9; 32],
            },
        })
        .unwrap();
    let mut joint = JointChainState::migrate_from_spot_unverified(&old, registry, 4).unwrap();
    assert_eq!(joint.anchor(), old.tip());
    assert_eq!(joint.state().processed_count(), 2);
    assert_eq!(joint.state().spot().open_count(), 1);
    assert_eq!(
        joint.state().ledger().available_atoms(p(7), &a("seller")),
        900
    );
    let bytes = joint.checkpoint_bytes().unwrap();
    let restored = JointChainState::from_checkpoint_bytes(
        &bytes,
        network,
        old.tip(),
        &a("admin"),
        &permitted(),
        4,
    )
    .unwrap();
    assert_eq!(restored, joint);
    joint
        .apply_unverified_block(&block(&joint, 51, vec![cancel(52, "seller")]))
        .unwrap();
    assert_eq!(
        joint.state().ledger().available_atoms(p(7), &a("seller")),
        1000
    );
    joint.rollback_tip(joint.tip()).unwrap();
    assert_eq!(joint, restored);
    assert!(JointChainState::from_checkpoint_bytes(
        &bytes,
        network,
        anchor(network),
        &a("admin"),
        &permitted(),
        4
    )
    .is_err());
}

#[test]
fn old_checkpoint_formats_wrong_admin_and_recomputed_crc_forgery_fail_closed() {
    let mut chain = start(TradeLayerMainnet::Litecoin);
    chain
        .apply_unverified_block(&block(&chain, 21, vec![place(true, 11, 100, 200)]))
        .unwrap();
    let bytes = chain.checkpoint_bytes().unwrap();
    let parse = |input: &[u8], net, admin: &str| {
        JointChainState::from_checkpoint_bytes(input, net, anchor(net), &a(admin), &permitted(), 4)
            .is_err()
    };
    assert!(parse(&bytes, TradeLayerMainnet::Bitcoin, "admin"));
    assert!(parse(&bytes, TradeLayerMainnet::Litecoin, "wrong"));
    assert!(parse(
        &bytes[..bytes.len() - 1],
        TradeLayerMainnet::Litecoin,
        "admin"
    ));
    let mut tampered = bytes.clone();
    tampered[25] ^= 1;
    assert!(parse(&tampered, TradeLayerMainnet::Litecoin, "admin"));
    let old = chain.state().spot().snapshot_bytes().unwrap();
    assert!(parse(&old, TradeLayerMainnet::Litecoin, "admin"));
    let mut forged = bytes.clone();
    forged[6] = TradeLayerMainnet::Dogecoin.tag();
    let len = forged.len() - 4;
    let mut crc = !0u32;
    for b in &forged[..len] {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    forged[len..].copy_from_slice(&(!crc).to_be_bytes());
    assert!(parse(&forged, TradeLayerMainnet::Litecoin, "admin"));
}

#[test]
fn immutable_checkpoint_write_failure_preserves_registry_and_ledger() {
    let mut chain = new(TradeLayerMainnet::Dogecoin);
    let prior = chain.clone();
    let path = std::env::temp_dir().join(format!(
        "tlr-joint-stage22-collision-{}-{}",
        std::process::id(),
        prior.tip().height
    ));
    std::fs::write(&path, b"immutable-existing").unwrap();
    let next = block(&chain, 20, vec![activate()]);
    assert!(chain
        .apply_unverified_block_and_checkpoint(&next, &path)
        .is_err());
    assert_eq!(chain, prior);
    assert_eq!(std::fs::read(&path).unwrap(), b"immutable-existing");
    std::fs::remove_file(path).unwrap();

    let path = std::env::temp_dir().join(format!(
        "tlr-joint-stage22-success-{}-{}",
        std::process::id(),
        prior.tip().height
    ));
    chain
        .apply_unverified_block_and_checkpoint(&next, &path)
        .unwrap();
    let restored = JointChainState::load_checkpoint(
        &path,
        TradeLayerMainnet::Dogecoin,
        anchor(TradeLayerMainnet::Dogecoin),
        &a("admin"),
        &permitted(),
        4,
    )
    .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(restored, chain);
}

#[test]
fn managed_grant_and_redeem_account_on_the_same_spot_ledger() {
    let mut chain = start(TradeLayerMainnet::Bitcoin);
    chain
        .apply_unverified_block(&block(
            &chain,
            21,
            vec![
                jt(
                    11,
                    "buyer",
                    JointOperation::Grant {
                        id: p(8),
                        recipient: a("seller"),
                        amount_atoms: 50,
                    },
                ),
                jt(
                    12,
                    "buyer",
                    JointOperation::Redeem {
                        id: p(8),
                        amount_atoms: 20,
                    },
                ),
            ],
        ))
        .unwrap();
    assert_eq!(
        chain
            .state()
            .ledger()
            .property(p(8))
            .unwrap()
            .total_supply_atoms,
        1030
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(8), &a("buyer")),
        980
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(8), &a("seller")),
        50
    );

    chain
        .apply_unverified_block(&block(
            &chain,
            22,
            vec![place(true, 13, 100, 200), place(false, 14, 220, 100)],
        ))
        .unwrap();
    assert_eq!(chain.state().spot().fills().len(), 1);
    assert_eq!(
        chain
            .state()
            .ledger()
            .property(p(8))
            .unwrap()
            .total_supply_atoms,
        1030
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(8), &a("buyer")),
        760
    );
    assert_eq!(
        chain.state().ledger().available_atoms(p(8), &a("seller")),
        270
    );
    chain.state().verify_components_unverified().unwrap();
}

#[test]
fn mixed_block_rollback_restores_registry_activation_and_balances() {
    let mut chain = start(TradeLayerMainnet::Litecoin);
    let before = chain.clone();
    chain
        .apply_unverified_block(&block(
            &chain,
            21,
            vec![
                jt(
                    11,
                    "admin",
                    JointOperation::Registry(RegistryAction::Activate {
                        types: vec![13],
                        code_hash: [19; 32],
                    }),
                ),
                jt(
                    12,
                    "seller",
                    JointOperation::Send {
                        id: p(7),
                        recipient: a("alice"),
                        amount_atoms: 45,
                    },
                ),
            ],
        ))
        .unwrap();
    assert_eq!(chain.state().registry().activation(13).unwrap().height, 2);
    assert_eq!(
        chain.state().ledger().available_atoms(p(7), &a("alice")),
        45
    );
    let bytes = chain.checkpoint_bytes().unwrap();
    let restored = JointChainState::from_checkpoint_bytes(
        &bytes,
        TradeLayerMainnet::Litecoin,
        anchor(TradeLayerMainnet::Litecoin),
        &a("admin"),
        &permitted(),
        4,
    )
    .unwrap();
    assert_eq!(restored, chain);
    chain.rollback_tip(chain.tip()).unwrap();
    assert_eq!(chain, before);
    assert!(chain.state().registry().activation(13).is_none());
    assert_eq!(chain.state().ledger().available_atoms(p(7), &a("alice")), 0);
}

#[test]
fn spot_migration_refuses_reused_registry_and_spot_input_ids() {
    let network = TradeLayerMainnet::Dogecoin;
    let initial = SpotExchange::from_ledger_unverified(ledger(), permitted()).unwrap();
    let mut old = NetworkBoundSpot::new_unverified(network, anchor(network), initial, 4).unwrap();
    old.apply_unverified_block(&UnverifiedSpotBlock {
        chain: network.chain_id(),
        header: BlockHeader {
            height: 1,
            parent: network.genesis(),
            id: BlockId([45; 32]),
        },
        operations: vec![UnverifiedSpotOperation::PlaceType5(
            UnverifiedSpotPlacement {
                txid: tx(50),
                actor: a("seller"),
                height: 1,
                intent: SpotIntent {
                    offered: p(7),
                    desired: p(8),
                    offered_atoms: 40,
                    expected_atoms: 80,
                    stop: false,
                    post: false,
                },
            },
        )],
    })
    .unwrap();

    let mut registry = Registry::new(a("admin"));
    registry
        .apply(RegistryTx {
            txid: tx(50),
            actor: a("admin"),
            height: 1,
            action: RegistryAction::Activate {
                types: vec![5, 6],
                code_hash: [20; 32],
            },
        })
        .unwrap();
    assert!(matches!(
        JointChainState::migrate_from_spot_unverified(&old, registry, 4),
        Err(JointChainError::Joint(JointError::ReplayConflict))
    ));
}
