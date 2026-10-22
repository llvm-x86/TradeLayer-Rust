//! Stage 21: three independent TradeLayer mainnets; no cross-chain ledger,
//! no cross-chain proof promotion and no Raito/Cairo eligibility for Scrypt.

use std::collections::BTreeSet;
use tradelayer_core::chain::{BlockHeader, BlockId, CandidateBlock, ChainId, Tip};
use tradelayer_core::execution::ProtocolOperation;
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_bound::{NetworkBoundSpot, NetworkBoundUnified, ProfileReplayError};
use tradelayer_core::network_profile::{
    EconomicProofStatus, HeaderEvidence, NetworkAssetId, NetworkProfileError, PowFamily,
    RaitoCairoStatus, TradeLayerMainnet, BITCOIN_MAINNET_GENESIS, DOGECOIN_MAINNET_GENESIS,
    LITECOIN_MAINNET_GENESIS,
};
use tradelayer_core::registry::RegistryAction;
use tradelayer_core::spot::{SpotExchange, SpotIntent, SpotPair, UnverifiedSpotPlacement};
use tradelayer_core::spot_chain::{UnverifiedSpotBlock, UnverifiedSpotOperation};
use tradelayer_core::unified_chain::{AuthenticatedProtocolOperation, TrustedProtocolAdapter};

fn account(value: &str) -> AccountId {
    AccountId::try_from(value).unwrap()
}
fn id(value: u32) -> PropertyId {
    PropertyId::try_from(value).unwrap()
}
fn txid(value: u8) -> TxId {
    TxId::from_bytes([value; 32])
}
fn anchor(network: TradeLayerMainnet) -> Tip {
    Tip {
        height: 0,
        id: network.genesis(),
    }
}
fn header(parent: Tip, marker: u8) -> BlockHeader {
    BlockHeader {
        height: parent.height + 1,
        id: BlockId([marker; 32]),
        parent: parent.id,
    }
}

#[test]
fn pinned_genesis_hashes_are_exact_mainnet_core_constants() {
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
    assert_eq!(
        hex(&BITCOIN_MAINNET_GENESIS.0),
        "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
    );
    assert_eq!(
        hex(&LITECOIN_MAINNET_GENESIS.0),
        "12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2"
    );
    assert_eq!(
        hex(&DOGECOIN_MAINNET_GENESIS.0),
        "1a91e3dace36e2be3bf030a65679fe821aa1d6ef92e7c9902eb318182c355691"
    );
}

#[test]
fn native_pow_profiles_and_cairo_raito_paths_never_cross() {
    use TradeLayerMainnet::{Bitcoin, Dogecoin, Litecoin};
    assert_eq!(Bitcoin.pow(), PowFamily::BitcoinSha256d);
    assert_eq!(Litecoin.pow(), PowFamily::LitecoinScrypt);
    assert_eq!(Dogecoin.pow(), PowFamily::DogecoinScryptAuxPow);
    assert_eq!(
        Bitcoin.header_evidence(),
        HeaderEvidence::BitcoinHeaderResearchOnly
    );
    assert_eq!(
        Litecoin.header_evidence(),
        HeaderEvidence::ExternalFullNodeRequired
    );
    assert_eq!(
        Dogecoin.header_evidence(),
        HeaderEvidence::ExternalFullNodeRequired
    );
    assert_eq!(
        Bitcoin.raito_cairo(),
        RaitoCairoStatus::BitcoinSha256dResearchOnly
    );
    assert_eq!(
        Litecoin.raito_cairo(),
        RaitoCairoStatus::UnavailableForScrypt
    );
    assert_eq!(
        Dogecoin.raito_cairo(),
        RaitoCairoStatus::UnavailableForScrypt
    );
    assert_eq!(Bitcoin.require_raito_research_eligibility(), Ok(()));
    assert_eq!(
        Litecoin.require_raito_research_eligibility(),
        Err(NetworkProfileError::BitcoinRaitoOnly)
    );
    assert_eq!(
        Dogecoin.require_raito_research_eligibility(),
        Err(NetworkProfileError::BitcoinRaitoOnly)
    );
    for network in [Bitcoin, Litecoin, Dogecoin] {
        assert_eq!(
            network.economic_proof(),
            EconomicProofStatus::MetadataAuditOnly
        );
        assert!(!network.economic_proof_authorizes_settlement());
        assert_eq!(network.chain_id().0, network.genesis().0);
        assert_eq!(TradeLayerMainnet::from_tag(network.tag()), Ok(network));
        assert_eq!(
            network.check_genesis_if_applicable(Tip {
                height: 0,
                id: Bitcoin.genesis(),
            }),
            if network == Bitcoin {
                Ok(())
            } else {
                Err(NetworkProfileError::WrongGenesisAnchor)
            }
        );
    }
}

