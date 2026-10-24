//! Stage25E2B research-only, all actors/prices/fills are synthetic.
//! No unauthenticated CRC, account or quote can authorize a live payout.
use std::collections::BTreeSet;
use tradelayer_core::chain::{BlockHeader, BlockId, Tip};
use tradelayer_core::clearing_chain::{
    ClearingChainState, ClearingChainError, UnverifiedClearingBlock,
    UnverifiedClearingEvent,
};
use tradelayer_core::clearing_math::PnlRounding;
use tradelayer_core::clearing_sandbox::{
    ClearingError, ClearingSandbox, CollateralPolicy, InsurancePolicy,
    MarkPolicy, MarkTerms, MatchedContractTrade,
};
use tradelayer_core::ledger::{
    AccountId, Ledger, Operation, PropertyId, PropertyKind, TxId,
};
use tradelayer_core::network_profile::TradeLayerMainnet;
use tradelayer_core::spot::SpotExchange;

const ONE: u64 = 100_000_000;
fn a(s: &str) -> AccountId {
    AccountId::try_from(s).unwrap()
}
fn p() -> PropertyId {
    PropertyId::try_from(7).unwrap()
}
fn tx(n: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}
fn insurer() -> InsurancePolicy {
    InsurancePolicy { sponsor: a("alice") }
}
fn terms() -> MarkTerms {
    MarkTerms {
        notional_e8: ONE,
        inverse: false,
        rounding: PnlRounding::ExactRational,
    }
}
fn policy() -> CollateralPolicy {
    CollateralPolicy {
        initial_atoms_per_contract: 10_000_000,
        maintenance_atoms_per_contract: 10_000_000,
    }
}
fn base() -> ClearingSandbox {
    let mut ledger = Ledger::new();
    ledger.apply_batch([
        (tx(1), Operation::CreateProperty {
            id: p(), issuer: a("alice"), kind: PropertyKind::Fixed,
            initial_atoms: 3_000_000_000,
        }),
        (tx(2), Operation::Transfer {
            id: p(), from: a("alice"), to: a("bob"),
            amount_atoms: 600_000_000,
        }),
        (tx(3), Operation::Transfer {
            id: p(), from: a("alice"), to: a("carol"),
            amount_atoms: 500_000_000,
        }),
        (tx(4), Operation::Transfer {
            id: p(), from: a("alice"), to: a("dave"),
            amount_atoms: 500_000_000,
        }),
    ]).unwrap();
    let spot = SpotExchange::from_ledger_unverified(ledger, BTreeSet::from([p()])).unwrap();
    ClearingSandbox::from_spot_unverified(
        TradeLayerMainnet::Litecoin, 19, p(), spot,
    ).unwrap()
    .with_collateral_policy_unverified(policy()).unwrap()
    .with_mark_policy_unverified(MarkPolicy {
        height: 100, price_e8: 100 * ONE,
        notional_e8: ONE, inverse: false,
        rounding: PnlRounding::ExactRational,
    }).unwrap()
    .with_insurance_policy_unverified(insurer()).unwrap()
}
fn opening() -> ClearingSandbox {
    let mut s = base();
    s.deposit_insurance_unverified(tx(10), a("alice"), 100_000_000).unwrap();
    s.deposit_margin_unverified(tx(11), a("alice"), 200_000_000).unwrap();
    s.deposit_margin_unverified(tx(12), a("bob"), 25_000_000).unwrap();
    s.apply_matched_trades_unverified(
        tx(13),
        &[MatchedContractTrade {
            long: a("alice"), short: a("bob"), contracts: 1,
        }],
    ).unwrap();
    s
}
fn restore_state(bytes: &[u8]) -> ClearingSandbox {
    ClearingSandbox::from_snapshot_bytes_with_insurance(
        bytes, TradeLayerMainnet::Litecoin, 19, p(),
        &BTreeSet::from([p()]), Some(policy()), Some(terms()),
        Some(&insurer()),
    ).unwrap()
}
fn anchor() -> Tip {
    Tip { height: 100, id: BlockId([1; 32]) }
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
fn restore_chain(bytes: &[u8]) -> ClearingChainState {
    ClearingChainState::from_checkpoint_bytes_with_insurance(
        bytes, TradeLayerMainnet::Litecoin, anchor(), 19, p(),
        &BTreeSet::from([p()]), Some(policy()), Some(terms()),
        Some(&insurer()), 3,
    ).unwrap()
}

#[test]
fn two_escrows_and_insurer_pay_actual_losing_margin_and_full_winner_pnl() {
    let mut s = opening();
    let before = s.clone();
    let total = s.ledger().property(p()).unwrap().total_supply_atoms;
    let preview = s.preview_distressed_closeout_unverified(101, 101 * ONE).unwrap();
    assert_eq!(preview.losing_margin_debited_atoms, 25_000_000);
    assert_eq!(preview.insurance_required_atoms, 75_000_000);
    assert_eq!(preview.insurance_available_atoms, 100_000_000);
    assert_eq!(preview.creditor_paid_atoms, 100_000_000);
    assert_eq!(preview.uncovered_insurance_atoms, 0);
    assert_eq!(preview.distressed_owner_count, 1);
    assert_eq!(s, before);
    let receipt = s.settle_distressed_closeout_unverified(tx(14), 101, 101 * ONE).unwrap();
    assert_eq!(receipt.ledger_transfer_legs, 2);
    assert_eq!(receipt.insurance_debited_atoms, 75_000_000);
    assert_eq!(receipt.losing_margin_debited_atoms, 25_000_000);
    assert_eq!(receipt.creditor_paid_atoms, 100_000_000);
    assert_eq!(receipt.closed_owner_count, 2);
    assert_eq!(s.margin_atoms(&a("bob")), 0);
    assert_eq!(s.margin_atoms(&a("alice")), 300_000_000);
    assert_eq!(s.insurance_reserve_atoms(), 25_000_000);
    assert!(s.positions().is_empty());
    assert_eq!(s.mark_policy().unwrap().height, 101);
    assert_eq!(s.mark_policy().unwrap().price_e8, 101 * ONE);
    assert_eq!(s.ledger().property(p()).unwrap().total_supply_atoms, total);
    assert_eq!(s.ledger().verify_conservation(), Ok(()));
    assert_eq!(s.verify(), Ok(()));
    assert!(s.spot().snapshot_bytes().is_err());
    assert_eq!(s.snapshot_bytes().unwrap(), restore_state(&s.snapshot_bytes().unwrap()).snapshot_bytes().unwrap());
}

#[test]
fn insufficient_insurance_does_not_credit_any_winner_or_seize_margin() {
    let mut s = opening();
    let original = s.clone();
    let preview = s.preview_distressed_closeout_unverified(101, 102 * ONE).unwrap();
    assert_eq!(preview.insurance_required_atoms, 175_000_000);
    assert_eq!(preview.insurance_available_atoms, 100_000_000);
    assert_eq!(preview.uncovered_insurance_atoms, 75_000_000);
    assert_eq!(
        s.settle_distressed_closeout_unverified(tx(14), 101, 102 * ONE),
        Err(ClearingError::InsufficientInsurance {
            required_atoms: 175_000_000,
            available_atoms: 100_000_000,
        })
    );
    assert_eq!(s, original);
    assert_eq!(s.processed_count(), original.processed_count());
    assert_eq!(s.insurance_reserve_atoms(), 100_000_000);
}

#[test]
fn full_book_closeout_requires_actual_maintenance_breach_and_policy() {
    let mut s = base();
    s.deposit_insurance_unverified(tx(10), a("alice"), ONE).unwrap();
    s.deposit_margin_unverified(tx(11), a("alice"), 200_000_000).unwrap();
    s.deposit_margin_unverified(tx(12), a("bob"), 200_000_000).unwrap();
    s.apply_matched_trades_unverified(tx(13), &[
        MatchedContractTrade { long: a("alice"), short: a("bob"), contracts: 1 }
    ]).unwrap();
    let baseline = s.clone();
    assert_eq!(
        s.settle_distressed_closeout_unverified(tx(14), 101, 101 * ONE),
        Err(ClearingError::NoDistress)
    );
    assert_eq!(s, baseline);
    let mut without_insurer = ClearingSandbox::from_ledger_unverified(
        TradeLayerMainnet::Litecoin, 19, p(), {
            let mut l = Ledger::new();
            l.apply(tx(1), Operation::CreateProperty {
                id: p(), issuer: a("alice"), kind: PropertyKind::Fixed,
                initial_atoms: 1_000_000,
            }).unwrap();
            l
        },
    ).unwrap();
    assert_eq!(
        without_insurer.settle_distressed_closeout_unverified(tx(2), 101, 101 * ONE),
        Err(ClearingError::InsurancePolicyMissing)
    );
}

#[test]
fn solvent_but_below_maintenance_full_closeout_needs_no_insurer_atoms() {
    let mut s = base();
    s.deposit_margin_unverified(tx(11), a("alice"), 200_000_000).unwrap();
    s.deposit_margin_unverified(tx(12), a("bob"), 25_000_000).unwrap();
    s.apply_matched_trades_unverified(tx(13), &[
        MatchedContractTrade { long: a("alice"), short: a("bob"), contracts: 1 }
    ]).unwrap();
    // Bob loses 20M: equity 5M < maintenance 10M but not insolvent.
    let preview = s.preview_distressed_closeout_unverified(101, 100 * ONE + ONE / 5).unwrap();
    assert_eq!(preview.insurance_required_atoms, 0);
    assert_eq!(preview.distressed_owner_count, 1);
    let receipt = s.settle_distressed_closeout_unverified(
        tx(14), 101, 100 * ONE + ONE / 5,
    ).unwrap();
    assert_eq!(receipt.insurance_debited_atoms, 0);
    assert_eq!(receipt.creditor_paid_atoms, 20_000_000);
    assert_eq!(s.margin_atoms(&a("bob")), 5_000_000);
    assert!(s.positions().is_empty());
    assert_eq!(s.insurance_reserve_atoms(), 0);
}

#[test]
fn fault_injection_aborts_before_every_leg_and_after_the_last() {
    let initial = opening();
    for boundary in 0..=2 {
        let mut s = initial.clone();
        let original_bytes = s.snapshot_bytes().unwrap();
        assert_eq!(
            s.settle_distressed_closeout_fault_injected_unverified(
                tx(14), 101, 101 * ONE, boundary,
            ),
            Err(ClearingError::InjectedFailure { before_leg: boundary })
        );
        assert_eq!(s, initial);
        assert_eq!(s.snapshot_bytes().unwrap(), original_bytes);
        // The aborted external ID must be free for one genuine later commit.
        s.settle_distressed_closeout_unverified(tx(14), 101, 101 * ONE).unwrap();
        assert_eq!(s.insurance_reserve_atoms(), 25_000_000);
    }
}

#[test]
fn shared_insurer_cannot_pay_two_shortfalls_from_one_reserve() {
    let mut s = opening();
    s.settle_distressed_closeout_unverified(tx(14), 101, 101 * ONE).unwrap();
    assert_eq!(s.insurance_reserve_atoms(), 25_000_000);
    s.deposit_margin_unverified(tx(15), a("bob"), 25_000_000).unwrap();
    s.apply_matched_trades_unverified(tx(16), &[
        MatchedContractTrade { long: a("alice"), short: a("bob"), contracts: 1 }
    ]).unwrap();
    let before = s.clone();
    assert_eq!(
        s.settle_distressed_closeout_unverified(tx(17), 102, 102 * ONE),
        Err(ClearingError::InsufficientInsurance {
            required_atoms: 75_000_000,
            available_atoms: 25_000_000,
        })
    );
    assert_eq!(s, before);
    assert_eq!(s.insurance_reserve_atoms(), 25_000_000);
}

#[test]
fn multi_party_settlement_uses_insurer_last_and_never_pays_twice() {
    let mut s = base();
    s.deposit_insurance_unverified(tx(10), a("alice"), 250_000_000).unwrap();
    for (id, name, atoms) in [
        (11, "alice", 100_000_000),
        (12, "bob", 25_000_000),
        (13, "carol", 50_000_000),
        (14, "dave", 50_000_000),
    ] {
        s.deposit_margin_unverified(tx(id), a(name), atoms).unwrap();
    }
    s.apply_matched_trades_unverified(tx(15), &[
        MatchedContractTrade { long: a("alice"), short: a("bob"), contracts: 1 },
        MatchedContractTrade { long: a("alice"), short: a("carol"), contracts: 1 },
        MatchedContractTrade { long: a("dave"), short: a("carol"), contracts: 1 },
    ]).unwrap();
    let first = s.clone();
    let preview = s.preview_distressed_closeout_unverified(101, 101 * ONE).unwrap();
    assert_eq!(preview.losing_margin_debited_atoms, 75_000_000);
    assert_eq!(preview.insurance_required_atoms, 225_000_000);
    assert_eq!(preview.creditor_paid_atoms, 300_000_000);
    let receipt = s.settle_distressed_closeout_unverified(tx(16), 101, 101 * ONE).unwrap();
    assert_eq!(receipt.ledger_transfer_legs, 4);
    assert_eq!(s.margin_atoms(&a("alice")), 300_000_000);
    assert_eq!(s.margin_atoms(&a("dave")), 150_000_000);
    assert_eq!(s.margin_atoms(&a("bob")), 0);
    assert_eq!(s.margin_atoms(&a("carol")), 0);
    assert_eq!(s.insurance_reserve_atoms(), 25_000_000);
    assert!(s.positions().is_empty());
    assert_eq!(s.ledger().property(p()).unwrap().total_supply_atoms,
        first.ledger().property(p()).unwrap().total_supply_atoms);
    assert_eq!(
        s.settle_distressed_closeout_unverified(tx(16), 102, 102 * ONE),
        Err(ClearingError::Replay(tx(16)))
    );
    assert_eq!(s.verify(), Ok(()));
}

#[test]
fn replay_and_stale_mark_guard_closeout_from_spoofed_retries() {
    let mut s = opening();
    assert_eq!(
        s.settle_distressed_closeout_unverified(tx(13), 101, 101 * ONE),
        Err(ClearingError::Replay(tx(13)))
    );
    assert_eq!(
        s.settle_distressed_closeout_unverified(tx(14), 100, 101 * ONE),
        Err(ClearingError::StaleMark)
    );
    s.settle_distressed_closeout_unverified(tx(14), 101, 101 * ONE).unwrap();
    let done = s.clone();
    assert_eq!(
        s.settle_distressed_closeout_unverified(tx(14), 102, 102 * ONE),
        Err(ClearingError::Replay(tx(14)))
    );
    assert_eq!(s, done);
}

#[test]
fn failed_insured_settlement_recovers_from_tlcl_v2_without_new_credits() {
    let s = opening();
    let before = s.snapshot_bytes().unwrap();
    let mut restored = restore_state(&before);
    assert_eq!(restored, s);
    assert!(restored.settle_distressed_closeout_fault_injected_unverified(
        tx(14), 101, 101 * ONE, 1,
    ).is_err());
    assert_eq!(restored.snapshot_bytes().unwrap(), before);
    restored.settle_distressed_closeout_unverified(tx(14), 101, 101 * ONE).unwrap();
    let after = restored.snapshot_bytes().unwrap();
    assert_eq!(restore_state(&after), restored);
    assert_eq!(restored.insurance_reserve_atoms(), 25_000_000);
    assert_eq!(restored.processed_count(), s.processed_count() + 1);
    assert!(restored.spot().snapshot_bytes().is_err());
}

#[test]
fn synthetic_block_final_closeout_and_reorg_restore_full_insurer_balance() {
    let mut journal = ClearingChainState::new_unverified(
        TradeLayerMainnet::Litecoin, anchor(), base(), 3,
    ).unwrap();
    let first = block(101, 2, 1, vec![
        UnverifiedClearingEvent::InsuranceDeposit {
            txid: tx(10), sponsor: a("alice"), atoms: 100_000_000,
        },
        UnverifiedClearingEvent::MarginDeposit {
            txid: tx(11), owner: a("alice"), atoms: 200_000_000,
        },
        UnverifiedClearingEvent::MarginDeposit {
            txid: tx(12), owner: a("bob"), atoms: 25_000_000,
        },
        UnverifiedClearingEvent::MatchedTrades {
            txid: tx(13),
            trades: vec![MatchedContractTrade {
                long: a("alice"), short: a("bob"), contracts: 1,
            }],
        },
        UnverifiedClearingEvent::DistressedCloseout {
            txid: tx(14), new_price_e8: 101 * ONE,
        },
    ]);
    let prior = journal.clone();
    journal.apply_unverified_block(&first).unwrap();
    assert_eq!(journal.state().insurance_reserve_atoms(), 25_000_000);
    assert!(journal.state().positions().is_empty());
    assert_eq!(journal.state().spot().processed_through(), Some(101));
    let bytes = journal.checkpoint_bytes().unwrap();
    assert_eq!(&bytes[..5], b"TLCC\x02");
    let loaded = restore_chain(&bytes);
    assert_eq!(loaded, journal);
    let mut rolled = loaded.clone();
    rolled.rollback_tip(Tip { height: 101, id: BlockId([2;32]) }).unwrap();
    assert_eq!(rolled, prior);
    let alternative = block(101, 55, 1, vec![
        UnverifiedClearingEvent::InsuranceDeposit {
            txid: tx(10), sponsor: a("alice"), atoms: 30_000_000,
        },
    ]);
    rolled.apply_unverified_block(&alternative).unwrap();
    assert_eq!(rolled.state().insurance_reserve_atoms(), 30_000_000);
    assert_eq!(rolled.state().ledger().verify_conservation(), Ok(()));
}

#[test]
fn synthetic_block_rejects_late_claim_without_committing_previous_deposits() {
    let mut journal = ClearingChainState::new_unverified(
        TradeLayerMainnet::Litecoin, anchor(), base(), 3,
    ).unwrap();
    let prior = journal.clone();
    let insufficient = block(101, 2, 1, vec![
        UnverifiedClearingEvent::InsuranceDeposit {
            txid: tx(10), sponsor: a("alice"), atoms: 10_000_000,
        },
        UnverifiedClearingEvent::MarginDeposit {
            txid: tx(11), owner: a("alice"), atoms: 200_000_000,
        },
        UnverifiedClearingEvent::MarginDeposit {
            txid: tx(12), owner: a("bob"), atoms: 25_000_000,
        },
        UnverifiedClearingEvent::MatchedTrades {
            txid: tx(13),
            trades: vec![MatchedContractTrade {
                long: a("alice"), short: a("bob"), contracts: 1,
            }],
        },
        UnverifiedClearingEvent::DistressedCloseout {
            txid: tx(14), new_price_e8: 101 * ONE,
        },
    ]);
    assert!(matches!(
        journal.apply_unverified_block(&insufficient),
        Err(ClearingChainError::Clearing(ClearingError::InsufficientInsurance {..}))
    ));
    assert_eq!(journal, prior);
    let malformed = block(101, 2, 1, vec![
        UnverifiedClearingEvent::DistressedCloseout {
            txid: tx(14), new_price_e8: 101 * ONE,
        },
        UnverifiedClearingEvent::Ordinary {
            txid: tx(15),
            operation: Operation::Transfer {
                id: p(), from: a("alice"), to: a("bob"), amount_atoms: 1,
            },
        },
    ]);
    assert!(matches!(
        journal.apply_unverified_block(&malformed),
        Err(ClearingChainError::MarkMustEndBlock)
    ));
    assert_eq!(journal, prior);
}

#[test]
fn generated_pnl_rounding_imbalance_remains_default_deny_even_with_insurance() {
    let mut s = base();
    // Three-party half-atom PnL mismatch should NOT be paid by the insurer.
    s.deposit_insurance_unverified(tx(10), a("alice"), ONE).unwrap();
    s.deposit_margin_unverified(tx(11), a("alice"), 100_000_000).unwrap();
    s.deposit_margin_unverified(tx(12), a("bob"), 100_000_000).unwrap();
    s.deposit_margin_unverified(tx(13), a("carol"), 100_000_000).unwrap();
    s.apply_matched_trades_unverified(tx(14), &[
        MatchedContractTrade { long: a("alice"), short: a("bob"), contracts: 1 },
        MatchedContractTrade { long: a("carol"), short: a("bob"), contracts: 1 },
    ]).unwrap();
    // A full mark difference of one atom yields PnL ±1/±2, perfectly
    // balanced under the 1-unit notional. A different notional policy must
    // be chosen at construction for actual residual testing.
    assert_eq!(s.preview_mark_to_market_unverified(101, 100 * ONE + 1).unwrap().net_atoms, 0);
    assert_eq!(
        s.settle_distressed_closeout_unverified(tx(15), 101, 100 * ONE + 1),
        Err(ClearingError::NoDistress)
    );
}
