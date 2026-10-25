//! Stage25M: point-in-time *claimed* direct-clearlist replay for type 4/19.
//!
//! Pinned JS: tradelayer.js@dca0888b src/validity.js:473-653,1804-1930;
//! src/clearlist.js:147-170,224-275,415-449,561-570;
//! src/channels.js:800-912. This is a bounded CHARACTERIZATION of direct
//! memberships, not an independently proven historical source adapter.
//!
//! Type 4: exact committing SENDER must match at least one PROPERTY direct
//! whitelist unless property whitelistId was scalar zero; optional positive
//! commitClearlistId requires the sender directly or via derived eligibility.
//! This narrow audit requires a direct witness for both positive checks.
//! Type 19: union of both channel columns' declared clearLists, and EITHER
//! participant's direct membership suffices. The historical predicate also
//! supports xpub-derived membership, which this narrower audit does NOT
//! calculate or disprove.
//!
//! Callers supply unverified time-ordered direct attestation events and an
//! unverified block/transaction location. Checking a claimed sequence cannot
//! authenticate its tx inclusion, ancestry, state replay, historic activation,
//! banned-country policy, channel signer consent or canonical state revision.
//! The result cannot grant Ledger, clearing or consensus authorization.

use crate::network_profile::TradeLayerMainnet;

pub const MAX_CLAIMED_ATTESTATIONS: usize = 4096;
const MAX_ADDRESS_BYTES: usize = 128;
const MAX_LISTS: usize = 64;

/// Block ID and transaction position are CLAIMED, not best-chain evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimedEventPosition {
    pub height: u32,
    pub tx_index: u32,
    pub block_hash: [u8; 32],
}

