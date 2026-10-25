//! Stage25P: source-CONDITIONAL type7/8/9 administrator projection.
//!
//! Pinned tradelayer.js@dca0888b: validity.js:881-1098;
//! clearlist.js:9-30,86-98,112-130,135-170,415-449,561-570;
//! logic.js:194-205,851-920; txDecoder.js:226-261.
//!
//! Every raw type7/8/9 candidate gets a *relative* Bitcoin txid Merkle
//! inclusion in the same caller-anchored, locally linked SHA256d segment.
//! A claimed first-input sender, type activation, initial list counter and
//! list-8 validation outcome are NOT authenticated by inclusion. The pinned
//! Type8 validator fetches the "attestations" collection through getList,
//! whereas create/update read/write "clearlists", and it expects different
//! administrator field names. Thus Type8 transitions are HYPOTHETICAL;
//! this module does NOT assert they were accepted by historical consensus.
//!
//! Source-like list creation allocates max(existing_ids)+1; this module
//! accepts an unverified explicit next-ID seed and requires the caller to
//! declare an exact new ID, exposing the allocation dependency.
//! Type9 requires the projected primary administrator (backup is insufficient)
//! and an existing directly attested member before revocation. It excludes
//! list-zero country/banlist, xpub-derived and merkle-root events.
//!
//! Never a TrustedChainAdapter, signature verifier or ledger permission.

use std::collections::BTreeSet;

use crate::bitcoin_inclusion::{
    CheckedHeaderSegment, IncludedTransaction, InclusionError, MerklePath,
};
use crate::channel_admission_audit::{
    AdmissionAuditError, ClaimedDirectAction, ClaimedDirectAttestation, ClaimedEventPosition,
    ClaimedHistoricalDirectView, HistoricalAdmissionKind, Type19ChannelListClaim,
    Type4CommitListClaim, UnverifiedHistoricalAdmissionSlice,
};
use crate::channel_type9_inclusion_audit::{
    inspect_raw_type9_candidate_unverified, Type9OriginError,
};
use crate::network_profile::TradeLayerMainnet;
use crate::read_only::{inspect_transaction, CarrierFormat, ScanError};
use crate::wire::{parse_base36, parse_frame, WireError};

pub const MAX_PROJECTED_789_EVENTS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectedAdminError {
    UnsupportedNetwork,
    TooManyEvents,
    BadInitialCounter,
    BadCarrier,
    UnsupportedEventType,
    BadEventFields,
    AmbiguousNestedEnvelope,
    BadClaimedSender,
    UntrustedTypeReportedInactive,
    WrongCreatedListId,
    UnexpectedCreatedListId,
    UnknownListId,
    UnverifiedSourceSenderNotCurrentAdmin,
    UnverifiedSourceSenderNotAdminOrBackup,
    DirectAttestationAlreadyRevoked,
    InvalidEventOrder,
    ClaimedPositionMismatch,
    DuplicateObservedTransaction,
    WrongTargetType,
    RelativeInclusion(InclusionError),
    Transaction(ScanError),
    Wire(WireError),
    Type9(Type9OriginError),
    DirectSubset(AdmissionAuditError),
    /// No event has independent sender/consensus/activation authentication.
    HistoricalSourceAuthorityUnavailable,
    /// getList reads attestation rows, not the clearlist used by updateAdmin.
    Type8ValidatorLookupUnresolved,
    CanonicalSignedRevisionUnspecified,
}

