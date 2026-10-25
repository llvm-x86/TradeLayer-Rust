//! Stage25N: Bitcoin-only relative raw type-9 attestation origin evidence.
//! Pinned tradelayer.js@dca0888b: types.js:25-61,248-269;
//! txEncoder.js:220-233, txDecoder.js:250-260, validity.js:1003-1098,
//! logic.js:889-921 and clearlist.js:147-189,415-449.
//!
//! The raw OP_RETURN type-9 candidate is bound to a serialized transaction,
//! checked txid-Merkle proof and source-like direct attestation claim.
//! Caller-supplied anchor, source sender/admin, script execution, history
//! completeness, activation, best-chain, UTXO and signed-state revision
//! remain UNVERIFIED. The source may double-prefix the type-9 wire frame.
//! Such ambiguity is refused. No trusted economic capabilities can result.

use crate::bitcoin_inclusion::{
    CheckedHeaderSegment, IncludedTransaction, InclusionError, MerklePath,
};
use crate::channel_admission_audit::{
    AdmissionAuditError, ClaimedDirectAction, ClaimedDirectAttestation, ClaimedEventPosition,
    ClaimedHistoricalDirectView, HistoricalAdmissionKind, Type19ChannelListClaim,
    Type4CommitListClaim, UnverifiedHistoricalAdmissionSlice, MAX_CLAIMED_ATTESTATIONS,
};
use crate::network_profile::TradeLayerMainnet;
use crate::read_only::{inspect_transaction, CarrierFormat, ScanError};
use crate::wire::{parse_base36, parse_frame, WireError};
use std::collections::BTreeSet;

pub const MAX_RELATIVE_TYPE9_PROOFS: usize = 64;
const MAX_TARGET_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type9OriginError {
    UnsupportedNetwork,
    TooManyEvents,
    BadCarrier,
    NotType9,
    NotTargetType,
    AmbiguousNestedEnvelope,
    InvalidType9Fields,
    ExcludedDerivedOrCountryEvent,
    EventSemanticMismatch,
    ConflictingClaimedLocation,
    DuplicateIncludedTransaction,
    WrongAdmissionKind,
    HeaderOrMerkle(InclusionError),
    Transaction(ScanError),
    Wire(WireError),
    HistoricalClaim(AdmissionAuditError),
    OriginIsNotAuthorization,
    CanonicalRevisionUnspecified,
}

impl From<ScanError> for Type9OriginError {
    fn from(value: ScanError) -> Self {
        Self::Transaction(value)
    }
}
impl From<WireError> for Type9OriginError {
    fn from(value: WireError) -> Self {
        Self::Wire(value)
    }
}
impl From<AdmissionAuditError> for Type9OriginError {
    fn from(value: AdmissionAuditError) -> Self {
        Self::HistoricalClaim(value)
    }
}

/// Source-candidate fields extracted from raw transaction bytes, still
/// lacking signed admin, type-9 activation and replay-state verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedType9Payload {
    action: ClaimedDirectAction,
    clearlist_id: u32,
    target: String,
    metadata: String,
}
impl UnverifiedType9Payload {
    pub const fn action(&self) -> ClaimedDirectAction {
        self.action
    }
    pub const fn clearlist_id(&self) -> u32 {
        self.clearlist_id
    }
    pub fn target(&self) -> &str {
        &self.target
    }
    pub fn metadata(&self) -> &str {
        &self.metadata
    }
    pub const fn require_authorized_attestation(&self) -> Result<(), Type9OriginError> {
        Err(Type9OriginError::OriginIsNotAuthorization)
    }
}