impl ClaimedEventPosition {
    const fn key(self) -> (u32, u32) {
        (self.height, self.tx_index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimedDirectAction {
    Attest,
    Revoke,
}

/// Claims *direct* membership. Xpub/merkle-root derived eligibility is
/// intentionally not represented as proof of an address membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaimedDirectAttestation<'a> {
    pub network: TradeLayerMainnet,
    pub at: ClaimedEventPosition,
    pub txid_raw: [u8; 32],
    pub clearlist_id: u32,
    pub address: &'a str,
    pub action: ClaimedDirectAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionAuditError {
    UnsupportedNetwork,
    BadEventPosition,
    BadSubject,
    InvalidClaimedHistory,
    TooManyObservations,
    ForeignNetworkEvidence,
    FutureOrSameTransactionEvidence,
    OutOfOrderEvidence,
    DuplicateTransactionPosition,
    ConflictingBlockAtHeight,
    EmptyPropertyWhitelist,
    AmbiguousPropertyWhitelist,
    ForbiddenVestingProperty,
    NoDirectPropertyWitness,
    NoDirectCommitClearlistWitness,
    WrongChannelSender,
    IncompleteChannelParticipants,
    ExpiredChannelTrade,
    TooManyDeclaredLists,
    NoDirectChannelWitness,
    /// This narrow module has no verified chain, full historic policy or auth.
    IndependentAdmissionProofUnavailable,
    /// The pinned JS has no proven canonical signed-state revision transcript.
    CanonicalChannelRevisionUnspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyWhitelist<'a> {
    /// Explicit source scalar whitelistId = 0 only.
    UnrestrictedScalarZero,
    /// Source property whitelistId as one or more actual positive list IDs.
    AnyDirectList(&'a [u32]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Type4CommitListClaim<'a> {
    pub sender: &'a str,
    pub property_id: u32,
    pub property_whitelist: PropertyWhitelist<'a>,
    pub commit_clearlist_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Type19ChannelListClaim<'a> {
    pub claimed_sender_channel: &'a str,
    pub registered_channel: &'a str,
    pub participant_a: &'a str,
    pub participant_b: &'a str,
    pub column_a_lists: &'a [u32],
    pub column_b_lists: &'a [u32],
    pub expiry_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalAdmissionKind {
    Type4Commit,
    Type19ChannelTrade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectMatchSide {
    CommitSender,
    ChannelA,
    ChannelB,
}

/// This result is deliberately an *unverified* source-rule observation.
/// Neither a signed-state capability nor a verified channel can be constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedHistoricalAdmissionSlice {
    kind: HistoricalAdmissionKind,
    at: ClaimedEventPosition,
    matched_list_id: Option<u32>,
    matched_side: Option<DirectMatchSide>,
}

impl UnverifiedHistoricalAdmissionSlice {
    pub const fn kind(&self) -> HistoricalAdmissionKind {
        self.kind
    }
    pub const fn claimed_position(&self) -> ClaimedEventPosition {
        self.at
    }
    pub const fn matched_list_id(&self) -> Option<u32> {
        self.matched_list_id
    }
    pub const fn matched_side(&self) -> Option<DirectMatchSide> {
        self.matched_side
    }
    pub const fn require_authenticated_channel_admission(&self) -> Result<(), AdmissionAuditError> {
        Err(AdmissionAuditError::IndependentAdmissionProofUnavailable)
    }
    pub const fn require_canonical_signed_revision(&self) -> Result<(), AdmissionAuditError> {
        Err(AdmissionAuditError::CanonicalChannelRevisionUnspecified)
    }
}

fn check_name(address: &str) -> Result<(), AdmissionAuditError> {
    if address.is_empty()
        || address.len() > MAX_ADDRESS_BYTES
        || address.trim() != address
        || !address.is_ascii()
    {
        return Err(AdmissionAuditError::BadSubject);
    }
    Ok(())
}

/// All entries are caller-supplied. The strictly ordered history rejects
/// future membership, contradictory same-height block IDs and conflicting
/// events at the same transaction position. It does NOT establish ancestry.
#[derive(Debug, Clone, Copy)]
pub struct ClaimedHistoricalDirectView<'a> {
    network: TradeLayerMainnet,
    at: ClaimedEventPosition,
    entries: &'a [ClaimedDirectAttestation<'a>],
}

impl<'a> ClaimedHistoricalDirectView<'a> {
    pub fn audit(
        network: TradeLayerMainnet,
        at: ClaimedEventPosition,
        entries: &'a [ClaimedDirectAttestation<'a>],
    ) -> Result<Self, AdmissionAuditError> {
        if !matches!(
            network,
            TradeLayerMainnet::Bitcoin | TradeLayerMainnet::Litecoin
        ) {
            return Err(AdmissionAuditError::UnsupportedNetwork);
        }
        if at.height == 0 || at.block_hash == [0; 32] {
            return Err(AdmissionAuditError::BadEventPosition);
        }
        if entries.len() > MAX_CLAIMED_ATTESTATIONS {
            return Err(AdmissionAuditError::TooManyObservations);
        }

        let mut last: Option<ClaimedEventPosition> = None;
        for entry in entries {
            if entry.network != network {
                return Err(AdmissionAuditError::ForeignNetworkEvidence);
            }
            if entry.at.height == 0 || entry.at.block_hash == [0; 32] || entry.txid_raw == [0; 32] {
                return Err(AdmissionAuditError::BadEventPosition);
            }
            check_name(entry.address)?;
            if entry.at.key() >= at.key() {
                return Err(AdmissionAuditError::FutureOrSameTransactionEvidence);
            }
            if entry.at.height == at.height && entry.at.block_hash != at.block_hash {
                return Err(AdmissionAuditError::ConflictingBlockAtHeight);
            }
            if let Some(previous) = last {
                if entry.at.height == previous.height && entry.at.block_hash != previous.block_hash
                {
                    return Err(AdmissionAuditError::ConflictingBlockAtHeight);
                }
                if entry.at.key() == previous.key() {
                    return Err(AdmissionAuditError::DuplicateTransactionPosition);
                }
                if entry.at.key() < previous.key() {
                    return Err(AdmissionAuditError::OutOfOrderEvidence);
                }
            }
            last = Some(entry.at);
        }
        Ok(Self {
            network,
            at,
            entries,
        })
    }

    pub const fn claimed_position(&self) -> ClaimedEventPosition {
        self.at
    }

    pub const fn claimed_network(&self) -> TradeLayerMainnet {
        self.network
    }

    fn direct_member(&self, list_id: u32, address: &str) -> bool {
        let mut active = false;
        for event in self.entries {
            if event.clearlist_id == list_id && event.address == address {
                active = event.action == ClaimedDirectAction::Attest;
            }
        }
        active
    }

    /// Characterize the historical direct-membership portion of type-4.
    /// Type activation, property metadata, channel binding, geolocation,
    /// spend proof and economic state are independent missing verifiers.
    pub fn type4_commit_lists(
        &self,
        claim: Type4CommitListClaim<'_>,
    ) -> Result<UnverifiedHistoricalAdmissionSlice, AdmissionAuditError> {
        check_name(claim.sender)?;
        if claim.property_id == 0 || matches!(claim.property_id, 2 | 3) {
            return Err(AdmissionAuditError::ForbiddenVestingProperty);
        }
        let mut matched_list_id = None;
        match claim.property_whitelist {
            PropertyWhitelist::UnrestrictedScalarZero => {}
            PropertyWhitelist::AnyDirectList(ids) => {
                if ids.is_empty() || ids.len() > MAX_LISTS {
                    return Err(AdmissionAuditError::EmptyPropertyWhitelist);
                }
                // Do not port JavaScript loose-equality coercions for arrays
                // containing 0 into a supposed versioned consensus rule.
                if ids.contains(&0) {
                    return Err(AdmissionAuditError::AmbiguousPropertyWhitelist);
                }
                matched_list_id = ids
                    .iter()
                    .copied()
                    .find(|id| self.direct_member(*id, claim.sender));
                if matched_list_id.is_none() {
                    return Err(AdmissionAuditError::NoDirectPropertyWitness);
                }
            }
        }
        if let Some(list_id) = claim.commit_clearlist_id {
            if list_id > 0 {
                if !self.direct_member(list_id, claim.sender) {
                    // Source allows derived proof; lack of a direct witness is
                    // an evidence gap, NOT a historical rejection verdict.
                    return Err(AdmissionAuditError::NoDirectCommitClearlistWitness);
                }
                // Retain the more restrictive positive optional check when
                // both property and commit restrictions are present.
                matched_list_id = Some(list_id);
            }
        }
        Ok(UnverifiedHistoricalAdmissionSlice {
            kind: HistoricalAdmissionKind::Type4Commit,
            at: self.at,
            matched_list_id,
            matched_side: matched_list_id.map(|_| DirectMatchSide::CommitSender),
        })
    }

    /// Characterize the historical channel-list OR rule, never the entire
    /// type-19 validator: its price, reference, expiry, margin, country,
    /// activation and channel state checks still require source adapters.
    pub fn type19_channel_lists(
        &self,
        claim: Type19ChannelListClaim<'_>,
    ) -> Result<UnverifiedHistoricalAdmissionSlice, AdmissionAuditError> {
        for address in [
            claim.claimed_sender_channel,
            claim.registered_channel,
            claim.participant_a,
            claim.participant_b,
        ] {
            check_name(address)?;
        }
        if claim.claimed_sender_channel != claim.registered_channel {
            return Err(AdmissionAuditError::WrongChannelSender);
        }
        if claim.participant_a == claim.participant_b {
            return Err(AdmissionAuditError::IncompleteChannelParticipants);
        }
        if self.at.height > claim.expiry_height {
            return Err(AdmissionAuditError::ExpiredChannelTrade);
        }
        if claim.column_a_lists.len() + claim.column_b_lists.len() > MAX_LISTS {
            return Err(AdmissionAuditError::TooManyDeclaredLists);
        }
        let mut matched = None;
        // Pinned type19 source: union of declared lists, EITHER side.
        for id in claim.column_a_lists.iter().chain(claim.column_b_lists) {
            if self.direct_member(*id, claim.participant_a) {
                matched = Some((*id, DirectMatchSide::ChannelA));
                break;
            }
            if self.direct_member(*id, claim.participant_b) {
                matched = Some((*id, DirectMatchSide::ChannelB));
                break;
            }
        }
        if matched.is_none()
            && (!claim.column_a_lists.is_empty() || !claim.column_b_lists.is_empty())
        {
            // A derived xpub might still satisfy the real historical rule.
            return Err(AdmissionAuditError::NoDirectChannelWitness);
        }
        Ok(UnverifiedHistoricalAdmissionSlice {
            kind: HistoricalAdmissionKind::Type19ChannelTrade,
            at: self.at,
            matched_list_id: matched.map(|(id, _)| id),
            matched_side: matched.map(|(_, side)| side),
        })
    }
}