#[test]
fn property_7_is_distinct_on_each_tradelayer_instance() {
    let a = NetworkAssetId::new(TradeLayerMainnet::Bitcoin, id(7));
    let b = NetworkAssetId::new(TradeLayerMainnet::Litecoin, id(7));
    let c = NetworkAssetId::new(TradeLayerMainnet::Dogecoin, id(7));
    assert_ne!(a, b);
    assert_ne!(b, c);
    assert_ne!(a, c);
    let all = BTreeSet::from([a, b, c]);
    assert_eq!(all.len(), 3);
    assert!(a.belongs_to(TradeLayerMainnet::Bitcoin));
    assert!(!a.belongs_to(TradeLayerMainnet::Litecoin));
}

struct FakeProtocolAdapter {
    supply: u128,
}
impl TrustedProtocolAdapter for FakeProtocolAdapter {
    fn verify_block(&self, _: &CandidateBlock) -> Result<(), String> {
        // This mock has no relationship to real PoW, scripts or UTXOs.
        Ok(())
    }
    fn authenticate_and_decode(
        &self,
        _network: ChainId,
        _header: &BlockHeader,
        raw: &[u8],
    ) -> Result<Option<AuthenticatedProtocolOperation>, String> {
        match raw {
            [1] => Ok(Some(AuthenticatedProtocolOperation {
                txid: txid(11),
                actor: account("admin"),
                operation: ProtocolOperation::Registry(RegistryAction::Activate {
                    types: vec![1],
                    code_hash: [0x23; 32],
                }),
            })),
            [2] => Ok(Some(AuthenticatedProtocolOperation {
                txid: txid(12),
                actor: account("issuer"),
                operation: ProtocolOperation::Issue {
                    id: id(7),
                    kind: PropertyKind::Fixed,
                    initial_atoms: self.supply,
                },
            })),
            _ => Err("unknown synthetic test transaction".into()),
        }
    }
}
fn block(network: TradeLayerMainnet, prior: Tip, marker: u8, raw: u8) -> CandidateBlock {
    CandidateBlock {
        chain: network.chain_id(),
        header: header(prior, marker),
        raw_transactions: vec![vec![raw]],
    }
}

#[test]
fn independent_unified_tradelayers_allow_same_property_and_local_txids() {
    let networks = [
        TradeLayerMainnet::Bitcoin,
        TradeLayerMainnet::Litecoin,
        TradeLayerMainnet::Dogecoin,
    ];
    let mut states = Vec::new();
    for (i, network) in networks.into_iter().enumerate() {
        let mut instance =
            NetworkBoundUnified::new_unverified(network, anchor(network), account("admin"), 3)
                .unwrap();
        let synthetic = FakeProtocolAdapter {
            supply: (i as u128 + 1) * 100,
        };
        instance
            .apply_block(&block(network, instance.tip(), 10, 1), &synthetic)
            .unwrap();
        instance
            .apply_block(&block(network, instance.tip(), 11, 2), &synthetic)
            .unwrap();
        assert_eq!(
            instance
                .state()
                .state()
                .ledger()
                .available_atoms(id(7), &account("issuer")),
            synthetic.supply
        );
        assert_eq!(instance.state().state().processed_count(), 2);
        states.push(instance);
    }
    assert_ne!(
        states[0].checkpoint_fingerprint_unverified().unwrap(),
        states[1].checkpoint_fingerprint_unverified().unwrap()
    );
    assert_ne!(
        states[1].checkpoint_fingerprint_unverified().unwrap(),
        states[2].checkpoint_fingerprint_unverified().unwrap()
    );
    for (i, instance) in states.iter().enumerate() {
        let bytes = instance.checkpoint_bytes().unwrap();
        let same = NetworkBoundUnified::from_checkpoint_bytes(
            &bytes,
            networks[i],
            anchor(networks[i]),
            &account("admin"),
            3,
        )
        .unwrap();
        assert_eq!(same, *instance);
        assert_eq!(same.checkpoint_bytes().unwrap(), bytes);
        for other in networks {
            if other != networks[i] {
                assert!(NetworkBoundUnified::from_checkpoint_bytes(
                    &bytes,
                    other,
                    anchor(other),
                    &account("admin"),
                    3
                )
                .is_err());
            }
        }
        let opposite = block(networks[(i + 1) % 3], instance.tip(), 33, 2);
        let mut saved = instance.clone();
        assert!(matches!(
            saved.apply_block(&opposite, &FakeProtocolAdapter { supply: 100 }),
            Err(ProfileReplayError::Unified(_))
        ));
        assert_eq!(saved, *instance);
    }
}