/// Strict one-carrier/one-push, single-prefix source subset. Full historic
/// JavaScript encoding is inconsistent because two layers can prefix tl9.
pub fn inspect_raw_type9_candidate_unverified(
    raw_transaction: &[u8],
) -> Result<UnverifiedType9Payload, Type9OriginError> {
    let observation = inspect_transaction(raw_transaction)?;
    if observation.op_return_count != 1 || observation.carriers.len() != 1 {
        return Err(Type9OriginError::BadCarrier);
    }
    let carrier = &observation.carriers[0];
    if carrier.format != CarrierFormat::LegacyAscii {
        return Err(Type9OriginError::BadCarrier);
    }
    let payload =
        std::str::from_utf8(&carrier.payload).map_err(|_| Type9OriginError::BadCarrier)?;
    let frame = parse_frame(payload)?;
    if frame.tx_type.as_u8() != 9 {
        return Err(Type9OriginError::NotType9);
    }
    if frame.body.starts_with("tl9") {
        return Err(Type9OriginError::AmbiguousNestedEnvelope);
    }
    if carrier.status != "opaque-type" {
        return Err(Type9OriginError::BadCarrier);
    }
    let fields: Vec<&str> = frame.body.split(',').collect();
    if fields.len() != 4 {
        return Err(Type9OriginError::InvalidType9Fields);
    }
    let action = match fields[0] {
        "0" => ClaimedDirectAction::Attest,
        "1" => ClaimedDirectAction::Revoke,
        _ => return Err(Type9OriginError::InvalidType9Fields),
    };
    let digits = fields[1];
    if digits.is_empty() || (digits.len() > 1 && digits.starts_with('0')) {
        return Err(Type9OriginError::InvalidType9Fields);
    }
    let clearlist_id = u32::try_from(parse_base36(digits)?)
        .map_err(|_| Type9OriginError::InvalidType9Fields)?;
    let target = fields[2];
    let metadata = fields[3];
    if target.is_empty()
        || target.len() > MAX_TARGET_BYTES
        || !target.is_ascii()
        || target.trim() != target
        || metadata.is_empty()
    {
        return Err(Type9OriginError::InvalidType9Fields);
    }
    // List 0 is country self-cert/banlist, not ordinary direct membership.
    // Xpub, merkle-root and ref-target interpretation require their own
    // full-source versioned authorization and cannot pass this direct subset.
    if clearlist_id == 0
        || target.eq_ignore_ascii_case("BANLIST")
        || target.starts_with("ref:")
        || metadata.to_ascii_lowercase().starts_with("xpub:")
    {
        return Err(Type9OriginError::ExcludedDerivedOrCountryEvent);
    }
    Ok(UnverifiedType9Payload {
        action,
        clearlist_id,
        target: target.to_owned(),
        metadata: metadata.to_owned(),
    })
}

#[derive(Debug, Clone, Copy)]
pub struct ClaimedIncludedType9<'a> {
    pub claim: ClaimedDirectAttestation<'a>,
    pub raw_transaction: &'a [u8],
    pub merkle_path: &'a MerklePath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedBitcoinType9History<'a> {
    target: IncludedTransaction,
    observed_events: Vec<IncludedTransaction>,
    claimed_events: Vec<ClaimedDirectAttestation<'a>>,
    target_position: ClaimedEventPosition,
    kind: HistoricalAdmissionKind,
}

impl UnverifiedBitcoinType9History<'_> {
    pub const fn target(&self) -> IncludedTransaction {
        self.target
    }
    pub fn included_event_count(&self) -> usize {
        self.observed_events.len()
    }
    pub fn relative_events(&self) -> &[IncludedTransaction] {
        &self.observed_events
    }
    fn stage25m_view(&self) -> Result<ClaimedHistoricalDirectView<'_>, Type9OriginError> {
        Ok(ClaimedHistoricalDirectView::audit(
            TradeLayerMainnet::Bitcoin,
            self.target_position,
            &self.claimed_events,
        )?)
    }
    pub fn type4_direct_subset(
        &self,
        claim: Type4CommitListClaim<'_>,
    ) -> Result<UnverifiedHistoricalAdmissionSlice, Type9OriginError> {
        if self.kind != HistoricalAdmissionKind::Type4Commit {
            return Err(Type9OriginError::WrongAdmissionKind);
        }
        Ok(self.stage25m_view()?.type4_commit_lists(claim)?)
    }
    pub fn type19_direct_subset(
        &self,
        claim: Type19ChannelListClaim<'_>,
    ) -> Result<UnverifiedHistoricalAdmissionSlice, Type9OriginError> {
        if self.kind != HistoricalAdmissionKind::Type19ChannelTrade {
            return Err(Type9OriginError::WrongAdmissionKind);
        }
        Ok(self.stage25m_view()?.type19_channel_lists(claim)?)
    }
    pub const fn require_authenticated_admission(&self) -> Result<(), Type9OriginError> {
        Err(Type9OriginError::OriginIsNotAuthorization)
    }
    pub const fn require_canonical_signed_revision(&self) -> Result<(), Type9OriginError> {
        Err(Type9OriginError::CanonicalRevisionUnspecified)
    }
}

