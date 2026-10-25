//! Source-compatible deterministic subset of pinned legacy activation.js.
//!
//! Source: patrickdugan/tradelayer.js@dca0888b, src/activation.js:9-303.
//! This is a caller-supplied, UNVERIFIED projection, not a consensus gate.
//! The existing Rust registry::Registry deliberately uses stricter activation
//! policy (no reactivation, same-block guard, ID-34 semantic conflict).
//! This module reproduces the source activation registry's *data behavior*
//! separately; it NEVER enables Ledger, channels, clearing or settlement.
//!
//! Historical source discrepancies:
//! - Reactivating a known type replaces its activationBlock, preserving the
//!   old codeHash if the new optional codeHash is falsy/absent.
//! - isTxTypeActive returns the current active flag with no height argument.
//! - getActivationBlock returns null for a stored zero height, while
//!   checkActivationBlock returns zero if the field exists.
//! - more than 90 percent active means at least 33 of the 36 slots.
//! - type zero calls Vesting.initializeTokens, creates native property and
//!   insurance balances, then updates activation flags; we CANNOT port that
//!   side effect with a bare activation flag. Type zero ALWAYS denies here.
//! - source names at 31/32 differ from the actual legacy tx dispatcher.
//! - source JS persistence resets to default on malformed/missing database;
//!   this port does not accept such a reset as historical state proof.
//! - network/testnet genesis-admin addresses are labels; comparing actor
//!   strings is not a signature, script, UTXO or historical authorization.

use crate::network_profile::TradeLayerMainnet;

pub const LEGACY_ACTIVATION_TYPE_COUNT: usize = 36;
pub const MAX_ACTIVATION_BATCH: usize = 1024;
pub const MAX_CLAIMED_CODE_HASH_BYTES: usize = 256;

/// Exact labels from activation.js::initializeTxRegistry, NOT trusted
/// transaction semantics. The legacy tx dispatcher disagrees at IDs 31/32.
pub const LEGACY_ACTIVATION_NAMES: [&str; LEGACY_ACTIVATION_TYPE_COUNT] = [
    "Activate TradeLayer",
    "Token Issue",
    "Send",
    "Trade Token for UTXO",
    "Commit Token",
    "On-chain Token for Token",
    "cancelOrder",
    "Create Whitelist",
    "Update Admin",
    "Issue or Revoke Attestation",
    "AMM",
    "Grant Managed Token",
    "Redeem Managed Token",
    "Create Oracle",
    "Publish Oracle Data",
    "Close Oracle",
    "Create Future Contract Series",
    "Exercise Derivative",
    "Trade Contract On-chain",
    "Trade Contract Channel",
    "Trade Tokens Channel",
    "Withdrawal",
    "Transfer",
    "Settle Channel PNL",
    "Mint Synthetic",
    "Redeem Synthetic",
    "Pay to Tokens",
    "Create Option Chain",
    "Trade Bai Urbun",
    "Trade Murabaha",
    "Oracle Stake/Fraud/Relay",
    "Batch Move Zk Rollup",
    "Publish New Tx",
    "Colored Coin",
    "Cross Layer Bridge",
    "Smart Contract Bind",
];

pub const PINNED_GENESIS_ADMIN_LITECOIN_TESTNET: &str =
    "tltc1qa0kd2d39nmeph3hvcx8ytv65ztcywg5sazhtw8";

/// Literal result of pinned activation.js::updateAdminAddress, depending
/// on the chain and test flags returned by its RPC client. Not a permission.
pub const fn legacy_admin_address(network: TradeLayerMainnet, testnet: bool) -> &'static str {
    match (network, testnet) {
        (TradeLayerMainnet::Bitcoin, false) => "bc1qktknrnx2jcchjht9anz0uy8ae02xryxq2vxeem",
        (TradeLayerMainnet::Bitcoin, true) => "tb1q8f84erfegxhaylmvpfll9m5rgwymqy4akjnnvq",
        (TradeLayerMainnet::Litecoin, false) => "MTmoypkhRQoJ172ZqxcsVumPZfJ8KCrQCB",
        (TradeLayerMainnet::Litecoin, true) => PINNED_GENESIS_ADMIN_LITECOIN_TESTNET,
        (TradeLayerMainnet::Dogecoin, false) => "DLSfu9qvEggkeXAgCAwBBw5BVLvMCtkewz",
        (TradeLayerMainnet::Dogecoin, true) => "nop27JQWbGr95ySHXZMzCg8XXxYzbCBZAW",
    }
}