fn spot() -> SpotExchange {
    let mut ledger = Ledger::new();
    ledger
        .apply(
            txid(1),
            Operation::CreateProperty {
                id: id(7),
                issuer: account("seller"),
                kind: PropertyKind::Fixed,
                initial_atoms: 1000,
            },
        )
        .unwrap();
    ledger
        .apply(
            txid(2),
            Operation::CreateProperty {
                id: id(8),
                issuer: account("buyer"),
                kind: PropertyKind::Fixed,
                initial_atoms: 1000,
            },
        )
        .unwrap();
    SpotExchange::from_ledger_unverified(ledger, BTreeSet::from([id(7), id(8)])).unwrap()
}
fn spot_block(
    network: TradeLayerMainnet,
    parent: Tip,
    marker: u8,
    orders: Vec<UnverifiedSpotOperation>,
) -> UnverifiedSpotBlock {
    UnverifiedSpotBlock {
        chain: network.chain_id(),
        header: header(parent, marker),
        operations: orders,
    }
}
fn spot_order(n: u8, height: u64, sell: bool) -> UnverifiedSpotOperation {
    UnverifiedSpotOperation::PlaceType5(UnverifiedSpotPlacement {
        txid: txid(n),
        actor: account(if sell { "seller" } else { "buyer" }),
        height,
        intent: SpotIntent {
            offered: id(if sell { 7 } else { 8 }),
            desired: id(if sell { 8 } else { 7 }),
            offered_atoms: if sell { 100 } else { 220 },
            expected_atoms: if sell { 200 } else { 100 },
            stop: false,
            post: false,
        },
    })
}

#[test]
fn separate_spot_books_and_checkpoint_lanes_are_profile_bound() {
    let networks = [
        TradeLayerMainnet::Bitcoin,
        TradeLayerMainnet::Litecoin,
        TradeLayerMainnet::Dogecoin,
    ];
    for network in networks {
        let mut state =
            NetworkBoundSpot::new_unverified(network, anchor(network), spot(), 3).unwrap();
        let first = spot_block(network, state.tip(), 19, vec![spot_order(50, 1, true)]);
        state.apply_unverified_block(&first).unwrap();
        let before = state.clone();
        let second = spot_block(network, state.tip(), 20, vec![spot_order(51, 2, false)]);
        state.apply_unverified_block(&second).unwrap();
        assert_eq!(state.state().state().fills().len(), 1);
        assert_eq!(state.state().state().fills()[0].base_atoms, 100);
        assert_eq!(state.state().state().fills()[0].quote_atoms, 200);
        state.state().state().verify_reserves().unwrap();
        let bytes = state.checkpoint_bytes().unwrap();
        let loaded = NetworkBoundSpot::from_checkpoint_bytes(
            &bytes,
            network,
            anchor(network),
            3,
            &BTreeSet::from([id(7), id(8)]),
        )
        .unwrap();
        assert_eq!(state, loaded);
        for other in networks {
            if other != network {
                assert!(NetworkBoundSpot::from_checkpoint_bytes(
                    &bytes,
                    other,
                    anchor(other),
                    3,
                    &BTreeSet::from([id(7), id(8)])
                )
                .is_err());
            }
        }
        let mut restored = loaded;
        restored.rollback_tip(restored.tip()).unwrap();
        assert_eq!(restored, before);
        assert_eq!(
            restored
                .state()
                .state()
                .open_orders(SpotPair::new(id(7), id(8)).unwrap())
                .len(),
            1
        );
        let wrong = spot_block(
            networks.iter().copied().find(|x| *x != network).unwrap(),
            restored.tip(),
            22,
            vec![],
        );
        let saved = restored.clone();
        assert!(matches!(
            restored.apply_unverified_block(&wrong),
            Err(ProfileReplayError::Spot(_))
        ));
        assert_eq!(restored, saved);
    }
}

