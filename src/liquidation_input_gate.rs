//! Stage 25G: source-bound, read-only price/channel evidence preflight.
//!
//! The pinned historical JavaScript distinguishes a CURRENT-BLOCK oracle
//! publication from a CURRENT-BLOCK native volume-index price. Funding uses
//! distinct 192-block TWAP/VWAP references; neither funds a liquidation mark.
//! Source: tradelayer.js@dca0888b, clearing.js:1081–1212, 580–633;
//! validity.js:473–653, 1804–1960; channels.js:754–792.
//!
//! IMPORTANT: everything supplied to this module is a CALLER ASSERTION.
//! Correct structural fields DO NOT authenticate a Bitcoin/Litecoin/Dogecoin
//! block, oracle admin, historical clearlist, 2-of-2 channel signatures,
//! AMM depth, maker order or bankruptcy/ADL policy. There is deliberately no
//! AuthenticatedLiquidationObservation type, issuer, verifier or executable
//! settlement entry point. Every structurally consistent result remains
//! UNVERIFIED and cannot authorize type 3/23/31, E2B or Stage 25F transfers.

use crate::ledger::{AccountId, PropertyId, TxId};
use crate::network_profile::TradeLayerMainnet;
use std::collections::BTreeSet;

pub const MAX_UNVERIFIED_MARK_ROWS: usize = 128;
pub const MAX_UNVERIFIED_CLEARLIST_IDS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractMarkSource {
    Oracle { oracle_id: u32 },
    NativeVolumeIndex { notional_property: PropertyId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractMarkExpectation {
    pub network: TradeLayerMainnet,
    pub contract_id: u32,
    pub collateral_property: PropertyId,
    pub source: ContractMarkSource,
    pub target_height: u64,
    /// Previous known mark is *unverified* data, not an authenticated tip.
    pub previous_mark: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertedPriceRoute {
    OraclePublication {
        oracle_id: u32,
        publication_txid: TxId,
    },
    NativeVolumeIndex {
        notional_property: PropertyId,
        collateral_property: PropertyId,
    },
    /// The two 192-block funding references are explicitly not fresh marks.
    OracleFundingTwap { oracle_id: u32 },
    NativeFundingVwap {
        notional_property: PropertyId,
        collateral_property: PropertyId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssertedPriceRecord {
    pub network: TradeLayerMainnet,
    pub contract_id: u32,
    pub height: u64,
    pub price_e8: u64,
    pub route: AssertedPriceRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceGateError {
    InvalidContractExpectation,
    InvalidPreviousMark,
    TooManyPriceRecords,
    WrongNetwork,
    WrongContract,
    FutureRecord,
    MissingCurrentBlockMark,
    DuplicateCurrentBlockMark,
    WrongPriceRoute,
    InvalidOraclePublicationId,
    InvalidPrice,
    UnchangedMark,
    InvalidChannelIdentity,
    WrongChannelHeight,
    ExpiredChannelUpdate,
    TooManyClearlistAssertions,
    DuplicateClearlistAssertion,
    MissingChannelStateReference,
    AuthenticationUnavailable,
}

/// A *structurally* matching, unverified candidate with no financial
/// authority. Private fields prevent callers from silently treating its
/// contents as a validated input to the clearing engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnverifiedFreshMark {
    network: TradeLayerMainnet,
    contract_id: u32,
    height: u64,
    price_e8: u64,
    route: AssertedPriceRoute,
}

impl UnverifiedFreshMark {
    pub const fn network(self) -> TradeLayerMainnet {
        self.network
    }
    pub const fn contract_id(self) -> u32 {
        self.contract_id
    }
    pub const fn height(self) -> u64 {
        self.height
    }
    pub const fn price_e8(self) -> u64 {
        self.price_e8
    }
    pub const fn asserted_route(self) -> AssertedPriceRoute {
        self.route
    }
}

/// Strict current-block price *characterization*. Historical JS can choose
/// the first row of an ambiguous Oracle DB query; that is not a safe
/// deterministic consensus rule. Multiple current-height claims reject.
/// Older entries may be present for cached history, but cannot become a new
/// mark even if they report a plausible latest price.
pub fn audit_current_block_mark_unverified(
    expectation: ContractMarkExpectation,
    records: &[AssertedPriceRecord],
) -> Result<UnverifiedFreshMark, SourceGateError> {
    if expectation.contract_id == 0 || expectation.target_height == 0 {
        return Err(SourceGateError::InvalidContractExpectation);
    }
    if matches!(expectation.source, ContractMarkSource::Oracle { oracle_id: 0 }) {
        return Err(SourceGateError::InvalidContractExpectation);
    }
    if expectation
        .previous_mark
        .is_some_and(|(height, price)| price == 0 || height >= expectation.target_height)
    {
        return Err(SourceGateError::InvalidPreviousMark);
    }
    if records.len() > MAX_UNVERIFIED_MARK_ROWS {
        return Err(SourceGateError::TooManyPriceRecords);
    }
    let mut selected = None;
    for record in records {
        if record.network != expectation.network {
            return Err(SourceGateError::WrongNetwork);
        }
        if record.contract_id != expectation.contract_id {
            return Err(SourceGateError::WrongContract);
        }
        if record.height > expectation.target_height {
            return Err(SourceGateError::FutureRecord);
        }
        if record.height != expectation.target_height {
            continue;
        }
        if selected.is_some() {
            return Err(SourceGateError::DuplicateCurrentBlockMark);
        }
        if record.price_e8 == 0 {
            return Err(SourceGateError::InvalidPrice);
        }
        match (expectation.source, record.route) {
            (
                ContractMarkSource::Oracle { oracle_id: expected },
                AssertedPriceRoute::OraclePublication {
                    oracle_id: actual,
                    publication_txid,
                },
            ) if expected == actual => {
                if publication_txid == TxId::from_bytes([0; 32]) {
                    return Err(SourceGateError::InvalidOraclePublicationId);
                }
            }
            (
                ContractMarkSource::NativeVolumeIndex {
                    notional_property: expected,
                },
                AssertedPriceRoute::NativeVolumeIndex {
                    notional_property: actual,
                    collateral_property,
                },
            ) if expected == actual && collateral_property == expectation.collateral_property => {}
            _ => return Err(SourceGateError::WrongPriceRoute),
        }
        selected = Some(*record);
    }
    let record = selected.ok_or(SourceGateError::MissingCurrentBlockMark)?;
    if expectation
        .previous_mark
        .is_some_and(|(_, price)| price == record.price_e8)
    {
        // Pinned JS returns updated=false for an unchanged latest mark.
        return Err(SourceGateError::UnchangedMark);
    }
    Ok(UnverifiedFreshMark {
        network: expectation.network,
        contract_id: expectation.contract_id,
        height: expectation.target_height,
        price_e8: record.price_e8,
        route: record.route,
    })
}

/// Type-4/type-19/channel source-envelope CLAIMS only. Neither the
/// commitment pubkeys nor the signatures and redeem/witness script are
/// available to this validator. Clearlist IDs are assertions about policy,
/// NOT proof of historical attestation at the commitment's block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertedChannelUpdate {
    pub network: TradeLayerMainnet,
    pub contract_id: u32,
    pub update_height: u64,
    pub expires_at_height: u64,
    pub channel: AccountId,
    pub participant_a: AccountId,
    pub participant_b: AccountId,
    pub claimed_state_reference: [u8; 32],
    pub declared_clearlist_ids: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd)]
pub enum MissingIndependentProof {
    AuthenticatedNetworkBlockAndTransactionInclusion,
    HistoricalOracleAdminOrNativeVolumeIndex,
    HistoricalCommitAndClearlistState,
    GenuineTwoOfTwoSpendOrSignedChannelUpdate,
    ExecutedAmmFirstOrMakerFill,
    RatifiedLiquidationRoundingWaterfallAndActivation,
}

pub const ALL_MISSING_PROOFS: [MissingIndependentProof; 6] = [
    MissingIndependentProof::AuthenticatedNetworkBlockAndTransactionInclusion,
    MissingIndependentProof::HistoricalOracleAdminOrNativeVolumeIndex,
    MissingIndependentProof::HistoricalCommitAndClearlistState,
    MissingIndependentProof::GenuineTwoOfTwoSpendOrSignedChannelUpdate,
    MissingIndependentProof::ExecutedAmmFirstOrMakerFill,
    MissingIndependentProof::RatifiedLiquidationRoundingWaterfallAndActivation,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedLiquidationClaim {
    mark: UnverifiedFreshMark,
    channel: AssertedChannelUpdate,
}

impl UnverifiedLiquidationClaim {
    pub const fn mark(&self) -> UnverifiedFreshMark {
        self.mark
    }
    pub fn channel(&self) -> &AssertedChannelUpdate {
        &self.channel
    }
    pub const fn missing_proofs(&self) -> &[MissingIndependentProof; 6] {
        &ALL_MISSING_PROOFS
    }
    /// No proof validator exists in this module. Both a plausible synthetic
    /// claim and an actual real-world document are denied until separate
    /// independently verified L1/channel/market/activation evidence exists.
    pub const fn require_authenticated_liquidation(&self) -> Result<(), SourceGateError> {
        Err(SourceGateError::AuthenticationUnavailable)
    }
}

pub fn audit_liquidation_claim_unverified(
    mark: UnverifiedFreshMark,
    claim: AssertedChannelUpdate,
) -> Result<UnverifiedLiquidationClaim, SourceGateError> {
    if claim.network != mark.network {
        return Err(SourceGateError::WrongNetwork);
    }
    if claim.contract_id != mark.contract_id {
        return Err(SourceGateError::WrongContract);
    }
    if claim.update_height != mark.height {
        return Err(SourceGateError::WrongChannelHeight);
    }
    if claim.expires_at_height < claim.update_height {
        return Err(SourceGateError::ExpiredChannelUpdate);
    }
    if claim.channel.as_str().starts_with('~')
        || claim.participant_a.as_str().starts_with('~')
        || claim.participant_b.as_str().starts_with('~')
        || claim.participant_a == claim.participant_b
        || claim.channel == claim.participant_a
        || claim.channel == claim.participant_b
    {
        return Err(SourceGateError::InvalidChannelIdentity);
    }
    if claim.claimed_state_reference == [0; 32] {
        return Err(SourceGateError::MissingChannelStateReference);
    }
    if claim.declared_clearlist_ids.len() > MAX_UNVERIFIED_CLEARLIST_IDS {
        return Err(SourceGateError::TooManyClearlistAssertions);
    }
    let mut ids = BTreeSet::new();
    if claim
        .declared_clearlist_ids
        .iter()
        .any(|id| !ids.insert(*id))
    {
        return Err(SourceGateError::DuplicateClearlistAssertion);
    }
    // Deliberately never accepts a booleans-only "admin verified",
    // "whitelisted", "2-of-2 signed" or "AMM filled" assertion.
    Ok(UnverifiedLiquidationClaim {
        mark,
        channel: claim,
    })
}
