//! Stage25E2: real insurer atoms, no payout or liquidation permission.
//! All participants, sponsors, headers and funding are synthetic assertions.
use std::collections::BTreeSet;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tradelayer_core::chain::{BlockHeader, BlockId, Tip};
use tradelayer_core::clearing_chain::{
    ClearingChainState, UnverifiedClearingBlock, UnverifiedClearingEvent,
};
use tradelayer_core::clearing_math::PnlRounding;
use tradelayer_core::clearing_sandbox::{
    ClearingError, ClearingSandbox, CollateralPolicy, InsurancePolicy, MarkPolicy, MarkTerms,
    MatchedContractTrade,
};
use tradelayer_core::ledger::{AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::spot::{SpotError, SpotExchange, SpotIntent, UnverifiedSpotPlacement};

const ONE: u64 = 100_000_000;
fn a(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn p(n: u32) -> PropertyId {
    PropertyId::try_from(n).unwrap()
}
fn tx(n: u64) -> TxId {
    let mut bytes = [0; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn insurer() -> InsurancePolicy {
    InsurancePolicy {
        sponsor: a("alice"),
    }
}
fn collateral() -> CollateralPolicy {
    CollateralPolicy {
        initial_atoms_per_contract: 10_000_000,
        maintenance_atoms_per_contract: 5_000_000,
    }
}
fn mark() -> MarkPolicy {
    MarkPolicy {
        height: 100,
        price_e8: 100 * ONE,
        notional_e8: ONE,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn terms() -> MarkTerms {
    MarkTerms {
        notional_e8: ONE,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn permitted() -> BTreeSet<PropertyId> {
    BTreeSet::from([p(7), p(8)])
}
fn baseline() -> ClearingSandbox {
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
                    amount_atoms: 300_000_000,
                },
            ),
        ])
        .unwrap();
    let spot = SpotExchange::from_ledger_unverified(ledger, permitted()).unwrap();
    ClearingSandbox::from_spot_unverified(TradeLayerMainnet::Litecoin, 19, p(7), spot)
        .unwrap()
        .with_collateral_policy_unverified(collateral())
        .unwrap()
        .with_mark_policy_unverified(mark())
        .unwrap()
}
fn insured() -> ClearingSandbox {
    baseline()
        .with_insurance_policy_unverified(insurer())
        .unwrap()
}
fn restored(
    bytes: &[u8],
    sponsor: Option<&InsurancePolicy>,
) -> Result<ClearingSandbox, tradelayer_core::clearing_sandbox::ClearingSnapshotError> {
    ClearingSandbox::from_snapshot_bytes_with_insurance(
        bytes,
        TradeLayerMainnet::Litecoin,
        19,
        p(7),
        &permitted(),
        Some(collateral()),
        Some(terms()),
        sponsor,
    )
}
fn chain_anchor() -> Tip {
    Tip {
        height: 100,
        id: BlockId([1; 32]),
    }
}
fn block(
    height: u64,
    id: u8,
    parent: u8,
    events: Vec<UnverifiedClearingEvent>,
) -> UnverifiedClearingBlock {
    UnverifiedClearingBlock {
        network: TradeLayerMainnet::Litecoin,
        header: BlockHeader {
            height,
            id: BlockId([id; 32]),
            parent: BlockId([parent; 32]),
        },
        events,
    }
}
fn restore_chain(
    bytes: &[u8],
    sponsor: Option<&InsurancePolicy>,
) -> Result<ClearingChainState, tradelayer_core::clearing_chain::ClearingChainCheckpointError> {
    ClearingChainState::from_checkpoint_bytes_with_insurance(
        bytes,
        TradeLayerMainnet::Litecoin,
        chain_anchor(),
        19,
        p(7),
        &permitted(),
        Some(collateral()),
        Some(terms()),
        sponsor,
        3,
    )
}
fn repair_crc(bytes: &mut [u8]) {
    let end = bytes.len() - 4;
    let mut crc = !0u32;
    for byte in &bytes[..end] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    bytes[end..].copy_from_slice(&(!crc).to_be_bytes());
}
fn offer(id: u64, actor: &str, height: u64) -> UnverifiedSpotPlacement {
    UnverifiedSpotPlacement {
        txid: tx(id),
        actor: a(actor),
        height,
        intent: SpotIntent {
            offered: p(7),
            desired: p(8),
            offered_atoms: ONE,
            expected_atoms: ONE,
            post: false,
            stop: false,
        },
    }
}

#[test]
fn sponsor_donates_existing_atoms_into_unique_insurance_reserve() {
    let mut s = insured();
    let escrow = s.insurance_escrow_for_audit().unwrap();
    assert!(escrow.as_str().starts_with("~ins:"));
    let starting = s.ledger().available_atoms(p(7), &a("alice"));
    let baseline = s.ledger().property(p(7)).unwrap().total_supply_atoms;
    assert_eq!(s.insurance_reserve_atoms(), 0);
    s.deposit_insurance_unverified(tx(10), a("alice"), 100_000_000)
        .unwrap();
    assert_eq!(s.insurance_reserve_atoms(), 100_000_000);
    assert_eq!(s.ledger().available_atoms(p(7), &escrow), 100_000_000);
    assert_eq!(
        s.ledger().available_atoms(p(7), &a("alice")),
        starting - 100_000_000
    );
    assert_eq!(
        s.ledger().property(p(7)).unwrap().total_supply_atoms,
        baseline
    );
    assert_eq!(s.ledger().verify_conservation(), Ok(()));
    assert_eq!(s.processed_count(), 1);
    assert_eq!(s.verify(), Ok(()));
    assert!(std::ptr::eq(s.ledger(), s.spot().ledger()));
}

#[test]
fn unrelated_actor_replay_and_overdraft_are_atomic() {
    let mut s = insured();
    let initial = s.clone();
    assert_eq!(
        s.deposit_insurance_unverified(tx(10), a("bob"), 100),
        Err(ClearingError::UnauthorizedInsuranceSponsor)
    );
    assert_eq!(
        s.deposit_insurance_unverified(tx(10), a("alice"), 0),
        Err(ClearingError::ZeroAmount)
    );
    assert!(s
        .deposit_insurance_unverified(tx(10), a("alice"), u128::MAX)
        .is_err());
    assert_eq!(s, initial);
    s.deposit_insurance_unverified(tx(10), a("alice"), 100)
        .unwrap();
    let funded = s.clone();
    assert_eq!(
        s.deposit_insurance_unverified(tx(10), a("alice"), 1),
        Err(ClearingError::Replay(tx(10)))
    );
    assert_eq!(s, funded);
}

#[test]
fn no_spot_order_or_ordinary_transfer_can_spend_insurance() {
    let mut s = insured();
    s.deposit_insurance_unverified(tx(10), a("alice"), 50_000_000)
        .unwrap();
    let escrow = s.insurance_escrow_for_audit().unwrap();
    let before = s.clone();
    assert_eq!(
        s.apply_ordinary_operation_unverified(
            tx(11),
            Operation::Transfer {
                id: p(7),
                from: escrow.clone(),
                to: a("bob"),
                amount_atoms: 1,
            },
        ),
        Err(ClearingError::Spot(SpotError::ReservedAccount))
    );
    assert_eq!(
        s.enqueue_spot_order_unverified(offer(12, escrow.as_str(), 101)),
        Err(ClearingError::Spot(SpotError::ReservedAccount))
    );
    assert_eq!(s, before);
    assert!(s.spot().snapshot_bytes().is_err());
}

#[test]
fn explicit_insurance_policy_is_frozen_before_first_deposit() {
    let s = insured();
    assert_eq!(s.insurance_reserve_atoms(), 0);
    assert!(s.spot().snapshot_bytes().is_err()); // Even policy-only history is fenced.
    assert_eq!(
        s.clone().with_insurance_policy_unverified(insurer()),
        Err(ClearingError::InsurancePolicyFrozen)
    );
    assert_eq!(
        baseline().with_insurance_policy_unverified(InsurancePolicy {
            sponsor: a("~ins:forged")
        }),
        Err(ClearingError::InvalidInsuranceSponsor)
    );
    assert_eq!(
        baseline().deposit_insurance_unverified(tx(10), a("alice"), 10),
        Err(ClearingError::InsurancePolicyMissing)
    );
    assert_eq!(
        s.clone().with_mark_policy_unverified(mark()),
        Err(ClearingError::MarkPolicyFrozen)
    );
}

#[test]
fn insurer_identity_is_separated_by_network_and_contract() {
    let a = insured().insurance_escrow_for_audit().unwrap();
    let other_network =
        ClearingSandbox::from_ledger_unverified(TradeLayerMainnet::Dogecoin, 19, p(7), {
            let mut ledger = Ledger::new();
            ledger
                .apply(
                    tx(1),
                    Operation::CreateProperty {
                        id: p(7),
                        issuer: a_account("alice"),
                        kind: PropertyKind::Fixed,
                        initial_atoms: 100,
                    },
                )
                .unwrap();
            ledger
        })
        .unwrap()
        .with_insurance_policy_unverified(insurer())
        .unwrap();
    let b = other_network.insurance_escrow_for_audit().unwrap();
    let alternate_contract =
        ClearingSandbox::from_spot_unverified(TradeLayerMainnet::Litecoin, 20, p(7), {
            let mut ledger = Ledger::new();
            ledger
                .apply(
                    tx(1),
                    Operation::CreateProperty {
                        id: p(7),
                        issuer: a_account("alice"),
                        kind: PropertyKind::Fixed,
                        initial_atoms: 100,
                    },
                )
                .unwrap();
            SpotExchange::from_ledger_unverified(ledger, BTreeSet::new()).unwrap()
        })
        .unwrap()
        .with_insurance_policy_unverified(insurer())
        .unwrap();
    let c = alternate_contract.insurance_escrow_for_audit().unwrap();
    assert_ne!(a, b);
    assert_ne!(a, c);
    assert_ne!(b, c);
}
fn a_account(name: &str) -> AccountId {
    a(name)
}

#[test]
fn tlcl_v2_is_canonical_and_legacy_v1_remains_byte_stable() {
    let old = baseline();
    let old_bytes = old.snapshot_bytes().unwrap();
    assert_eq!(&old_bytes[..5], b"TLCL\x01");
    assert_eq!(restored(&old_bytes, None).unwrap(), old);
    assert_eq!(old.snapshot_bytes().unwrap(), old_bytes);
    assert!(restored(&old_bytes, Some(&insurer())).is_err());

    let mut s = insured();
    assert_eq!(&s.snapshot_bytes().unwrap()[..5], b"TLCL\x02");
    s.deposit_insurance_unverified(tx(10), a("alice"), 100_000_000)
        .unwrap();
    let bytes = s.snapshot_bytes().unwrap();
    let loaded = restored(&bytes, Some(&insurer())).unwrap();
    assert_eq!(loaded, s);
    assert_eq!(loaded.snapshot_bytes().unwrap(), bytes);
    assert_eq!(loaded.insurance_reserve_atoms(), 100_000_000);
    assert!(restored(&bytes, None).is_err());
    assert!(ClearingSandbox::from_snapshot_bytes(
        &bytes,
        TradeLayerMainnet::Litecoin,
        19,
        p(7),
        &permitted(),
        Some(collateral()),
        Some(terms())
    )
    .is_err());
    let changed = InsurancePolicy { sponsor: a("bob") };
    assert!(restored(&bytes, Some(&changed)).is_err());
    assert!(ClearingSandbox::from_snapshot_bytes_with_insurance(
        &bytes,
        TradeLayerMainnet::Bitcoin,
        19,
        p(7),
        &permitted(),
        Some(collateral()),
        Some(terms()),
        Some(&insurer())
    )
    .is_err());
}

#[test]
fn recomputed_crc_cannot_change_insurance_sponsor_or_downgrade_v2() {
    let mut s = insured();
    s.deposit_insurance_unverified(tx(10), a("alice"), 100)
        .unwrap();
    let bytes = s.snapshot_bytes().unwrap();
    let mut changed = bytes.clone();
    changed[4] = 1;
    repair_crc(&mut changed);
    assert!(restored(&changed, Some(&insurer())).is_err());
    assert!(restored(&changed, None).is_err());
    let mut changed = bytes.clone();
    // An independently supplied sponsor is non-negotiable, even with a
    // recomputed CRC, a valid UTF-8 account and conserved ledger atoms.
    let sponsor_offset = 5 + 1 + 32 + 32 + 4 + 4 + 1 + 16 + 16 + 1 + 8 + 8 + 8 + 1 + 1;
    assert_eq!(changed[sponsor_offset], 5);
    changed[sponsor_offset + 1] = b'b';
    repair_crc(&mut changed);
    assert!(restored(&changed, Some(&insurer())).is_err());
    let mut changed = bytes.clone();
    changed.push(9);
    assert!(restored(&changed, Some(&insurer())).is_err());
}

#[test]
fn tlcc_v2_recovers_donations_and_rolls_back_alternate_forks() {
    let state = insured();
    let mut chain =
        ClearingChainState::new_unverified(TradeLayerMainnet::Litecoin, chain_anchor(), state, 3)
            .unwrap();
    let first = block(
        101,
        2,
        1,
        vec![
            UnverifiedClearingEvent::InsuranceDeposit {
                txid: tx(10),
                sponsor: a("alice"),
                atoms: 100_000_000,
            },
            UnverifiedClearingEvent::MarginDeposit {
                txid: tx(11),
                owner: a("alice"),
                atoms: 50_000_000,
            },
            UnverifiedClearingEvent::MarginDeposit {
                txid: tx(12),
                owner: a("bob"),
                atoms: 50_000_000,
            },
            UnverifiedClearingEvent::MatchedTrades {
                txid: tx(13),
                trades: vec![MatchedContractTrade {
                    long: a("alice"),
                    short: a("bob"),
                    contracts: 1,
                }],
            },
            UnverifiedClearingEvent::Mark {
                txid: tx(14),
                new_price_e8: 100 * ONE + 1,
            },
        ],
    );
    chain.apply_unverified_block(&first).unwrap();
    assert_eq!(chain.state().insurance_reserve_atoms(), 100_000_000);
    let after_first = chain.clone();
    chain
        .apply_unverified_block(&block(
            102,
            3,
            2,
            vec![UnverifiedClearingEvent::InsuranceDeposit {
                txid: tx(15),
                sponsor: a("alice"),
                atoms: 75_000_000,
            }],
        ))
        .unwrap();
    assert_eq!(chain.state().insurance_reserve_atoms(), 175_000_000);
    let bytes = chain.checkpoint_bytes().unwrap();
    assert_eq!(&bytes[..5], b"TLCC\x02");
    let mut restored = restore_chain(&bytes, Some(&insurer())).unwrap();
    assert_eq!(restored, chain);
    assert_eq!(restored.checkpoint_bytes().unwrap(), bytes);
    assert!(restore_chain(&bytes, None).is_err());
    assert!(ClearingChainState::from_checkpoint_bytes(
        &bytes,
        TradeLayerMainnet::Litecoin,
        chain_anchor(),
        19,
        p(7),
        &permitted(),
        Some(collateral()),
        Some(terms()),
        3
    )
    .is_err());
    let wrong = InsurancePolicy { sponsor: a("bob") };
    assert!(restore_chain(&bytes, Some(&wrong)).is_err());

    restored
        .rollback_tip(Tip {
            height: 102,
            id: BlockId([3; 32]),
        })
        .unwrap();
    assert_eq!(restored, after_first);
    assert_eq!(restored.state().insurance_reserve_atoms(), 100_000_000);
    restored
        .apply_unverified_block(&block(
            102,
            55,
            2,
            vec![UnverifiedClearingEvent::InsuranceDeposit {
                txid: tx(15),
                sponsor: a("alice"),
                atoms: 25_000_000,
            }],
        ))
        .unwrap();
    assert_eq!(restored.state().insurance_reserve_atoms(), 125_000_000);
    assert_eq!(restored.state().ledger().verify_conservation(), Ok(()));
}

#[test]
fn immutable_v2_checkpoint_failure_preserves_all_reserve_and_replay_history() {
    let mut chain = ClearingChainState::new_unverified(
        TradeLayerMainnet::Litecoin,
        chain_anchor(),
        insured(),
        3,
    )
    .unwrap();
    let before = chain.clone();
    let dir = std::env::temp_dir().join(format!(
        "tradelayer-insurance-v2-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&dir).unwrap();
    let occupied = dir.join("occupied.tlcc");
    fs::write(&occupied, b"no overwrite").unwrap();
    let first = block(
        101,
        2,
        1,
        vec![UnverifiedClearingEvent::InsuranceDeposit {
            txid: tx(10),
            sponsor: a("alice"),
            atoms: 100_000_000,
        }],
    );
    assert!(chain
        .apply_unverified_block_and_checkpoint(&first, &occupied)
        .is_err());
    assert_eq!(chain, before);
    assert_eq!(fs::read(&occupied).unwrap(), b"no overwrite");
    let missing = dir.join("missing").join("cannot.tlcc");
    assert!(chain
        .apply_unverified_block_and_checkpoint(&first, &missing)
        .is_err());
    assert_eq!(chain, before);
    let path = dir.join("good.tlcc");
    chain
        .apply_unverified_block_and_checkpoint(&first, &path)
        .unwrap();
    let from_disk = ClearingChainState::load_checkpoint_with_insurance(
        &path,
        TradeLayerMainnet::Litecoin,
        chain_anchor(),
        19,
        p(7),
        &permitted(),
        Some(collateral()),
        Some(terms()),
        Some(&insurer()),
        3,
    )
    .unwrap();
    assert_eq!(from_disk, chain);
    assert_eq!(from_disk.state().insurance_reserve_atoms(), 100_000_000);
    fs::remove_dir_all(dir).unwrap();
}