#[test]
fn profile_checkpoints_cannot_cross_unified_or_spot_lanes() {
    let net = TradeLayerMainnet::Litecoin;
    let unified =
        NetworkBoundUnified::new_unverified(net, anchor(net), account("admin"), 3).unwrap();
    let spot = NetworkBoundSpot::new_unverified(net, anchor(net), spot(), 3).unwrap();
    let ub = unified.checkpoint_bytes().unwrap();
    let sb = spot.checkpoint_bytes().unwrap();
    assert!(NetworkBoundSpot::from_checkpoint_bytes(
        &ub,
        net,
        anchor(net),
        3,
        &BTreeSet::from([id(7), id(8)])
    )
    .is_err());
    assert!(NetworkBoundUnified::from_checkpoint_bytes(
        &sb,
        net,
        anchor(net),
        &account("admin"),
        3
    )
    .is_err());
    assert_ne!(ub, sb);
}

#[test]
fn wrong_genesis_and_spoofed_high_checkpoint_metadata_fail_safe() {
    for network in [
        TradeLayerMainnet::Bitcoin,
        TradeLayerMainnet::Litecoin,
        TradeLayerMainnet::Dogecoin,
    ] {
        let wrong = Tip {
            height: 0,
            id: BlockId([0xb9; 32]),
        };
        assert!(matches!(
            NetworkBoundUnified::new_unverified(network, wrong, account("admin"), 3),
            Err(ProfileReplayError::Profile(
                NetworkProfileError::WrongGenesisAnchor
            ))
        ));
        assert!(matches!(
            NetworkBoundSpot::new_unverified(network, wrong, spot(), 3),
            Err(ProfileReplayError::Profile(
                NetworkProfileError::WrongGenesisAnchor
            ))
        ));
        // A nonzero anchor is allowed only as a *caller claim*, never
        // silently promoted to authenticated best-chain provenance.
        let claimed = Tip {
            height: 123456,
            id: BlockId([0xb9; 32]),
        };
        let state =
            NetworkBoundUnified::new_unverified(network, claimed, account("admin"), 3).unwrap();
        let bytes = state.checkpoint_bytes().unwrap();
        assert!(NetworkBoundUnified::from_checkpoint_bytes(
            &bytes,
            network,
            anchor(network),
            &account("admin"),
            3
        )
        .is_err());
        assert!(NetworkBoundUnified::from_checkpoint_bytes(
            &bytes,
            network,
            claimed,
            &account("mallory"),
            3
        )
        .is_err());
    }
}

#[test]
fn failed_immutable_checkpoint_write_leaves_state_unmodified() {
    let net = TradeLayerMainnet::Bitcoin;
    let mut state =
        NetworkBoundUnified::new_unverified(net, anchor(net), account("admin"), 3).unwrap();
    let original = state.clone();
    let path = std::env::temp_dir().join(format!(
        "tlr-stage21-collision-{}-unified",
        std::process::id()
    ));
    std::fs::write(&path, b"existing").unwrap();
    let b = block(net, state.tip(), 50, 1);
    assert!(state
        .apply_block_and_checkpoint(&b, &FakeProtocolAdapter { supply: 5 }, &path)
        .is_err());
    assert_eq!(state, original);
    assert_eq!(std::fs::read(&path).unwrap(), b"existing");
    std::fs::remove_file(path).unwrap();
}