fn check_position(
    inclusion: IncludedTransaction,
    claimed: ClaimedEventPosition,
    raw_expected_txid: Option<[u8; 32]>,
) -> Result<(), Type9OriginError> {
    let mut raw_txid = inclusion.txid_display();
    raw_txid.reverse();
    if inclusion.height() != u64::from(claimed.height)
        || inclusion.tx_index() != claimed.tx_index
        || inclusion.block_id().0 != claimed.block_hash
        || raw_expected_txid.is_some_and(|id| id != raw_txid)
    {
        return Err(Type9OriginError::ConflictingClaimedLocation);
    }
    Ok(())
}

fn inspect_target_kind(
    raw: &[u8],
    expected_kind: HistoricalAdmissionKind,
) -> Result<(), Type9OriginError> {
    let inspected = inspect_transaction(raw)?;
    if inspected.op_return_count != 1 || inspected.carriers.len() != 1 {
        return Err(Type9OriginError::BadCarrier);
    }
    let carrier = &inspected.carriers[0];
    let expected_type = match expected_kind {
        HistoricalAdmissionKind::Type4Commit => 4,
        HistoricalAdmissionKind::Type19ChannelTrade => 19,
    };
    if carrier.format != CarrierFormat::LegacyAscii
        || carrier.transaction_type != Some(expected_type)
        || carrier.status != "opaque-type"
    {
        return Err(Type9OriginError::NotTargetType);
    }
    // Target carrier TYPE is only a selector. The economic body, claimed
    // property/channel, sender and state are not decoded or authenticated.
    let frame = parse_frame(
        std::str::from_utf8(&carrier.payload).map_err(|_| Type9OriginError::BadCarrier)?,
    )?;
    if frame.body.starts_with("tl4") || frame.body.starts_with("tlj") {
        return Err(Type9OriginError::AmbiguousNestedEnvelope);
    }
    Ok(())
}

/// Relative Bitcoin txid inclusion + exact raw event claims + Stage25M
/// direct-list source semantics. Network-specific Litecoin Scrypt and
/// Dogecoin AuxPoW cannot use this SHA256d-header evidence adapter.
pub fn audit_bitcoin_type9_relative_history_unverified<'a>(
    network: TradeLayerMainnet,
    segment: &CheckedHeaderSegment,
    target_position: ClaimedEventPosition,
    raw_target_transaction: &[u8],
    target_path: &MerklePath,
    kind: HistoricalAdmissionKind,
    events: &[ClaimedIncludedType9<'a>],
) -> Result<UnverifiedBitcoinType9History<'a>, Type9OriginError> {
    if network != TradeLayerMainnet::Bitcoin {
        return Err(Type9OriginError::UnsupportedNetwork);
    }
    if events.len() > MAX_RELATIVE_TYPE9_PROOFS || events.len() > MAX_CLAIMED_ATTESTATIONS {
        return Err(Type9OriginError::TooManyEvents);
    }
    let target = segment
        .check_transaction_inclusion(
            u64::from(target_position.height),
            raw_target_transaction,
            target_path,
        )
        .map_err(Type9OriginError::HeaderOrMerkle)?;
    check_position(target, target_position, None)?;
    inspect_target_kind(raw_target_transaction, kind)?;

    let mut seen_txids = BTreeSet::new();
    let mut target_raw = target.txid_display();
    target_raw.reverse();
    seen_txids.insert(target_raw);

    let mut observed_events = Vec::with_capacity(events.len());
    let mut claimed_events = Vec::with_capacity(events.len());
    for item in events {
        if item.claim.network != network {
            return Err(Type9OriginError::UnsupportedNetwork);
        }
        let included = segment
            .check_transaction_inclusion(
                u64::from(item.claim.at.height),
                item.raw_transaction,
                item.merkle_path,
            )
            .map_err(Type9OriginError::HeaderOrMerkle)?;
        check_position(included, item.claim.at, Some(item.claim.txid_raw))?;
        if !seen_txids.insert(item.claim.txid_raw) {
            return Err(Type9OriginError::DuplicateIncludedTransaction);
        }
        let parsed = inspect_raw_type9_candidate_unverified(item.raw_transaction)?;
        if parsed.action != item.claim.action
            || parsed.clearlist_id != item.claim.clearlist_id
            || parsed.target != item.claim.address
        {
            return Err(Type9OriginError::EventSemanticMismatch);
        }
        observed_events.push(included);
        claimed_events.push(item.claim);
    }
    // Chronological, same-fork, no-future and no-same-tx claimed checks.
    ClaimedHistoricalDirectView::audit(TradeLayerMainnet::Bitcoin, target_position, &claimed_events)?;
    Ok(UnverifiedBitcoinType9History {
        target,
        observed_events,
        claimed_events,
        target_position,
        kind,
    })
}