/// The source isSenderGenesisAdmin hardcodes Litecoin testnet, even if the
/// instance is running on another network. Keep that oddity observational.
pub fn legacy_is_sender_genesis_admin(address: &str) -> bool {
    address == PINNED_GENESIS_ADMIN_LITECOIN_TESTNET
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClaimedActivationPosition {
    pub height: u64,
    pub tx_index: u32,
    pub block_id: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimedLegacyActivation<'a> {
    pub network: TradeLayerMainnet,
    pub at: ClaimedActivationPosition,
    pub transaction_type: u8,
    /// Source validity compares sender to network admin; this is just a
    /// claimed STRING, not a signature or validated first-input provenance.
    pub claimed_sender: &'a str,
    /// Nonempty means JS truthy for a string; absent or empty preserves old.
    pub claimed_code_hash: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyActivationError {
    EmptyBatch,
    TooManyEvents,
    UnknownType(u8),
    WrongNetwork,
    WrongClaimedAdmin,
    InvalidCodeHash,
    MissingBlockId,
    OutOfOrderClaimedEvents,
    ConflictingClaimedBlockAtHeight,
    /// Type zero needs the source's native-token and fund initializer.
    UnverifiedGenesisTokenInitialization,
    /// No source JSON database, authenticated chain or admin witness.
    HistoricalActivationAuthorityUnavailable,
    /// A snapshot cannot authorize a settlement or economic transition.
    TrustedProtocolActivationUnavailable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnverifiedActivationSlot {
    active: bool,
    activation_block: Option<u64>,
    code_hash: Option<String>,
}

impl UnverifiedActivationSlot {
    pub const fn active(&self) -> bool {
        self.active
    }
    /// Mirrors JS getActivationBlock() which uses activationBlock || null.
    pub fn source_get_activation_block(&self) -> Option<u64> {
        self.activation_block.filter(|block| *block != 0)
    }
    /// Mirrors JS checkActivationBlock() which only tests undefined.
    pub const fn source_check_activation_block(&self) -> Option<u64> {
        self.activation_block
    }
    pub fn source_code_hash(&self) -> Option<&str> {
        self.code_hash.as_deref()
    }
}

/// Data-state projection. Fields are private; source-active does not mean
/// chain-accepted or authorized. No method can produce a trusted capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedLegacyActivationRegistry {
    network: TradeLayerMainnet,
    testnet: bool,
    slots: [UnverifiedActivationSlot; LEGACY_ACTIVATION_TYPE_COUNT],
    last_claimed: Option<ClaimedActivationPosition>,
    events_applied: u64,
}

impl UnverifiedLegacyActivationRegistry {
    pub fn new(network: TradeLayerMainnet, testnet: bool) -> Self {
        Self {
            network,
            testnet,
            slots: std::array::from_fn(|_| UnverifiedActivationSlot::default()),
            last_claimed: None,
            events_applied: 0,
        }
    }
    pub const fn network(&self) -> TradeLayerMainnet {
        self.network
    }
    pub const fn testnet(&self) -> bool {
        self.testnet
    }
    pub fn source_admin(&self) -> &'static str {
        legacy_admin_address(self.network, self.testnet)
    }
    pub fn slot(&self, transaction_type: u8) -> Option<&UnverifiedActivationSlot> {
        self.slots.get(usize::from(transaction_type))
    }
    pub fn name(transaction_type: u8) -> Option<&'static str> {
        LEGACY_ACTIVATION_NAMES
            .get(usize::from(transaction_type))
            .copied()
    }
    /// JS isTxTypeActive asks only for the current stored active field;
    /// there is NO temporal check against activationBlock in that accessor.
    pub fn source_is_tx_type_active(&self, transaction_type: u8) -> bool {
        self.slot(transaction_type)
            .is_some_and(UnverifiedActivationSlot::active)
    }
    pub fn source_check_activation_block(&self, transaction_type: u8) -> Option<u64> {
        self.slot(transaction_type)
            .and_then(UnverifiedActivationSlot::source_check_activation_block)
    }
    pub fn source_get_activation_block(&self, transaction_type: u8) -> Option<u64> {
        self.slot(transaction_type)
            .and_then(UnverifiedActivationSlot::source_get_activation_block)
    }
    pub fn source_code_hash(&self, transaction_type: u8) -> Option<&str> {
        self.slot(transaction_type)
            .and_then(UnverifiedActivationSlot::source_code_hash)
    }
    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.active).count()
    }
    /// The source computes percentage >90, not >=90. There are 36 slots.
    pub fn source_are_activations_above_threshold(&self) -> bool {
        self.active_count() * 10 > LEGACY_ACTIVATION_TYPE_COUNT * 9
    }
    pub const fn last_claimed(&self) -> Option<ClaimedActivationPosition> {
        self.last_claimed
    }
    pub const fn claimed_event_count(&self) -> u64 {
        self.events_applied
    }
    pub const fn require_authenticated_legacy_activation(
        &self,
    ) -> Result<(), LegacyActivationError> {
        Err(LegacyActivationError::HistoricalActivationAuthorityUnavailable)
    }
    pub const fn require_trusted_protocol_activation(&self) -> Result<(), LegacyActivationError> {
        Err(LegacyActivationError::TrustedProtocolActivationUnavailable)
    }

    /// All-or-nothing mutation of the unverified projection. This source
    /// adapter is STRICTER than JS in requiring nonzero claimed block IDs,
    /// chronological events and bounded ASCII hashes. These are not inferred
    /// consensus rules. The caller-supplied sender/block/tx index remains
    /// unauthenticated, so this method is never a trusted admission gate.
    pub fn apply_claimed_batch(
        &mut self,
        events: &[ClaimedLegacyActivation<'_>],
    ) -> Result<(), LegacyActivationError> {
        if events.is_empty() {
            return Err(LegacyActivationError::EmptyBatch);
        }
        if events.len() > MAX_ACTIVATION_BATCH {
            return Err(LegacyActivationError::TooManyEvents);
        }
        let mut next = self.clone();
        for event in events {
            if event.network != next.network {
                return Err(LegacyActivationError::WrongNetwork);
            }
            if event.at.block_id == [0; 32] {
                return Err(LegacyActivationError::MissingBlockId);
            }
            if let Some(previous) = next.last_claimed {
                if event.at.height == previous.height && event.at.block_id != previous.block_id {
                    return Err(LegacyActivationError::ConflictingClaimedBlockAtHeight);
                }
                if (event.at.height, event.at.tx_index) <= (previous.height, previous.tx_index) {
                    return Err(LegacyActivationError::OutOfOrderClaimedEvents);
                }
            }
            if event.transaction_type > 35 {
                return Err(LegacyActivationError::UnknownType(event.transaction_type));
            }
            // JS validateActivateTradeLayer rejects wrong sender, but here
            // equality is only a descriptive check of UNVERIFIED strings.
            if event.claimed_sender != next.source_admin() {
                return Err(LegacyActivationError::WrongClaimedAdmin);
            }
            // No code path may record native genesis without all initialized
            // property, insurance, registry and ledger effects being verified.
            if event.transaction_type == 0 {
                return Err(LegacyActivationError::UnverifiedGenesisTokenInitialization);
            }
            let new_code_hash = event.claimed_code_hash.filter(|s| !s.is_empty());
            if new_code_hash.is_some_and(|s| {
                s.len() > MAX_CLAIMED_CODE_HASH_BYTES
                    || !s.is_ascii()
                    || !s.bytes().all(|c| (0x20..=0x7e).contains(&c))
            }) {
                return Err(LegacyActivationError::InvalidCodeHash);
            }
            let slot = &mut next.slots[usize::from(event.transaction_type)];
            // Source activate() does NOT reject a previously active type.
            slot.active = true;
            slot.activation_block = Some(event.at.height);
            if let Some(hash) = new_code_hash {
                slot.code_hash = Some(hash.to_owned());
            }
            next.last_claimed = Some(event.at);
            next.events_applied += 1;
        }
        *self = next;
        Ok(())
    }
}
