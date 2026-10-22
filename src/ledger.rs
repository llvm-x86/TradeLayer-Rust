//! In-memory, deterministic accounting primitives for TradeLayer.
//!
//! THIS IS NOT CHAIN CONSENSUS. Account IDs are opaque identifiers supplied by
//! a trusted caller. The rules for mapping a signed on-chain transaction to an
//! actor, activation, whitelists, legacy state, and special token classes have
//! deliberately not been implemented. This module must not be connected to a
//! live chain until that validation layer and replay corpus exist.
//!
//! Every mutation uses a private clone and commits only after validating
//! conservation. This favors auditability at the current prototype scale;
//! copying the entire state per batch is NOT a production throughput design.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

/// Maximum opaque identifier length, matching the draft wire-v1 send field.
pub const MAX_ACCOUNT_BYTES: usize = 90;

/// Property IDs are nonzero. The actual chain-specific registry is separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PropertyId(u32);

impl PropertyId {
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for PropertyId {
    type Error = LedgerError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(LedgerError::InvalidPropertyId)
        } else {
            Ok(Self(value))
        }
    }
}

/// An opaque account identity. No chain/network or signature validation is
/// performed by this type. It is not proof of control of an address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountId(String);

impl AccountId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for AccountId {
    type Error = LedgerError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > MAX_ACCOUNT_BYTES
            || !bytes.iter().all(|b| (0x21..=0x7e).contains(b))
        {
            return Err(LedgerError::InvalidAccount);
        }
        Ok(Self(value.to_owned()))
    }
}

/// Unique transaction identifier. A future chain adapter must derive this
/// from verified transaction bytes, never user-supplied arbitrary metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TxId([u8; 32]);

impl TxId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Property classes supported by this initial accounting model.
/// Native/vesting/synthetic/procedural balances are out of scope; channels are an offline model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyKind {
    Fixed,
    Managed,
}

/// Metadata whose total supply is reconciled against all owned balances.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub issuer: AccountId,
    pub kind: PropertyKind,
    pub total_supply_atoms: u128,
}

