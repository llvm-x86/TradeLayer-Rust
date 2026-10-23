//! Independent, dependency-free vector oracle for accounting map refinement.
//! No BTreeMap or production canonical_columns helper is used in this model.
//! The real Ledger is checked after every admitted or rejected transaction.
//! Inputs remain synthetic; this does not authenticate any blockchain data.

use tradelayer_core::ledger::{
    AccountId, ChannelSide, Ledger, Operation, PropertyId, PropertyKind, SupplyAuditRow,
    SupplyColumnProofKey, TxId,
};
use tradelayer_core::network_profile::TradeLayerMainnet;

fn property(id: u32) -> PropertyId {
    PropertyId::try_from(id).unwrap()
}

fn account(name: &str) -> AccountId {
    AccountId::try_from(name).unwrap()
}

fn txid(n: u64) -> TxId {
    let mut bytes = [0u8; 32];
    bytes[24..].copy_from_slice(&n.to_be_bytes());
    TxId::from_bytes(bytes)
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Send { property: u32, from: &'static str, to: &'static str, amount: u128 },
    Mint { property: u32, actor: &'static str, to: &'static str, amount: u128 },
    Burn { property: u32, actor: &'static str, amount: u128 },
    Commit {
        property: u32, owner: &'static str, channel: &'static str,
        side: ChannelSide, amount: u128,
    },
    Withdraw {
        property: u32, owner: &'static str, channel: &'static str,
        side: ChannelSide, amount: u128, unlock: u64,
    },
    ChannelMove {
        property: u32, source: &'static str, side: ChannelSide,
        destination: &'static str, receiving: ChannelSide, amount: u128,
    },
}

impl Action {
    fn into_operation(self) -> Operation {
        match self {
            Self::Send { property: p, from, to, amount } => Operation::Transfer {
                id: property(p), from: account(from), to: account(to), amount_atoms: amount,
            },
            Self::Mint { property: p, actor, to, amount } => Operation::Mint {
                id: property(p), issuer: account(actor), to: account(to), amount_atoms: amount,
            },
            Self::Burn { property: p, actor, amount } => Operation::Burn {
                id: property(p), issuer: account(actor), amount_atoms: amount,
            },
            Self::Commit { property: p, owner, channel, side, amount } =>
                Operation::ChannelCommit {
                    id: property(p), owner: account(owner), channel: account(channel),
                    side, amount_atoms: amount,
                },
            Self::Withdraw { property: p, owner, channel, side, amount, unlock } =>
                Operation::ChannelWithdraw {
                    id: property(p), owner: account(owner), channel: account(channel),
                    side, amount_atoms: amount, unlock_height: unlock,
                },
            Self::ChannelMove { property: p, source, side, destination, receiving, amount } =>
                Operation::ChannelTransfer {
                    id: property(p), from_channel: account(source), from_side: side,
                    to_channel: account(destination), to_side: receiving,
                    amount_atoms: amount,
                },
        }
    }
}

/// Unlike the live ordered maps, an entry is just a linear-search vector slot.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Slot {
    Available(&'static str),
    Escrow(&'static str, ChannelSide),
    Reserved(u64),
}

#[derive(Clone)]
struct Withdrawal {
    id: u64,
    property: u32,
    owner: &'static str,
    amount: u128,
    unlock: u64,
}

#[derive(Clone)]
struct RefLedger {
    // (property ID, managed, issued)
    properties: Vec<(u32, bool, u128)>,
    columns: Vec<(u32, Slot, u128)>,
    // (channel name, side, beneficiary)
    owners: Vec<(&'static str, ChannelSide, &'static str)>,
    withdrawals: Vec<Withdrawal>,
    used: Vec<u64>,
}

impl RefLedger {
    fn new() -> Self {
        Self {
            properties: vec![(41, false, 1000), (42, true, 1000)],
            columns: vec![
                (41, Slot::Available("alice"), 1000),
                (42, Slot::Available("alice"), 1000),
            ],
            owners: vec![],
            withdrawals: vec![],
            used: vec![1, 2],
        }
    }

    fn atoms(&self, id: u32, slot: &Slot) -> u128 {
        self.columns
            .iter()
            .find(|(p, at, _)| *p == id && at == slot)
            .map_or(0, |(_, _, n)| *n)
    }

    fn put(&mut self, id: u32, slot: Slot, amount: u128) {
        self.columns.retain(|(p, old, _)| *p != id || *old != slot);
        if amount != 0 {
            self.columns.push((id, slot, amount));
        }
    }

    fn owner(&self, channel: &str, side: ChannelSide) -> Option<&'static str> {
        self.owners
            .iter()
            .find(|(c, s, _)| *c == channel && *s == side)
            .map(|(_, _, owner)| *owner)
    }

    fn opposite(side: ChannelSide) -> ChannelSide {
        match side { ChannelSide::A => ChannelSide::B, ChannelSide::B => ChannelSide::A }
    }

    fn can_claim(&self, channel: &str, side: ChannelSide, owner: &str) -> bool {
        self.owner(channel, Self::opposite(side)) != Some(owner)
            && self.owner(channel, side).is_none_or(|current| current == owner)
    }

    fn claim(&mut self, channel: &'static str, side: ChannelSide, owner: &'static str) {
        if self.owner(channel, side).is_none() {
            self.owners.push((channel, side, owner));
        }
    }

    fn property_meta(&self, id: u32) -> Option<(usize, bool, u128)> {
        self.properties
            .iter()
            .enumerate()
            .find(|(_, (p, _, _))| *p == id)
            .map(|(i, (_, managed, supply))| (i, *managed, *supply))
    }

    fn move_atoms(&mut self, p: u32, from: Slot, to: Slot, n: u128) -> bool {
        if n == 0 {
            return false;
        }
        let debit = self.atoms(p, &from);
        if debit < n {
            return false;
        }
        if from == to {
            return true;
        }
        let Some(credit) = self.atoms(p, &to).checked_add(n) else { return false };
        self.put(p, from, debit - n);
        self.put(p, to, credit);
        true
    }

    fn execute(&mut self, tx: u64, action: Action) -> bool {
        match action {
            Action::Send { property: p, from, to, amount } => {
                self.property_meta(p).is_some() && self.move_atoms(
                    p, Slot::Available(from), Slot::Available(to), amount)
            }
            Action::Mint { property: p, actor, to, amount } => {
                let Some((index, managed, supply)) = self.property_meta(p) else { return false };
                if !managed || actor != "alice" || amount == 0 {
                    return false;
                }
                let Some(total) = supply.checked_add(amount) else { return false };
                let old = self.atoms(p, &Slot::Available(to));
                let Some(credited) = old.checked_add(amount) else { return false };
                self.properties[index].2 = total;
                self.put(p, Slot::Available(to), credited);
                true
            }
            Action::Burn { property: p, actor, amount } => {
                let Some((index, managed, supply)) = self.property_meta(p) else { return false };
                if !managed || actor != "alice" || amount == 0 {
                    return false;
                }
                let balance = self.atoms(p, &Slot::Available("alice"));
                if balance < amount || supply < amount {
                    return false;
                }
                self.properties[index].2 = supply - amount;
                self.put(p, Slot::Available("alice"), balance - amount);
                true
            }
            Action::Commit { property: p, owner, channel, side, amount } => {
                if self.property_meta(p).is_none() || !self.can_claim(channel, side, owner) {
                    return false;
                }
                if self.move_atoms(
                    p, Slot::Available(owner), Slot::Escrow(channel, side), amount
                ) {
                    self.claim(channel, side, owner);
                    true
                } else {
                    false
                }
            }
            Action::Withdraw { property: p, owner, channel, side, amount, unlock } => {
                if self.property_meta(p).is_none()
                    || unlock == 0
                    || self.owner(channel, side) != Some(owner)
                {
                    return false;
                }
                if !self.move_atoms(
                    p, Slot::Escrow(channel, side), Slot::Reserved(tx), amount
                ) {
                    return false;
                }
                self.withdrawals.push(Withdrawal {
                    id: tx, property: p, owner, amount, unlock,
                });
                true
            }
            Action::ChannelMove {
                property: p, source, side, destination, receiving, amount,
            } => {
                if self.property_meta(p).is_none() || source == destination {
                    return false;
                }
                let Some(owner) = self.owner(source, side) else { return false };
                if !self.can_claim(destination, receiving, owner) {
                    return false;
                }
                if self.move_atoms(
                    p, Slot::Escrow(source, side), Slot::Escrow(destination, receiving), amount
                ) {
                    self.claim(destination, receiving, owner);
                    true
                } else {
                    false
                }
            }
        }
    }

    fn apply(&mut self, tx: u64, action: Action) -> bool {
        if self.used.contains(&tx) {
            return false;
        }
        let mut pending = self.clone();
        if !pending.execute(tx, action) {
            return false;
        }
        pending.used.push(tx);
        *self = pending;
        true
    }

    fn mature(&mut self, height: u64) -> Option<usize> {
        if height == 0 { return None }
        let mut staged = self.clone();
        let due: Vec<Withdrawal> = staged.withdrawals.iter()
            .filter(|x| x.unlock <= height).cloned().collect();
        for record in &due {
            let available = staged.atoms(record.property, &Slot::Available(record.owner));
            let credited = available.checked_add(record.amount)?;
            staged.put(record.property, Slot::Reserved(record.id), 0);
            staged.put(record.property, Slot::Available(record.owner), credited);
        }
        staged.withdrawals.retain(|record| record.unlock > height);
        *self = staged;
        Some(due.len())
    }

    fn expected_keys(&self, network: TradeLayerMainnet) -> Vec<(SupplyColumnProofKey, u128)> {
        let mut result = Vec::new();
        for (property, slot, atoms) in &self.columns {
            let (source_tag, identifier_bytes) = match slot {
                Slot::Available(name) => (0u8, name.as_bytes().to_vec()),
                Slot::Escrow(channel, ChannelSide::A) => (1, channel.as_bytes().to_vec()),
                Slot::Escrow(channel, ChannelSide::B) => (2, channel.as_bytes().to_vec()),
                Slot::Reserved(id) => (3, txid(*id).as_bytes().to_vec()),
            };
            result.push((SupplyColumnProofKey {
                network_tag: network.tag(), property_id: *property,
                source_tag, identifier_bytes,
            }, *atoms));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    fn expected_audit(&self) -> Vec<SupplyAuditRow> {
        let mut rows = Vec::new();
        for (p, _, supply) in &self.properties {
            let mut parts = [0u128; 3];
            for (id, slot, amount) in &self.columns {
                if id == p {
                    let bucket = match slot {
                        Slot::Available(_) => 0,
                        Slot::Escrow(_, _) => 1,
                        Slot::Reserved(_) => 2,
                    };
                    parts[bucket] = parts[bucket].checked_add(*amount).unwrap();
                }
            }
            assert_eq!(parts.iter().sum::<u128>(), *supply);
            rows.push(SupplyAuditRow {
                property: property(*p), total_supply_atoms: *supply,
                available_atoms: parts[0], escrow_atoms: parts[1],
                pending_withdrawal_atoms: parts[2],
            });
        }
        rows.sort_by_key(|row| row.property);
        rows
    }
}

fn check_concrete(ledger: &Ledger, spec: &RefLedger, step: u64) {
    ledger.verify_conservation().unwrap();
    assert_eq!(ledger.supply_audit_rows_unverified().unwrap(), spec.expected_audit(),
        "supply mismatch at iteration {step}");
    for network in [
        TradeLayerMainnet::Bitcoin, TradeLayerMainnet::Litecoin, TradeLayerMainnet::Dogecoin,
    ] {
        assert_eq!(
            ledger.supply_proof_keys_unverified(network).unwrap(),
            spec.expected_keys(network),
            "typed map mismatch for {network:?} at iteration {step}"
        );
    }
    assert_eq!(ledger.processed_count(), spec.used.len(), "replay count at {step}");
    for id in &spec.used {
        assert!(ledger.is_processed(txid(*id)), "accepted id missing at {step}");
    }
    if step % 67 == 0 {
        let encoded = ledger.snapshot_bytes().unwrap();
        let restored = Ledger::from_snapshot_bytes(&encoded).unwrap();
        assert_eq!(restored, *ledger);
        assert_eq!(restored.supply_audit_rows_unverified().unwrap(), spec.expected_audit());
    }
}

fn next(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *seed ^ (*seed >> 17)
}

#[test]
fn vector_oracle_refines_heterogeneous_rust_maps_through_randomized_operations() {
    const PEOPLE: [&str; 6] = ["alice", "bob", "carol", "dave", "erin", "frank"];
    const CHANNELS: [&str; 3] = ["chan-a", "chan-b", "chan-c"];

    let mut ledger = Ledger::new();
    ledger.apply(txid(1), Operation::CreateProperty {
        id: property(41), issuer: account("alice"), kind: PropertyKind::Fixed,
        initial_atoms: 1000,
    }).unwrap();
    ledger.apply(txid(2), Operation::CreateProperty {
        id: property(42), issuer: account("alice"), kind: PropertyKind::Managed,
        initial_atoms: 1000,
    }).unwrap();
    let mut spec = RefLedger::new();

    let initial = [
        (3, Action::Send { property: 41, from: "alice", to: "bob", amount: 300 }),
        (4, Action::Send { property: 42, from: "alice", to: "carol", amount: 220 }),
        (5, Action::Commit { property: 41, owner: "bob", channel: "chan-a",
            side: ChannelSide::A, amount: 80 }),
        (6, Action::Commit { property: 42, owner: "carol", channel: "chan-a",
            side: ChannelSide::B, amount: 60 }),
    ];
    for (tx, operation) in initial {
        assert!(spec.apply(tx, operation));
        ledger.apply(txid(tx), operation.into_operation()).unwrap();
    }
    check_concrete(&ledger, &spec, 0);

    let mut rng = 0xa701_d34b_5566_1234u64;
    let mut accepted = 0usize;
    let mut rejected = 0usize;
    for step in 0..1200u64 {
        let id = if next(&mut rng) % 2 == 0 { 41 } else { 42 };
        let who = PEOPLE[(next(&mut rng) as usize) % PEOPLE.len()];
        let other = PEOPLE[(next(&mut rng) as usize) % PEOPLE.len()];
        let channel = CHANNELS[(next(&mut rng) as usize) % CHANNELS.len()];
        let destination = CHANNELS[(next(&mut rng) as usize) % CHANNELS.len()];
        let side = if next(&mut rng) & 1 == 0 { ChannelSide::A } else { ChannelSide::B };
        let receiving = if next(&mut rng) & 1 == 0 { ChannelSide::A } else { ChannelSide::B };
        let amount = u128::from(next(&mut rng) % 180);
        let operation = match next(&mut rng) % 6 {
            0 => Action::Send { property: id, from: who, to: other, amount },
            1 => Action::Mint { property: id, actor: who, to: other, amount },
            2 => Action::Burn { property: id, actor: who, amount },
            3 => Action::Commit { property: id, owner: who, channel, side, amount },
            4 => Action::Withdraw {
                property: id, owner: who, channel, side, amount,
                unlock: if step % 53 == 0 { 0 } else { 40 + step * 2 },
            },
            _ => Action::ChannelMove {
                property: id, source: channel, side, destination, receiving, amount,
            },
        };
        let tx = step + 100;
        let before = ledger.clone();
        let expect = spec.apply(tx, operation);
        let actual = ledger.apply(txid(tx), operation.into_operation());
        assert_eq!(actual.is_ok(), expect, "step={step}, operation={operation:?}, actual={actual:?}");
        if expect {
            accepted += 1;
            assert!(ledger.is_processed(txid(tx)));
        } else {
            rejected += 1;
            assert_eq!(ledger, before, "rejected state changed at {step}");
            assert!(!ledger.is_processed(txid(tx)));
        }
        check_concrete(&ledger, &spec, step + 1);

        if step % 29 == 0 {
            let height = 40 + step * 2;
            let before = ledger.clone();
            let expected_mature = spec.mature(height);
            let actual_mature = ledger.settle_due_withdrawals_unverified(height);
            assert_eq!(actual_mature.ok(), expected_mature,
                "withdrawal maturity mismatch at {step}");
            if actual_mature.is_err() {
                assert_eq!(ledger, before);
            }
            check_concrete(&ledger, &spec, step + 2_000);
        }
    }
    assert!(accepted > 60, "exercise real successful mixed transitions");
    assert!(rejected > 60, "exercise real failed mixed transitions");
}