impl From<ScanError> for ProjectedAdminError {
    fn from(err: ScanError) -> Self {
        Self::Transaction(err)
    }
}
impl From<WireError> for ProjectedAdminError {
    fn from(err: WireError) -> Self {
        Self::Wire(err)
    }
}
impl From<AdmissionAuditError> for ProjectedAdminError {
    fn from(err: AdmissionAuditError) -> Self {
        Self::DirectSubset(err)
    }
}
impl From<Type9OriginError> for ProjectedAdminError {
    fn from(err: Type9OriginError) -> Self {
        Self::Type9(err)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedType789Event<'a> {
    pub at: ClaimedEventPosition,
    pub raw_transaction: &'a [u8],
    pub path: &'a MerklePath,
    /// NOT a verified sender: the historical source derives this from the
    /// first input's parent output script, whose UTXO is not supplied here.
    pub claimed_first_input_sender: &'a str,
    /// The historical activation registry is absent from this fixture.
    pub claimed_type_active: bool,
    /// Only source type7 carries this *external* expected allocated list ID.
    pub expected_new_list_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedClearlist {
    id: u32,
    primary_admin: String,
    backup: String,
    direct_members: BTreeSet<String>,
}
impl ProjectedClearlist {
    pub const fn id(&self) -> u32 {
        self.id
    }
    pub fn projected_primary_admin(&self) -> &str {
        &self.primary_admin
    }
    pub fn projected_backup(&self) -> &str {
        &self.backup
    }
    pub fn projected_direct_member(&self, address: &str) -> bool {
        self.direct_members.contains(address)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectEventRecord {
    at: ClaimedEventPosition,
    txid_raw: [u8; 32],
    list_id: u32,
    address: String,
    action: ClaimedDirectAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedProjectedAdminHistory {
    target: IncludedTransaction,
    observed_source_events: Vec<IncludedTransaction>,
    target_position: ClaimedEventPosition,
    target_kind: HistoricalAdmissionKind,
    next_unverified_list_id: u32,
    lists: Vec<ProjectedClearlist>,
    direct_events: Vec<DirectEventRecord>,
    type8_lookup_ambiguity_count: usize,
}
impl UnverifiedProjectedAdminHistory {
    pub const fn target(&self) -> IncludedTransaction {
        self.target
    }
    pub fn observed_source_event_count(&self) -> usize {
        self.observed_source_events.len()
    }
    pub fn relative_source_events(&self) -> &[IncludedTransaction] {
        &self.observed_source_events
    }
    pub const fn next_unverified_list_id(&self) -> u32 {
        self.next_unverified_list_id
    }
    pub fn projected_list(&self, list_id: u32) -> Option<&ProjectedClearlist> {
        self.lists.iter().find(|list| list.id == list_id)
    }
    pub const fn type8_lookup_ambiguity_count(&self) -> usize {
        self.type8_lookup_ambiguity_count
    }
    pub fn type4_projected_direct_subset(
        &self,
        claim: Type4CommitListClaim<'_>,
    ) -> Result<UnverifiedHistoricalAdmissionSlice, ProjectedAdminError> {
        if self.target_kind != HistoricalAdmissionKind::Type4Commit {
            return Err(ProjectedAdminError::WrongTargetType);
        }
        let records: Vec<_> = self
            .direct_events
            .iter()
            .map(|event| ClaimedDirectAttestation {
                network: TradeLayerMainnet::Bitcoin,
                at: event.at,
                txid_raw: event.txid_raw,
                clearlist_id: event.list_id,
                address: event.address.as_str(),
                action: event.action,
            })
            .collect();
        let view = ClaimedHistoricalDirectView::audit(
            TradeLayerMainnet::Bitcoin,
            self.target_position,
            &records,
        )?;
        Ok(view.type4_commit_lists(claim)?)
    }

    pub fn type19_projected_direct_subset(
        &self,
        claim: Type19ChannelListClaim<'_>,
    ) -> Result<UnverifiedHistoricalAdmissionSlice, ProjectedAdminError> {
        if self.target_kind != HistoricalAdmissionKind::Type19ChannelTrade {
            return Err(ProjectedAdminError::WrongTargetType);
        }
        let records: Vec<_> = self
            .direct_events
            .iter()
            .map(|event| ClaimedDirectAttestation {
                network: TradeLayerMainnet::Bitcoin,
                at: event.at,
                txid_raw: event.txid_raw,
                clearlist_id: event.list_id,
                address: event.address.as_str(),
                action: event.action,
            })
            .collect();
        let view = ClaimedHistoricalDirectView::audit(
            TradeLayerMainnet::Bitcoin,
            self.target_position,
            &records,
        )?;
        Ok(view.type19_channel_lists(claim)?)
    }
    pub const fn require_authenticated_historical_admin(&self) -> Result<(), ProjectedAdminError> {
        Err(ProjectedAdminError::HistoricalSourceAuthorityUnavailable)
    }
    pub const fn require_verified_type8_transition(&self) -> Result<(), ProjectedAdminError> {
        Err(ProjectedAdminError::Type8ValidatorLookupUnresolved)
    }
    pub const fn require_canonical_signed_channel_revision(
        &self,
    ) -> Result<(), ProjectedAdminError> {
        Err(ProjectedAdminError::CanonicalSignedRevisionUnspecified)
    }
}

enum ParsedEvent {
    Create { backup: String },
    Update { list_id: u32, new_admin: String, backup: bool },
    Direct { list_id: u32, address: String, action: ClaimedDirectAction },
}

fn source_name(raw: &str) -> Result<String, ProjectedAdminError> {
    if raw.is_empty()
        || raw.len() > 128
        || !raw.is_ascii()
        || raw.trim() != raw
        || raw.starts_with("ref:")
    {
        return Err(ProjectedAdminError::BadEventFields);
    }
    Ok(raw.to_owned())
}

fn id(raw: &str) -> Result<u32, ProjectedAdminError> {
    if raw.is_empty() || raw.len() > 1 && raw.starts_with('0') {
        return Err(ProjectedAdminError::BadEventFields);
    }
    let id = u32::try_from(parse_base36(raw)?)
        .map_err(|_| ProjectedAdminError::BadEventFields)?;
    if id == 0 {
        return Err(ProjectedAdminError::BadEventFields);
    }
    Ok(id)
}

fn parse_event(raw: &[u8]) -> Result<ParsedEvent, ProjectedAdminError> {
    let inspected = inspect_transaction(raw)?;
    if inspected.op_return_count != 1 || inspected.carriers.len() != 1 {
        return Err(ProjectedAdminError::BadCarrier);
    }
    let carrier = &inspected.carriers[0];
    if carrier.format != CarrierFormat::LegacyAscii || carrier.status != "opaque-type" {
        return Err(ProjectedAdminError::BadCarrier);
    }
    let payload = std::str::from_utf8(&carrier.payload)
        .map_err(|_| ProjectedAdminError::BadCarrier)?;
    let frame = parse_frame(payload)?;
    if frame.body.starts_with("tl") {
        return Err(ProjectedAdminError::AmbiguousNestedEnvelope);
    }
    match frame.tx_type.as_u8() {
        7 => {
            let parts: Vec<_> = frame.body.split(',').collect();
            if parts.len() != 4 || parts[1].is_empty() {
                return Err(ProjectedAdminError::BadEventFields);
            }
            // Source type7 has no ID; it is allocated from the DB max ID.
            // Name/url/description have no independently validated meaning.
            Ok(ParsedEvent::Create { backup: source_name(parts[0])? })
        }
        8 => {
            let parts: Vec<_> = frame.body.split(',').collect();
            if parts.len() != 6
                || parts[1..4] != ["1", "0", "0"]
                || !matches!(parts[5], "0" | "1")
            {
                return Err(ProjectedAdminError::BadEventFields);
            }
            Ok(ParsedEvent::Update {
                new_admin: source_name(parts[0])?,
                list_id: id(parts[4])?,
                backup: parts[5] == "1",
            })
        }
        9 => {
            let payload = inspect_raw_type9_candidate_unverified(raw)?;
            Ok(ParsedEvent::Direct {
                list_id: payload.clearlist_id(),
                address: source_name(payload.target())?,
                action: payload.action(),
            })
        }
        _ => Err(ProjectedAdminError::UnsupportedEventType),
    }
}

fn verify_position(
    included: IncludedTransaction,
    claimed: ClaimedEventPosition,
) -> Result<(), ProjectedAdminError> {
    if included.height() != u64::from(claimed.height)
        || included.tx_index() != claimed.tx_index
        || included.block_id().0 != claimed.block_hash
    {
        return Err(ProjectedAdminError::ClaimedPositionMismatch);
    }
    Ok(())
}

fn verify_target(
    raw: &[u8],
    kind: HistoricalAdmissionKind,
) -> Result<(), ProjectedAdminError> {
    let observed = inspect_transaction(raw)?;
    let selected_type = match kind {
        HistoricalAdmissionKind::Type4Commit => 4,
        HistoricalAdmissionKind::Type19ChannelTrade => 19,
    };
    if observed.op_return_count != 1 || observed.carriers.len() != 1 {
        return Err(ProjectedAdminError::WrongTargetType);
    }
    let carrier = &observed.carriers[0];
    if carrier.format != CarrierFormat::LegacyAscii
        || carrier.transaction_type != Some(selected_type)
        || carrier.status != "opaque-type"
    {
        return Err(ProjectedAdminError::WrongTargetType);
    }
    let value = std::str::from_utf8(&carrier.payload)
        .map_err(|_| ProjectedAdminError::WrongTargetType)?;
    if parse_frame(value)?.body.starts_with("tl") {
        return Err(ProjectedAdminError::AmbiguousNestedEnvelope);
    }
    Ok(())
}

/// Read-only conditional replay of caller-supplied *apparently* sourced
/// tx7/8/9 bytes. The type8 transition is a projected "if accepted" effect,
/// never a historical validation outcome or a trusted registry write.
#[allow(clippy::too_many_arguments)]
pub fn audit_bitcoin_projected_type789_history_unverified(
    network: TradeLayerMainnet,
    segment: &CheckedHeaderSegment,
    target_at: ClaimedEventPosition,
    raw_target: &[u8],
    target_path: &MerklePath,
    target_kind: HistoricalAdmissionKind,
    initial_next_list_id: u32,
    events: &[ClaimedType789Event<'_>],
) -> Result<UnverifiedProjectedAdminHistory, ProjectedAdminError> {
    if network != TradeLayerMainnet::Bitcoin {
        return Err(ProjectedAdminError::UnsupportedNetwork);
    }
    if events.len() > MAX_PROJECTED_789_EVENTS {
        return Err(ProjectedAdminError::TooManyEvents);
    }
    if initial_next_list_id == 0 {
        return Err(ProjectedAdminError::BadInitialCounter);
    }
    let target = segment
        .check_transaction_inclusion(u64::from(target_at.height), raw_target, target_path)
        .map_err(ProjectedAdminError::RelativeInclusion)?;
    verify_position(target, target_at)?;
    verify_target(raw_target, target_kind)?;
    let mut observed = Vec::with_capacity(events.len());
    let mut lists: Vec<ProjectedClearlist> = Vec::new();
    let mut direct_events = Vec::new();
    let mut seen_txids = BTreeSet::new();
    seen_txids.insert(target.txid_display());
    let mut next_id = initial_next_list_id;
    let mut previous: Option<ClaimedEventPosition> = None;
    let mut type8_ambiguities = 0;
    for event in events {
        let included = segment
            .check_transaction_inclusion(
                u64::from(event.at.height),
                event.raw_transaction,
                event.path,
            )
            .map_err(ProjectedAdminError::RelativeInclusion)?;
        verify_position(included, event.at)?;
        if (event.at.height, event.at.tx_index) >= (target_at.height, target_at.tx_index) {
            return Err(ProjectedAdminError::InvalidEventOrder);
        }
        if !seen_txids.insert(included.txid_display()) {
            return Err(ProjectedAdminError::DuplicateObservedTransaction);
        }
        if let Some(prior) = previous {
            if (event.at.height, event.at.tx_index) <= (prior.height, prior.tx_index) {
                return Err(ProjectedAdminError::InvalidEventOrder);
            }
        }
        previous = Some(event.at);
        if !event.claimed_type_active {
            return Err(ProjectedAdminError::UntrustedTypeReportedInactive);
        }
        let sender = source_name(event.claimed_first_input_sender)
            .map_err(|_| ProjectedAdminError::BadClaimedSender)?;
        let parsed = parse_event(event.raw_transaction)?;
        if !matches!(parsed, ParsedEvent::Create { .. }) && event.expected_new_list_id.is_some() {
            return Err(ProjectedAdminError::UnexpectedCreatedListId);
        }
        match parsed {
            ParsedEvent::Create { backup } => {
                if event.expected_new_list_id != Some(next_id) {
                    return Err(ProjectedAdminError::WrongCreatedListId);
                }
                lists.push(ProjectedClearlist {
                    id: next_id,
                    primary_admin: sender,
                    backup,
                    direct_members: BTreeSet::new(),
                });
                next_id = next_id.checked_add(1)
                    .ok_or(ProjectedAdminError::BadInitialCounter)?;
            }
            ParsedEvent::Update { list_id, new_admin, backup } => {
                let list = lists.iter_mut().find(|l| l.id == list_id)
                    .ok_or(ProjectedAdminError::UnknownListId)?;
                if sender != list.primary_admin && sender != list.backup {
                    return Err(ProjectedAdminError::UnverifiedSourceSenderNotAdminOrBackup);
                }
                // The JS update validator calls getList from attestations,
                // while create/update use clearlists, with different field
                // names. Conditional projection only; always flag ambiguity.
                type8_ambiguities += 1;
                if backup {
                    list.backup = new_admin;
                } else {
                    list.primary_admin = new_admin;
                }
            }
            ParsedEvent::Direct { list_id, address, action } => {
                let list = lists.iter_mut().find(|l| l.id == list_id)
                    .ok_or(ProjectedAdminError::UnknownListId)?;
                if sender != list.primary_admin {
                    return Err(ProjectedAdminError::UnverifiedSourceSenderNotCurrentAdmin);
                }
                match action {
                    ClaimedDirectAction::Attest => {
                        list.direct_members.insert(address.clone());
                    }
                    ClaimedDirectAction::Revoke => {
                        if !list.direct_members.remove(&address) {
                            return Err(ProjectedAdminError::DirectAttestationAlreadyRevoked);
                        }
                    }
                }
                let mut txid_raw = included.txid_display();
                txid_raw.reverse();
                direct_events.push(DirectEventRecord {
                    at: event.at,
                    txid_raw,
                    list_id,
                    address,
                    action,
                });
            }
        }
        observed.push(included);
    }
    Ok(UnverifiedProjectedAdminHistory {
        target,
        observed_source_events: observed,
        target_position: target_at,
        target_kind,
        next_unverified_list_id: next_id,
        lists,
        direct_events,
        type8_lookup_ambiguity_count: type8_ambiguities,
    })
}