/// Explicit ledger operations, intentionally separate from wire transaction
/// parsing and from the chain's actual transaction-type authorization policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    CreateProperty {
        id: PropertyId,
        issuer: AccountId,
        kind: PropertyKind,
        initial_atoms: u128,
    },
    Transfer {
        id: PropertyId,
        from: AccountId,
        to: AccountId,
        amount_atoms: u128,
    },
    Mint {
        id: PropertyId,
        issuer: AccountId,
        to: AccountId,
        amount_atoms: u128,
    },
    /// Managed-property issuer burns their own available units only.
    Burn {
        id: PropertyId,
        issuer: AccountId,
        amount_atoms: u128,
    },
    /// Type 4: move available tokens into an explicitly selected channel column.
    ChannelCommit {
        id: PropertyId,
        owner: AccountId,
        channel: AccountId,
        side: ChannelSide,
        amount_atoms: u128,
    },
    /// Type 21: reserve escrow for release at a separately checked block height.
    ChannelWithdraw {
        id: PropertyId,
        owner: AccountId,
        channel: AccountId,
        side: ChannelSide,
        amount_atoms: u128,
        unlock_height: u64,
    },
    /// Type 22: move escrow to another channel while retaining the column owner.
    ChannelTransfer {
        id: PropertyId,
        from_channel: AccountId,
        from_side: ChannelSide,
        to_channel: AccountId,
        to_side: ChannelSide,
        amount_atoms: u128,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    InvalidPropertyId,
    InvalidAccount,
    ZeroAmount,
    PropertyAlreadyExists(PropertyId),
    UnknownProperty(PropertyId),
    UnauthorizedIssuer(PropertyId),
    FixedSupply(PropertyId),
    InsufficientAvailable {
        property: PropertyId,
        available: u128,
        requested: u128,
    },
    ArithmeticOverflow,
    AlreadyProcessed(TxId),
    SupplyMismatch(PropertyId),
    BalanceWithoutProperty(PropertyId),
    NonCanonicalZeroBalance(PropertyId),
    UnknownChannel(AccountId),
    ChannelSideOccupied(AccountId),
    ChannelSideMismatch(AccountId),
    SameChannel,
    MalformedChannel(PropertyId),
    MalformedPendingWithdrawal(PropertyId),
    InvalidWithdrawalHeight,
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPropertyId => write!(f, "property ID must be nonzero"),
            Self::InvalidAccount => write!(f, "invalid account identifier"),
            Self::ZeroAmount => write!(f, "amount must be positive"),
            Self::PropertyAlreadyExists(id) => write!(f, "property {} already exists", id.get()),
            Self::UnknownProperty(id) => write!(f, "unknown property {}", id.get()),
            Self::UnauthorizedIssuer(id) => write!(f, "unauthorized issuer for {}", id.get()),
            Self::FixedSupply(id) => write!(f, "property {} has fixed supply", id.get()),
            Self::InsufficientAvailable {
                property,
                available,
                requested,
            } => write!(
                f,
                "property {} balance {} is below requested {}",
                property.get(),
                available,
                requested
            ),
            Self::ArithmeticOverflow => write!(f, "checked token arithmetic overflowed"),
            Self::AlreadyProcessed(_) => write!(f, "transaction already processed"),
            Self::SupplyMismatch(id) => write!(f, "property {} supply mismatch", id.get()),
            Self::BalanceWithoutProperty(id) => {
                write!(f, "balance exists without property {}", id.get())
            }
            Self::NonCanonicalZeroBalance(id) => {
                write!(f, "zero balance stored for property {}", id.get())
            }
            Self::UnknownChannel(id) => write!(f, "unknown channel {}", id.as_str()),
            Self::ChannelSideOccupied(id) => write!(f, "channel {} column occupied", id.as_str()),
            Self::ChannelSideMismatch(id) => write!(f, "channel {} participant mismatch", id.as_str()),
            Self::SameChannel => write!(f, "channel transfer requires distinct channels"),
            Self::MalformedChannel(id) => write!(f, "malformed channel balance {}", id.get()),
            Self::MalformedPendingWithdrawal(id) => write!(f, "malformed pending withdrawal {}", id.get()),
            Self::InvalidWithdrawalHeight => write!(f, "invalid withdrawal maturity height"),
        }
    }
}

impl std::error::Error for LedgerError {}

/// BTree collections keep iteration and equality independent of HashMap seeds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ledger {
    properties: BTreeMap<PropertyId, Property>,
    available: BTreeMap<(PropertyId, AccountId), u128>,
    channels: BTreeMap<AccountId, ChannelState>,
    pending_withdrawals: BTreeMap<TxId, PendingWithdrawal>,
    processed: BTreeSet<TxId>,
}

