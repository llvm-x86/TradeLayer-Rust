//! Stage 19, offline type-5 spot order matching and type-6 cancellation.
//!
//! Historical TradeLayer.js queues type-5 orders by pair and block, sorts
//! same-block senders by their last three address characters, and reserves the
//! offered asset before matching. This bounded Rust prototype implements that
//! ordering and an explicit *integer quote/base rational* safety policy.
//!
//! The legacy JS matcher's sell-side price uses amountOffered/amountExpected,
//! whereas the buy-side price has the opposite units. This experiment does
//! NOT claim economic consensus parity with JS. Stop/post orders, restricted
//! native/vesting/procedural assets, contract cancels, price cancels, and type 3
//! UTXO-settled trades fail closed until separately specified and verified.
//!
//! All actors, heights, tokens and transaction IDs are caller assertions.
//! This type owns its Ledger and internal escrow accounts; exposing it as a
//! trusted adapter, accepting network funds, or persisting only the Ledger
//! without orders would be unsafe. No live-chain or settlement authorization.

use crate::hash256::double_sha256;
use crate::ledger::{AccountId, Ledger, LedgerError, Operation, PropertyId, TxId};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_SPOT_OPEN_AND_QUEUED: usize = 4096;
pub const MAX_SPOT_FILLS: usize = 16384;
pub const MAX_SPOT_INPUT_EVENTS: usize = 65536;
const ESCROW_PREFIX: &str = "~spot:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpotPair {
    pub base: PropertyId,
    pub quote: PropertyId,
}

impl SpotPair {
    pub fn new(first: PropertyId, second: PropertyId) -> Result<Self, SpotError> {
        if first == second {
            return Err(SpotError::SameAsset);
        }
        Ok(Self {
            base: first.min(second),
            quote: first.max(second),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotSide {
    SellBase,
    BuyBase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotIntent {
    pub offered: PropertyId,
    pub desired: PropertyId,
    pub offered_atoms: u64,
    pub expected_atoms: u64,
    pub post: bool,
    pub stop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedSpotPlacement {
    pub txid: TxId,
    pub actor: AccountId,
    pub height: u64,
    pub intent: SpotIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelSelector {
    All,
    ByOrderTxId(TxId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedSpotCancellation {
    pub txid: TxId,
    pub actor: AccountId,
    pub height: u64,
    pub offered: PropertyId,
    pub desired: PropertyId,
    pub is_contract: bool,
    pub selector: CancelSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpotError {
    SameAsset,
    ZeroAmount,
    RestrictedProperty,
    UnknownProperty,
    ReservedAccount,
    EscrowCollision,
    UnsupportedFlag,
    UnsupportedContractCancel,
    UnsupportedPriceCancel,
    AuthenticatedUtxoProofRequired,
    Replay(TxId),
    InvalidHeight,
    HeightRegression,
    CapacityExceeded,
    NoMatchingOrder,
    UnauthorizedCancellation,
    SameOwnerCross,
    UnrepresentableFill,
    ArithmeticOverflow,
    ReserveMismatch,
    UnsupportedProtocolOperation,
    Ledger(LedgerError),
}

impl std::fmt::Display for SpotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SpotError {}
impl From<LedgerError> for SpotError {
    fn from(err: LedgerError) -> Self {
        Self::Ledger(err)
    }
}

/// Private source ledger transfers into distinct simulated accounts derived
/// from full 32-byte order IDs. Order IDs are always retained in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestingSpotOrder {
    pub txid: TxId,
    pub owner: AccountId,
    pub height: u64,
    pub offered: PropertyId,
    pub desired: PropertyId,
    pub offered_atoms: u64,
    pub expected_atoms: u64,
    pub remaining_offered: u64,
    pub remaining_desired: u64,
    sequence: u64,
}

impl RestingSpotOrder {
    pub fn pair(&self) -> SpotPair {
        SpotPair::new(self.offered, self.desired).expect("validated order")
    }
    pub fn side(&self) -> SpotSide {
        if self.offered == self.pair().base {
            SpotSide::SellBase
        } else {
            SpotSide::BuyBase
        }
    }
    /// (quote_atoms, base_atoms), unreduced to avoid rounding drift.
    pub fn quote_per_base(&self) -> (u64, u64) {
        match self.side() {
            SpotSide::SellBase => (self.expected_atoms, self.offered_atoms),
            SpotSide::BuyBase => (self.offered_atoms, self.expected_atoms),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotFill {
    pub sell_txid: TxId,
    pub buy_txid: TxId,
    pub pair: SpotPair,
    pub base_atoms: u64,
    pub quote_atoms: u64,
    pub trigger_txid: TxId,
    pub at_height: u64,
}

/// Standalone book/ledger pair. Its Ledger cannot be separately mutated once
/// moved into this wrapper. Existing checkpoint formats do not serialize
/// these orders and MUST NOT be used to restore this combined object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotExchange {
    ledger: Ledger,
    permitted: BTreeSet<PropertyId>,
    queued: Vec<UnverifiedSpotPlacement>,
    books: BTreeMap<SpotPair, Vec<RestingSpotOrder>>,
    seen: BTreeSet<TxId>,
    fills: Vec<SpotFill>,
    next_sequence: u64,
    processed_through: Option<u64>,
    /// Irreversible in this version: TLSP cannot encode clearing history.
    clearing_activity: bool,
}

impl SpotExchange {
    /// Explicitly caller-selected *unverified* policy. IDs 1-4 remain
    /// excluded until the native/country/vesting rules are ported.
    pub fn from_ledger_unverified(
        ledger: Ledger,
        permitted: BTreeSet<PropertyId>,
    ) -> Result<Self, SpotError> {
        if permitted.iter().any(|id| id.get() <= 4) {
            return Err(SpotError::RestrictedProperty);
        }
        if permitted.iter().any(|id| ledger.property(*id).is_none()) {
            return Err(SpotError::UnknownProperty);
        }
        ledger.verify_conservation()?;
        if !ledger.balances_with_account_prefix("~").is_empty() {
            return Err(SpotError::EscrowCollision);
        }
        Ok(Self {
            ledger,
            permitted,
            queued: Vec::new(),
            books: BTreeMap::new(),
            seen: BTreeSet::new(),
            fills: Vec::new(),
            next_sequence: 0,
            processed_through: None,
            clearing_activity: false,
        })
    }

    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Immutable policy and spot replay markers for opt-in Stage 22
    /// single-ledger migration and its joint replay audit.
    pub fn permitted_properties(&self) -> &BTreeSet<PropertyId> {
        &self.permitted
    }

    pub fn spot_input_ids_unverified(&self) -> &BTreeSet<TxId> {
        &self.seen
    }

    /// The Stage 22 joint state owns this exchange as its ONLY token ledger.
    /// This path is restricted to non-channel ledger operations and refuses
    /// all synthetic escrow account addresses. Each operation is staged on a
    /// clone and must leave existing order reserves intact.
    pub fn apply_joint_ledger_operation_unverified(
        &mut self,
        txid: TxId,
        operation: Operation,
    ) -> Result<(), SpotError> {
        let allowed = match &operation {
            Operation::CreateProperty { issuer, .. } => !issuer.as_str().starts_with('~'),
            Operation::Transfer { from, to, .. } => {
                !from.as_str().starts_with('~') && !to.as_str().starts_with('~')
            }
            Operation::Mint { issuer, to, .. } => {
                !issuer.as_str().starts_with('~') && !to.as_str().starts_with('~')
            }
            Operation::Burn { issuer, .. } => !issuer.as_str().starts_with('~'),
            _ => return Err(SpotError::UnsupportedProtocolOperation),
        };
        if !allowed {
            return Err(SpotError::ReservedAccount);
        }
        if self.seen.contains(&txid) || self.ledger.is_processed(txid) {
            return Err(SpotError::Replay(txid));
        }
        let mut candidate = self.clone();
        candidate.ledger.apply(txid, operation)?;
        candidate.verify_reserves()?;
        *self = candidate;
        Ok(())
    }
    /// Stage 25B research-only collateral transfer into/between/out of
    /// clearing escrows while this SpotExchange retains the SOLE Ledger.
    /// Never accepts an existing spot escrow or an arbitrary '~' namespace.
    /// The caller must derive and validate clearing escrow identities and
    /// preserve the accompanying clearing position metadata atomically.
    pub(crate) fn apply_clearing_transfer_unverified(
        &mut self,
        txid: TxId,
        id: PropertyId,
        from: AccountId,
        to: AccountId,
        amount_atoms: u128,
    ) -> Result<(), SpotError> {
        let margin_from = from.as_str().starts_with("~clrm:");
        let margin_to = to.as_str().starts_with("~clrm:");
        if !(margin_from || margin_to) {
            return Err(SpotError::ReservedAccount);
        }
        if from.as_str().starts_with('~') && !margin_from {
            return Err(SpotError::ReservedAccount);
        }
        if to.as_str().starts_with('~') && !margin_to {
            return Err(SpotError::ReservedAccount);
        }
        if self.seen.contains(&txid) || self.ledger.is_processed(txid) {
            return Err(SpotError::Replay(txid));
        }
        let mut candidate = self.clone();
        candidate.ledger.apply(
            txid,
            Operation::Transfer {
                id,
                from,
                to,
                amount_atoms,
            },
        )?;
        candidate.verify_reserves()?;
        candidate.clearing_activity = true;
        *self = candidate;
        Ok(())
    }

    pub(crate) fn has_clearing_activity_unverified(&self) -> bool {
        self.clearing_activity
    }

    /// A completed clearing event may have zero transfer legs. Persist the
    /// in-memory research-history fence even when no margin atoms move;
    /// otherwise TLSP or an old joint checkpoint can orphan its replay ID.
    /// The caller commits this change only on its staged atomic clone.
    pub(crate) fn note_clearing_event_unverified(&mut self) {
        self.clearing_activity = true;
    }

    pub fn processed_through(&self) -> Option<u64> {
        self.processed_through
    }

    /// Initial state from the caller's prefunded offline ledger, before
    /// any spot event has been queued, matched, or canceled.
    pub fn is_pristine_unverified(&self) -> bool {
        !self.clearing_activity
            && self.processed_through.is_none()
            && self.seen.is_empty()
            && self.queued.is_empty()
            && self.books.is_empty()
            && self.fills.is_empty()
            && self.next_sequence == 0
    }
    pub fn pending_count(&self) -> usize {
        self.queued.len()
    }

    /// Research-only ordering check for the combined clearing mark lane.
    /// Marking an epoch before all due spot queue entries are applied would
    /// manufacture an end-of-block result with a different economic order.
    pub(crate) fn has_queued_spot_at_or_before_unverified(&self, height: u64) -> bool {
        self.queued.iter().any(|entry| entry.height <= height)
    }
    pub fn open_count(&self) -> usize {
        self.books.values().map(Vec::len).sum()
    }
    pub fn fills(&self) -> &[SpotFill] {
        &self.fills
    }
    pub fn open_orders(&self, pair: SpotPair) -> &[RestingSpotOrder] {
        self.books.get(&pair).map_or(&[], Vec::as_slice)
    }

    fn check_pair(&self, offered: PropertyId, desired: PropertyId) -> Result<SpotPair, SpotError> {
        let pair = SpotPair::new(offered, desired)?;
        if !self.permitted.contains(&offered) || !self.permitted.contains(&desired) {
            return Err(SpotError::RestrictedProperty);
        }
        if self.ledger.property(offered).is_none() || self.ledger.property(desired).is_none() {
            return Err(SpotError::UnknownProperty);
        }
        Ok(pair)
    }

    /// Queue a type-5 order; reserve and matching occur at block close.
    /// No signature/txid/height origin is authenticated by this API.
    pub fn enqueue_type5_unverified(
        &mut self,
        entry: UnverifiedSpotPlacement,
    ) -> Result<(), SpotError> {
        if entry.height == 0 {
            return Err(SpotError::InvalidHeight);
        }
        if self.processed_through.is_some_and(|h| entry.height <= h) {
            return Err(SpotError::HeightRegression);
        }
        if entry.actor.as_str().starts_with('~') {
            return Err(SpotError::ReservedAccount);
        }
        self.check_pair(entry.intent.offered, entry.intent.desired)?;
        if entry.intent.offered_atoms == 0 || entry.intent.expected_atoms == 0 {
            return Err(SpotError::ZeroAmount);
        }
        if entry.intent.post || entry.intent.stop {
            return Err(SpotError::UnsupportedFlag);
        }
        if self.seen.contains(&entry.txid) {
            return Err(SpotError::Replay(entry.txid));
        }
        if self.seen.len() >= MAX_SPOT_INPUT_EVENTS
            || self.open_count() + self.pending_count() >= MAX_SPOT_OPEN_AND_QUEUED
        {
            return Err(SpotError::CapacityExceeded);
        }
        self.seen.insert(entry.txid);
        self.queued.push(entry);
        Ok(())
    }

    /// Applies all queued orders whose claimed height is <= height, sorted by
    /// canonical pair, then block height, sender suffix, and full txid.
    /// Earlier work in the batch is discarded if any later order fails.
    pub fn process_through_unverified(&mut self, height: u64) -> Result<usize, SpotError> {
        if height == 0 {
            return Err(SpotError::InvalidHeight);
        }
        if self.processed_through.is_some_and(|h| height <= h) {
            return Err(SpotError::HeightRegression);
        }
        let mut staged = self.clone();
        staged.queued.sort_by(|a, b| {
            let pa = SpotPair::new(a.intent.offered, a.intent.desired).expect("queued pair");
            let pb = SpotPair::new(b.intent.offered, b.intent.desired).expect("queued pair");
            pa.cmp(&pb)
                .then_with(|| a.height.cmp(&b.height))
                .then_with(|| compare_legacy_sender_suffix(&a.actor, &b.actor))
                .then_with(|| a.txid.cmp(&b.txid))
        });
        let entries = std::mem::take(&mut staged.queued);
        let mut applied = 0;
        for entry in entries {
            if entry.height <= height {
                staged.admit_one(entry)?;
                applied += 1;
            } else {
                staged.queued.push(entry);
            }
        }
        staged.ledger.verify_conservation()?;
        staged.verify_reserves()?;
        staged.processed_through = Some(height);
        *self = staged;
        Ok(applied)
    }

    fn admit_one(&mut self, entry: UnverifiedSpotPlacement) -> Result<(), SpotError> {
        let account = escrow_account(entry.txid)?;
        if self.ledger.available_atoms(entry.intent.offered, &account) != 0 {
            return Err(SpotError::EscrowCollision);
        }
        let pair = self.check_pair(entry.intent.offered, entry.intent.desired)?;
        self.ledger.apply(
            internal_txid(entry.txid, b"reserve", 0),
            Operation::Transfer {
                id: entry.intent.offered,
                from: entry.actor.clone(),
                to: account,
                amount_atoms: u128::from(entry.intent.offered_atoms),
            },
        )?;
        let order = RestingSpotOrder {
            txid: entry.txid,
            owner: entry.actor,
            height: entry.height,
            offered: entry.intent.offered,
            desired: entry.intent.desired,
            offered_atoms: entry.intent.offered_atoms,
            expected_atoms: entry.intent.expected_atoms,
            remaining_offered: entry.intent.offered_atoms,
            remaining_desired: entry.intent.expected_atoms,
            sequence: self.next_sequence,
        };
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(SpotError::ArithmeticOverflow)?;
        self.books.entry(pair).or_default().push(order);
        self.match_pair(pair, entry.txid, entry.height)?;
        Ok(())
    }

    fn match_pair(&mut self, pair: SpotPair, trigger: TxId, height: u64) -> Result<(), SpotError> {
        loop {
            let orders = self.books.get(&pair).expect("inserted pair exists");
            let sell_index = orders
                .iter()
                .enumerate()
                .filter(|(_, order)| order.side() == SpotSide::SellBase)
                .min_by(|(_, a), (_, b)| price_cmp(a, b).then_with(|| priority_cmp(a, b)))
                .map(|(i, _)| i);
            let buy_index = orders
                .iter()
                .enumerate()
                .filter(|(_, order)| order.side() == SpotSide::BuyBase)
                .min_by(|(_, a), (_, b)| price_cmp(b, a).then_with(|| priority_cmp(a, b)))
                .map(|(i, _)| i);
            let (sell_index, buy_index) = match (sell_index, buy_index) {
                (Some(sell), Some(buy)) => (sell, buy),
                _ => return Ok(()),
            };
            let seller = orders[sell_index].clone();
            let buyer = orders[buy_index].clone();
            if price_cmp(&buyer, &seller) == Ordering::Less {
                return Ok(());
            }
            // A self-cross stays in escrow for explicit cancellation. The JS
            // self-trade path drops an order without consistently unreserving.
            if seller.owner == buyer.owner {
                return Ok(());
            }
            if self.fills.len() >= MAX_SPOT_FILLS {
                return Err(SpotError::CapacityExceeded);
            }
            let base = seller.remaining_offered.min(buyer.remaining_desired);
            if base == 0 {
                return Err(SpotError::UnrepresentableFill);
            }
            // Legacy same-block matching uses the buy price when post=false.
            let price_source = if seller.height < buyer.height {
                &seller
            } else {
                &buyer
            };
            let (num, den) = price_source.quote_per_base();
            let amount = (u128::from(base) * u128::from(num)).div_ceil(u128::from(den));
            let quote = u64::try_from(amount).map_err(|_| SpotError::UnrepresentableFill)?;
            let sell_limit = u128::from(base) * u128::from(seller.expected_atoms);
            let buy_limit = u128::from(base) * u128::from(buyer.offered_atoms);
            // The ceil-to-one-atom rounding may violate a limit price.
            if quote == 0
                || quote > buyer.remaining_offered
                || u128::from(quote) * u128::from(seller.offered_atoms) < sell_limit
                || u128::from(quote) * u128::from(buyer.expected_atoms) > buy_limit
            {
                return Err(SpotError::UnrepresentableFill);
            }
            let buyer_done = buyer.remaining_desired == base;
            let refund = if buyer_done {
                buyer.remaining_offered - quote
            } else {
                0
            };
            let nonce = u32::try_from(self.fills.len()).map_err(|_| SpotError::CapacityExceeded)?;
            let mut legs = vec![
                (
                    internal_txid(trigger, b"fill-base", nonce),
                    Operation::Transfer {
                        id: pair.base,
                        from: escrow_account(seller.txid)?,
                        to: buyer.owner.clone(),
                        amount_atoms: u128::from(base),
                    },
                ),
                (
                    internal_txid(trigger, b"fill-quote", nonce),
                    Operation::Transfer {
                        id: pair.quote,
                        from: escrow_account(buyer.txid)?,
                        to: seller.owner.clone(),
                        amount_atoms: u128::from(quote),
                    },
                ),
            ];
            if refund > 0 {
                legs.push((
                    internal_txid(trigger, b"refund", nonce),
                    Operation::Transfer {
                        id: pair.quote,
                        from: escrow_account(buyer.txid)?,
                        to: buyer.owner.clone(),
                        amount_atoms: u128::from(refund),
                    },
                ));
            }
            self.ledger.apply_batch(legs)?;
            let orders = self.books.get_mut(&pair).expect("pair exists");
            orders[sell_index].remaining_offered -= base;
            orders[buy_index].remaining_offered -= quote;
            orders[buy_index].remaining_desired -= base;
            if buyer_done {
                orders[buy_index].remaining_offered = 0;
            }
            let mut removals = Vec::new();
            if orders[sell_index].remaining_offered == 0 {
                removals.push(sell_index);
            }
            if buyer_done {
                removals.push(buy_index);
            }
            removals.sort_unstable_by(|a, b| b.cmp(a));
            for index in removals {
                orders.remove(index);
            }
            self.fills.push(SpotFill {
                sell_txid: seller.txid,
                buy_txid: buyer.txid,
                pair,
                base_atoms: base,
                quote_atoms: quote,
                trigger_txid: trigger,
                at_height: height,
            });
        }
    }

    /// Type-6 token cancellation. A cancel applies to outstanding **posted**
    /// orders after their block has been processed; same-block ordering,
    /// cancel-by-price and contract cancels await a historical specification.
    pub fn cancel_type6_unverified(
        &mut self,
        request: UnverifiedSpotCancellation,
    ) -> Result<usize, SpotError> {
        if request.is_contract {
            return Err(SpotError::UnsupportedContractCancel);
        }
        let finalized = self.processed_through.ok_or(SpotError::InvalidHeight)?;
        if request.height <= finalized {
            return Err(SpotError::HeightRegression);
        }
        if request.actor.as_str().starts_with('~') {
            return Err(SpotError::ReservedAccount);
        }
        let pair = self.check_pair(request.offered, request.desired)?;
        if self.seen.contains(&request.txid) {
            return Err(SpotError::Replay(request.txid));
        }
        if self.seen.len() >= MAX_SPOT_INPUT_EVENTS {
            return Err(SpotError::CapacityExceeded);
        }
        let mut staged = self.clone();
        let orders = staged.books.entry(pair).or_default();
        if let CancelSelector::ByOrderTxId(id) = request.selector {
            if orders
                .iter()
                .any(|order| order.txid == id && order.owner != request.actor)
            {
                return Err(SpotError::UnauthorizedCancellation);
            }
        }
        let selected: Vec<_> = orders
            .iter()
            .filter(|order| {
                order.owner == request.actor
                    && order.offered == request.offered
                    && order.desired == request.desired
                    && match request.selector {
                        CancelSelector::All => true,
                        CancelSelector::ByOrderTxId(id) => order.txid == id,
                    }
            })
            .cloned()
            .collect();
        if selected.is_empty() {
            return Err(SpotError::NoMatchingOrder);
        }
        let operations = selected
            .iter()
            .enumerate()
            .map(|(index, order)| {
                Ok((
                    internal_txid(
                        request.txid,
                        b"cancel",
                        u32::try_from(index).map_err(|_| SpotError::CapacityExceeded)?,
                    ),
                    Operation::Transfer {
                        id: order.offered,
                        from: escrow_account(order.txid)?,
                        to: order.owner.clone(),
                        amount_atoms: u128::from(order.remaining_offered),
                    },
                ))
            })
            .collect::<Result<Vec<_>, SpotError>>()?;
        staged.ledger.apply_batch(operations)?;
        let ids: BTreeSet<TxId> = selected.iter().map(|order| order.txid).collect();
        staged
            .books
            .entry(pair)
            .or_default()
            .retain(|o| !ids.contains(&o.txid));
        staged.seen.insert(request.txid);
        staged.ledger.verify_conservation()?;
        staged.verify_reserves()?;
        *self = staged;
        Ok(selected.len())
    }

    /// Type 3 needs authenticated external-LTC payments, sender/recipient
    /// output semantics, channel column selection and fee accounting.
    pub fn type3_utxo_trade_unverified(&self) -> Result<(), SpotError> {
        Err(SpotError::AuthenticatedUtxoProofRequired)
    }

    /// Stage 25B: ordinary JointProtocolState/TLJP contains *only* spot
    /// escrow metadata. Refuse imported clearing margin balances lacking
    /// signed positions, ownership, loss policy and a combined checkpoint.
    pub(crate) fn reject_foreign_reserves_for_joint_unverified(&self) -> Result<(), SpotError> {
        if self.clearing_activity {
            return Err(SpotError::ReservedAccount);
        }
        if self
            .ledger
            .balances_with_account_prefix("~")
            .iter()
            .any(|(_, account, _)| !account.as_str().starts_with(ESCROW_PREFIX))
        {
            return Err(SpotError::ReservedAccount);
        }
        Ok(())
    }

    /// Canonical accounting invariant across all outstanding orders.
    pub fn verify_reserves(&self) -> Result<(), SpotError> {
        let mut expected = BTreeMap::new();
        for orders in self.books.values() {
            for order in orders {
                let escrow = escrow_account(order.txid)?;
                if order.remaining_offered == 0
                    || expected
                        .insert((order.offered, escrow), u128::from(order.remaining_offered))
                        .is_some()
                {
                    return Err(SpotError::ReserveMismatch);
                }
            }
        }
        for (id, account, atoms) in self.ledger.balances_with_account_prefix(ESCROW_PREFIX) {
            if expected.remove(&(id, account)) != Some(atoms) {
                return Err(SpotError::ReserveMismatch);
            }
        }
        if !expected.is_empty() {
            return Err(SpotError::ReserveMismatch);
        }
        Ok(())
    }
}

fn internal_txid(source: TxId, domain: &[u8], index: u32) -> TxId {
    let mut input = Vec::with_capacity(64 + domain.len());
    input.extend_from_slice(b"TL_SPOT_UNVERIFIED_V1:");
    input.extend_from_slice(domain);
    input.extend_from_slice(&source.as_bytes());
    input.extend_from_slice(&index.to_be_bytes());
    TxId::from_bytes(double_sha256(&input))
}

fn escrow_account(source: TxId) -> Result<AccountId, SpotError> {
    let mut value = String::from(ESCROW_PREFIX);
    for byte in source.as_bytes() {
        use std::fmt::Write;
        write!(value, "{byte:02x}").expect("string formatting");
    }
    AccountId::try_from(value.as_str()).map_err(SpotError::Ledger)
}

fn price_cmp(a: &RestingSpotOrder, b: &RestingSpotOrder) -> Ordering {
    let (an, ad) = a.quote_per_base();
    let (bn, bd) = b.quote_per_base();
    (u128::from(an) * u128::from(bd)).cmp(&(u128::from(bn) * u128::from(ad)))
}

fn priority_cmp(a: &RestingSpotOrder, b: &RestingSpotOrder) -> Ordering {
    a.height
        .cmp(&b.height)
        .then_with(|| a.sequence.cmp(&b.sequence))
        .then_with(|| a.txid.cmp(&b.txid))
}

fn suffix_key(account: &AccountId) -> [(u8, u8); 3] {
    let bytes = account.as_str().as_bytes();
    std::array::from_fn(|offset| match bytes.iter().rev().nth(offset) {
        Some(&value) => {
            let lowered = value.to_ascii_lowercase();
            (u8::from(lowered.is_ascii_digit()), lowered)
        }
        None => (2, 0),
    })
}

/// Matches the legacy ASCII sender suffix grouping: letters before digits,
/// then the last three characters, then the full address. Locale-sensitive
/// non-ASCII comparisons are deliberately excluded by AccountId.
pub fn compare_legacy_sender_suffix(a: &AccountId, b: &AccountId) -> Ordering {
    suffix_key(a)
        .cmp(&suffix_key(b))
        .then_with(|| a.as_str().cmp(b.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_priority_matches_js_ascii_order_for_three_tail_characters() {
        let a = |s| AccountId::try_from(s).unwrap();
        assert_eq!(
            compare_legacy_sender_suffix(&a("trader-aa"), &a("trader-a1")),
            Ordering::Less
        );
        assert_eq!(
            compare_legacy_sender_suffix(&a("account-a"), &a("account-b")),
            Ordering::Less
        );
        assert_eq!(
            compare_legacy_sender_suffix(&a("abc"), &a("ABC")),
            Ordering::Greater
        );
    }

    #[test]
    fn internal_escrow_and_ledger_ids_are_domain_separated() {
        let txid = TxId::from_bytes([1; 32]);
        assert_ne!(
            internal_txid(txid, b"reserve", 0),
            internal_txid(txid, b"cancel", 0)
        );
        assert_ne!(
            internal_txid(txid, b"fill-base", 0),
            internal_txid(txid, b"fill-quote", 0)
        );
        assert_ne!(
            escrow_account(txid).unwrap(),
            escrow_account(TxId::from_bytes([2; 32])).unwrap()
        );
    }
}

// Snapshot code is included in-module to retain private ledger/book invariants.
include!("spot_snapshot_impl.rs");