impl Ledger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn property(&self, id: PropertyId) -> Option<&Property> {
        self.properties.get(&id)
    }

    pub fn available_atoms(&self, id: PropertyId, account: &AccountId) -> u128 {
        self.available
            .get(&(id, account.clone()))
            .copied()
            .unwrap_or(0)
    }

    pub fn is_processed(&self, txid: TxId) -> bool {
        self.processed.contains(&txid)
    }

    pub fn processed_count(&self) -> usize {
        self.processed.len()
    }

    /// Atomically apply one operation. On error, all state is preserved.
    pub fn apply(&mut self, txid: TxId, op: Operation) -> Result<(), LedgerError> {
        self.apply_batch([(txid, op)])
    }

    /// Apply a batch in caller-supplied order. If even one transaction fails,
    /// *all* properties, balances, and replay markers remain unchanged.
    pub fn apply_batch(
        &mut self,
        transactions: impl IntoIterator<Item = (TxId, Operation)>,
    ) -> Result<(), LedgerError> {
        let mut pending = self.clone();
        for (txid, op) in transactions {
            if !pending.processed.insert(txid) {
                return Err(LedgerError::AlreadyProcessed(txid));
            }
            pending.execute(txid, op)?;
        }
        pending.verify_conservation()?;
        *self = pending;
        Ok(())
    }

    fn put_balance(&mut self, id: PropertyId, account: AccountId, atoms: u128) {
        if atoms == 0 {
            self.available.remove(&(id, account));
        } else {
            self.available.insert((id, account), atoms);
        }
    }

    fn existing_property(&self, id: PropertyId) -> Result<&Property, LedgerError> {
        self.properties
            .get(&id)
            .ok_or(LedgerError::UnknownProperty(id))
    }

    fn check_managed_issuer(
        &self,
        id: PropertyId,
        actor: &AccountId,
    ) -> Result<&Property, LedgerError> {
        let property = self.existing_property(id)?;
        if property.kind != PropertyKind::Managed {
            return Err(LedgerError::FixedSupply(id));
        }
        if &property.issuer != actor {
            return Err(LedgerError::UnauthorizedIssuer(id));
        }
        Ok(property)
    }

    fn execute(&mut self, txid: TxId, operation: Operation) -> Result<(), LedgerError> {
        match operation {
            Operation::CreateProperty {
                id,
                issuer,
                kind,
                initial_atoms,
            } => {
                if initial_atoms == 0 {
                    return Err(LedgerError::ZeroAmount);
                }
                if self.properties.contains_key(&id) {
                    return Err(LedgerError::PropertyAlreadyExists(id));
                }
                self.properties.insert(
                    id,
                    Property {
                        issuer: issuer.clone(),
                        kind,
                        total_supply_atoms: initial_atoms,
                    },
                );
                self.put_balance(id, issuer, initial_atoms);
                Ok(())
            }
            Operation::Transfer {
                id,
                from,
                to,
                amount_atoms,
            } => {
                self.existing_property(id)?;
                if amount_atoms == 0 {
                    return Err(LedgerError::ZeroAmount);
                }
                let from_before = self.available_atoms(id, &from);
                if from_before < amount_atoms {
                    return Err(LedgerError::InsufficientAvailable {
                        property: id,
                        available: from_before,
                        requested: amount_atoms,
                    });
                }
                // Self-transfers consume the transaction ID after proving funds,
                // but cannot create a spurious overflow by double-counting.
                if from == to {
                    return Ok(());
                }
                let to_after = self
                    .available_atoms(id, &to)
                    .checked_add(amount_atoms)
                    .ok_or(LedgerError::ArithmeticOverflow)?;
                self.put_balance(id, from, from_before - amount_atoms);
                self.put_balance(id, to, to_after);
                Ok(())
            }
            Operation::Mint {
                id,
                issuer,
                to,
                amount_atoms,
            } => {
                if amount_atoms == 0 {
                    return Err(LedgerError::ZeroAmount);
                }
                let supply_after = self
                    .check_managed_issuer(id, &issuer)?
                    .total_supply_atoms
                    .checked_add(amount_atoms)
                    .ok_or(LedgerError::ArithmeticOverflow)?;
                let balance_after = self
                    .available_atoms(id, &to)
                    .checked_add(amount_atoms)
                    .ok_or(LedgerError::ArithmeticOverflow)?;
                self.properties
                    .get_mut(&id)
                    .ok_or(LedgerError::UnknownProperty(id))?
                    .total_supply_atoms = supply_after;
                self.put_balance(id, to, balance_after);
                Ok(())
            }
            Operation::Burn {
                id,
                issuer,
                amount_atoms,
            } => {
                if amount_atoms == 0 {
                    return Err(LedgerError::ZeroAmount);
                }
                let supply = self.check_managed_issuer(id, &issuer)?.total_supply_atoms;
                let balance = self.available_atoms(id, &issuer);
                if balance < amount_atoms {
                    return Err(LedgerError::InsufficientAvailable {
                        property: id,
                        available: balance,
                        requested: amount_atoms,
                    });
                }
                let supply_after = supply
                    .checked_sub(amount_atoms)
                    .ok_or(LedgerError::SupplyMismatch(id))?;
                self.properties
                    .get_mut(&id)
                    .ok_or(LedgerError::UnknownProperty(id))?
                    .total_supply_atoms = supply_after;
                self.put_balance(id, issuer, balance - amount_atoms);
                Ok(())
            }
            Operation::ChannelCommit { id, owner, channel, side, amount_atoms } =>
                self.commit_channel(id, owner, channel, side, amount_atoms),
            Operation::ChannelWithdraw { id, owner, channel, side, amount_atoms, unlock_height } =>
                self.request_channel_withdrawal(txid, id, owner, channel, side, amount_atoms, unlock_height),
            Operation::ChannelTransfer { id, from_channel, from_side, to_channel, to_side, amount_atoms } =>
                self.transfer_channel(id, from_channel, from_side, to_channel, to_side, amount_atoms),
        }
    }

    /// Verify that each property's issued supply equals the sum of all
    /// available balances. This first slice has no reserved or other columns.
    pub fn verify_conservation(&self) -> Result<(), LedgerError> {
        let mut sums: BTreeMap<PropertyId, u128> = BTreeMap::new();
        for ((id, _account), atoms) in &self.available {
            if *atoms == 0 {
                return Err(LedgerError::NonCanonicalZeroBalance(*id));
            }
            if !self.properties.contains_key(id) {
                return Err(LedgerError::BalanceWithoutProperty(*id));
            }
            let sum = sums.get(id).copied().unwrap_or(0);
            sums.insert(
                *id,
                sum.checked_add(*atoms)
                    .ok_or(LedgerError::ArithmeticOverflow)?,
            );
        }
        self.add_channel_conservation(&mut sums)?;
        for (id, property) in &self.properties {
            let owned = sums.get(id).copied().unwrap_or(0);
            if owned != property.total_supply_atoms {
                return Err(LedgerError::SupplyMismatch(*id));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: u32) -> PropertyId {
        PropertyId::try_from(id).unwrap()
    }
    fn a(name: &str) -> AccountId {
        AccountId::try_from(name).unwrap()
    }
    fn tx(n: u8) -> TxId {
        TxId::from_bytes([n; 32])
    }

    #[test]
    fn checked_account_and_property_ids() {
        assert_eq!(PropertyId::try_from(0), Err(LedgerError::InvalidPropertyId));
        for invalid in ["", "alice bob", "\n", "🚀"] {
            assert_eq!(
                AccountId::try_from(invalid),
                Err(LedgerError::InvalidAccount)
            );
        }
        assert_eq!(
            AccountId::try_from("a".repeat(MAX_ACCOUNT_BYTES + 1).as_str()),
            Err(LedgerError::InvalidAccount)
        );
        assert_eq!(a("alice").as_str(), "alice");
    }

    #[test]
    fn rejects_supply_mismatch() {
        let mut ledger = Ledger::new();
        ledger
            .apply(
                tx(1),
                Operation::CreateProperty {
                    id: p(7),
                    issuer: a("alice"),
                    kind: PropertyKind::Fixed,
                    initial_atoms: 10,
                },
            )
            .unwrap();
        ledger.available.insert((p(7), a("alice")), 11);
        assert_eq!(
            ledger.verify_conservation(),
            Err(LedgerError::SupplyMismatch(p(7)))
        );
    }

    #[test]
    fn rejects_orphan_balances_and_zero_entries() {
        let mut ledger = Ledger::new();
        ledger.available.insert((p(7), a("alice")), 1);
        assert_eq!(
            ledger.verify_conservation(),
            Err(LedgerError::BalanceWithoutProperty(p(7)))
        );
        ledger.available.insert((p(7), a("alice")), 0);
        assert_eq!(
            ledger.verify_conservation(),
            Err(LedgerError::NonCanonicalZeroBalance(p(7)))
        );
    }
}

/// Canonical, checksummed checkpoint encoding for the prototype ledger.
/// A checksum detects accidental corruption, not malicious modification.
/// These bytes are NOT an independently verified consensus state commitment.
impl Ledger {
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, SnapshotError> {
        use crate::codec::{with_crc, MAX_CHECKPOINT_BYTES};
        self.verify_conservation()
            .map_err(SnapshotError::InvalidLedger)?;
        if [
            self.properties.len(),
            self.available.len(),
            self.processed.len(),
            self.channels.len(),
            self.pending_withdrawals.len(),
        ]
        .iter()
        .any(|&count| count > MAX_SNAPSHOT_ENTRIES)
        {
            return Err(SnapshotError::InvalidFormat("too many ledger records"));
        }
        let mut bytes = Vec::new();
        let channel_version = !self.channels.is_empty() || !self.pending_withdrawals.is_empty();
        bytes.extend_from_slice(if channel_version { b"TLL\x02" } else { b"TLL\x01" });
        bytes.extend_from_slice(&(self.properties.len() as u32).to_be_bytes());
        for (id, property) in &self.properties {
            bytes.extend_from_slice(&id.get().to_be_bytes());
            push_account(&mut bytes, &property.issuer);
            bytes.push(match property.kind {
                PropertyKind::Fixed => 1,
                PropertyKind::Managed => 2,
            });
            bytes.extend_from_slice(&property.total_supply_atoms.to_be_bytes());
        }
        bytes.extend_from_slice(&(self.available.len() as u32).to_be_bytes());
        for ((id, account), amount) in &self.available {
            bytes.extend_from_slice(&id.get().to_be_bytes());
            push_account(&mut bytes, account);
            bytes.extend_from_slice(&amount.to_be_bytes());
        }
        bytes.extend_from_slice(&(self.processed.len() as u32).to_be_bytes());
        for txid in &self.processed {
            bytes.extend_from_slice(&txid.as_bytes());
        }
        if channel_version {
            self.append_channel_snapshot(&mut bytes)?;
        }
        if bytes.len() > MAX_CHECKPOINT_BYTES - 4 {
            return Err(SnapshotError::InvalidFormat("ledger checkpoint too large"));
        }
        with_crc(bytes).map_err(|_| SnapshotError::InvalidFormat("ledger checkpoint too large"))
    }

    /// Decode only canonical ascending records. Unknown format versions,
    /// duplicates, trailing bytes, and broken conservation fail closed.
    pub fn from_snapshot_bytes(input: &[u8]) -> Result<Self, SnapshotError> {
        use crate::codec::{verify_crc, Cursor};
        let payload = verify_crc(input).map_err(snapshot_codec_error)?;
        let mut reader = Cursor::new(payload);
        let channel_version = match reader.fixed::<4>().map_err(snapshot_codec_error)? {
            [b'T', b'L', b'L', 1] => false,
            [b'T', b'L', b'L', 2] => true,
            _ => return Err(SnapshotError::InvalidFormat("ledger checkpoint version")),
        };
        let mut state = Self::new();
        let count = snapshot_count(&mut reader)?;
        let mut last = None;
        for _ in 0..count {
            let id = PropertyId::try_from(reader.u32().map_err(snapshot_codec_error)?)
                .map_err(SnapshotError::InvalidLedger)?;
            if last.is_some_and(|previous| id <= previous) {
                return Err(SnapshotError::InvalidFormat("unsorted property records"));
            }
            last = Some(id);
            let issuer = read_account(&mut reader)?;
            let kind = match reader.u8().map_err(snapshot_codec_error)? {
                1 => PropertyKind::Fixed,
                2 => PropertyKind::Managed,
                _ => return Err(SnapshotError::InvalidFormat("property kind")),
            };
            let supply = reader.u128().map_err(snapshot_codec_error)?;
            state.properties.insert(
                id,
                Property {
                    issuer,
                    kind,
                    total_supply_atoms: supply,
                },
            );
        }
        let count = snapshot_count(&mut reader)?;
        let mut last: Option<(PropertyId, AccountId)> = None;
        for _ in 0..count {
            let id = PropertyId::try_from(reader.u32().map_err(snapshot_codec_error)?)
                .map_err(SnapshotError::InvalidLedger)?;
            let account = read_account(&mut reader)?;
            let key = (id, account);
            if last.as_ref().is_some_and(|previous| &key <= previous) {
                return Err(SnapshotError::InvalidFormat("unsorted balance records"));
            }
            let amount = reader.u128().map_err(snapshot_codec_error)?;
            if amount == 0 {
                return Err(SnapshotError::InvalidFormat("zero balance"));
            }
            last = Some(key.clone());
            state.available.insert(key, amount);
        }
        let count = snapshot_count(&mut reader)?;
        let mut last: Option<TxId> = None;
        for _ in 0..count {
            let id = TxId::from_bytes(reader.fixed::<32>().map_err(snapshot_codec_error)?);
            if last.is_some_and(|previous| id <= previous) {
                return Err(SnapshotError::InvalidFormat("unsorted replay markers"));
            }
            last = Some(id);
            state.processed.insert(id);
        }
        if channel_version {
            state.read_channel_snapshot(&mut reader)?;
        }
        reader.end().map_err(snapshot_codec_error)?;
        state
            .verify_conservation()
            .map_err(SnapshotError::InvalidLedger)?;
        if state.snapshot_bytes()?.as_slice() != input {
            return Err(SnapshotError::InvalidFormat("noncanonical ledger snapshot"));
        }
        Ok(state)
    }
}

const MAX_SNAPSHOT_ENTRIES: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    InvalidFormat(&'static str),
    InvalidLedger(LedgerError),
}

impl core::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for SnapshotError {}

fn snapshot_codec_error(err: crate::codec::CodecError) -> SnapshotError {
    use crate::codec::CodecError;
    match err {
        CodecError::BadChecksum => SnapshotError::InvalidFormat("ledger checksum"),
        CodecError::TooLarge => SnapshotError::InvalidFormat("ledger oversized"),
        _ => SnapshotError::InvalidFormat("ledger malformed"),
    }
}

fn snapshot_count(reader: &mut crate::codec::Cursor<'_>) -> Result<usize, SnapshotError> {
    let count = reader.u32().map_err(snapshot_codec_error)? as usize;
    if count > MAX_SNAPSHOT_ENTRIES {
        Err(SnapshotError::InvalidFormat("too many ledger records"))
    } else {
        Ok(count)
    }
}

fn push_account(bytes: &mut Vec<u8>, account: &AccountId) {
    let raw = account.as_str().as_bytes();
    // AccountId can only be constructed after checking MAX_ACCOUNT_BYTES.
    bytes.push(raw.len() as u8);
    bytes.extend_from_slice(raw);
}

fn read_account(reader: &mut crate::codec::Cursor<'_>) -> Result<AccountId, SnapshotError> {
    let len = reader.u8().map_err(snapshot_codec_error)? as usize;
    if len == 0 || len > MAX_ACCOUNT_BYTES {
        return Err(SnapshotError::InvalidFormat("account length"));
    }
    let raw = reader.take(len).map_err(snapshot_codec_error)?;
    let value =
        core::str::from_utf8(raw).map_err(|_| SnapshotError::InvalidFormat("account encoding"))?;
    AccountId::try_from(value).map_err(SnapshotError::InvalidLedger)
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    fn account(value: &str) -> AccountId {
        AccountId::try_from(value).unwrap()
    }
    fn prop(value: u32) -> PropertyId {
        PropertyId::try_from(value).unwrap()
    }
    fn tx(v: u8) -> TxId {
        TxId::from_bytes([v; 32])
    }

    #[test]
    fn round_trip_and_byte_stability() {
        let mut state = Ledger::new();
        state
            .apply(
                tx(1),
                Operation::CreateProperty {
                    id: prop(7),
                    issuer: account("issuer"),
                    kind: PropertyKind::Managed,
                    initial_atoms: 100_000_000,
                },
            )
            .unwrap();
        state
            .apply(
                tx(2),
                Operation::Transfer {
                    id: prop(7),
                    from: account("issuer"),
                    to: account("recipient"),
                    amount_atoms: 43,
                },
            )
            .unwrap();
        let bytes = state.snapshot_bytes().unwrap();
        let recovered = Ledger::from_snapshot_bytes(&bytes).unwrap();
        assert_eq!(state, recovered);
        assert_eq!(bytes, recovered.snapshot_bytes().unwrap());
        let mut damaged = bytes;
        damaged[10] ^= 1;
        assert!(Ledger::from_snapshot_bytes(&damaged).is_err());
    }

    #[test]
    fn cannot_load_truncation_or_trailing_bytes() {
        let bytes = Ledger::new().snapshot_bytes().unwrap();
        for len in 0..bytes.len() {
            assert!(Ledger::from_snapshot_bytes(&bytes[..len]).is_err());
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(Ledger::from_snapshot_bytes(&extra).is_err());
    }
}

// Escrow book and pending withdrawal checkpoint extension.
include!("ledger_channels_impl.rs");
