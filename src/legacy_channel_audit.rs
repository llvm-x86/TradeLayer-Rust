//! Pinned channels.js read-only direct-commit projection (file 3/49).
//!
//! Source: tradelayer.js@dca0888b src/channels.js:497-739,802-917,920-940.
//! Source participants is a plain object {A,B}; its .size is undefined,
//! so areBothPartiesCommitted() is false even when both are occupied.
//! Direct recordCommitToChannel has separate assignment and balance paths.
//!
//! This reproduces only deterministic ASCII column selection, tie-break,
//! participant/balance swaps, crowded replacement, sticky clearLists/pay flags,
//! append-only commit observation, and withdrawal storage key collisions.
//! External first-input pubkey/multisig checks are UNVERIFIED claims.
//! Source BigNumber/JS-Number conversions are restricted to exact 1e-8 atoms
//! below 2^53. This is not a claim about all historic decimal precision.
//! Absolutely no trusted ledger, settlement, market, withdrawal, or signing
//! authority can be produced. Signed off-chain state revision, revocation,
//! invalidation and dispute protocol remains UNSPECIFIED.

use std::collections::BTreeMap;

pub const MAX_CLAIMED_COMMITS: usize = 4096;
pub const MAX_CLAIMED_DELTAS: usize = 4096;
pub const MAX_CLEARLIST_IDS: usize = 64;
pub const MAX_EXACT_ATOMS: u128 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}
impl Side {
    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
    const fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelSourceError {
    BadChannel,
    BadSender,
    BadTxid,
    InvalidProperty,
    InvalidAmount,
    Overflow,
    MissingHeight,
    TooManyCommits,
    TooManyChanges,
    TooManyLists,
    ClaimedPubkeyMismatch,
    ColumnUnresolved,
    CanonicalRevisionUnspecified,
    ChannelSpendUnverified,
    TrustedSettlementUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimedPubkeyPreflight {
    ClaimedMatch,
    ClaimedMismatch,
}

fn actor(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.is_ascii()
        && s.as_bytes().iter().all(|b| (0x21..=0x7e).contains(b))
}

/// Pinned source assignColumnBasedOnLastCharacter defaults to A for invalid
/// or too-short text; otherwise odd uppercase ASCII/digits select A.
pub fn source_last_char_column(address: &str, n: usize) -> Side {
    if n == 0 || address.len() < n || !address.is_ascii() {
        return Side::A;
    }
    let ch = address.as_bytes()[address.len() - n].to_ascii_uppercase();
    if matches!(
        ch,
        b'A' | b'C' | b'E' | b'G' | b'I' | b'K' | b'M' | b'O'
            | b'Q' | b'S' | b'U' | b'W' | b'Y'
            | b'1' | b'3' | b'5' | b'7' | b'9'
    ) {
        Side::A
    } else {
        Side::B
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTie {
    pub winner: String,
    pub loser: String,
    pub winner_column: Side,
    pub loser_column: Side,
}

pub fn source_tie_break(
    incoming: &str,
    counterparty: &str,
    column: Side,
) -> Result<SourceTie, ChannelSourceError> {
    if !actor(incoming) || !actor(counterparty) {
        return Err(ChannelSourceError::BadSender);
    }
    let mut winner = incoming;
    let mut loser = counterparty;
    for n in 1..=incoming.len().min(counterparty.len()) {
        let a = source_last_char_column(incoming, n);
        let b = source_last_char_column(counterparty, n);
        if a == column && b != column {
            break;
        }
        if b == column && a != column {
            winner = counterparty;
            loser = incoming;
            break;
        }
    }
    Ok(SourceTie {
        winner: winner.to_owned(),
        loser: loser.to_owned(),
        winner_column: column,
        loser_column: column.other(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedCommit<'a> {
    pub channel: &'a str,
    pub sender: &'a str,
    pub property_id: u32,
    pub token_atoms: u128,
    pub pay_enabled: bool,
    pub declared_clear_lists: &'a [u32],
    pub commit_clearlist_id: Option<u32>,
    pub height: u64,
    pub claimed_txid: &'a str,
    /// This is only the caller's assertion about handleChannelPubkey().
    pub alleged_pubkey_result: ClaimedPubkeyPreflight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCommitRecord {
    pub sender: String,
    pub property_id: u32,
    pub amount_atoms: u128,
    pub column: Side,
    pub height: u64,
    pub claimed_txid: String,
    pub pay_enabled: bool,
    pub commit_clearlist_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceParticipantChange {
    pub column: Side,
    pub previous: String,
    pub next: String,
    pub height: u64,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBalanceRow {
    pub column: Side,
    pub property_id: u32,
    pub amount_atoms: u128,
    pub counterparty: String,
    pub last_commitment_time: Option<u64>,
    pub commit_count: usize,
    pub sender_ever_pay_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedLegacyChannel {
    address: String,
    participants: [String; 2],
    balances: [BTreeMap<u32, u128>; 2],
    clear_lists: [Vec<u32>; 2],
    pay_enabled: [bool; 2],
    commits: Vec<SourceCommitRecord>,
    participant_changes: Vec<SourceParticipantChange>,
    last_commitment_time: Option<u64>,
    last_used_column: Option<Side>,
    /// Source overwrite can label existing token balance with a new owner.
    possible_crowded_displacement: bool,
}

impl UnverifiedLegacyChannel {
    pub fn new(address: &str) -> Result<Self, ChannelSourceError> {
        if !actor(address) {
            return Err(ChannelSourceError::BadChannel);
        }
        Ok(Self {
            address: address.to_owned(),
            participants: [String::new(), String::new()],
            balances: [BTreeMap::new(), BTreeMap::new()],
            clear_lists: [vec![], vec![]],
            pay_enabled: [false, false],
            commits: vec![],
            participant_changes: vec![],
            last_commitment_time: None,
            last_used_column: None,
            possible_crowded_displacement: false,
        })
    }
    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn participant(&self, side: Side) -> &str {
        &self.participants[side.index()]
    }
    pub fn balance_atoms(&self, side: Side, property_id: u32) -> u128 {
        self.balances[side.index()].get(&property_id).copied().unwrap_or(0)
    }
    pub fn clear_lists(&self, side: Side) -> &[u32] {
        &self.clear_lists[side.index()]
    }
    pub fn pay_enabled(&self, side: Side) -> bool {
        self.pay_enabled[side.index()]
    }
    pub fn commits(&self) -> &[SourceCommitRecord] {
        &self.commits
    }
    pub fn participant_changes(&self) -> &[SourceParticipantChange] {
        &self.participant_changes
    }
    pub const fn last_commitment_time(&self) -> Option<u64> {
        self.last_commitment_time
    }
    pub const fn last_used_column(&self) -> Option<Side> {
        self.last_used_column
    }
    pub const fn possible_crowded_displacement(&self) -> bool {
        self.possible_crowded_displacement
    }
    /// Source-built participants is a plain object, not a JavaScript Map.
    /// In source areBothPartiesCommitted, object.size === undefined.
    pub const fn source_both_parties_committed_helper(&self) -> bool {
        false
    }
    pub fn source_resolve_column(&self, sender: &str) -> Option<Side> {
        if sender.is_empty() {
            None
        } else if self.participant(Side::A) == sender {
            Some(Side::A)
        } else if self.participant(Side::B) == sender {
            Some(Side::B)
        } else {
            None
        }
    }
    pub fn source_predict_column(&self, sender: &str) -> Side {
        self.source_resolve_column(sender)
            .unwrap_or_else(|| source_last_char_column(sender, 1))
    }
    fn record_change(&mut self, side: Side, next: &str, height: u64, reason: &'static str) {
        let old = self.participants[side.index()].clone();
        if old != next {
            self.participants[side.index()] = next.to_owned();
            self.participant_changes.push(SourceParticipantChange {
                column: side,
                previous: old,
                next: next.to_owned(),
                height,
                reason,
            });
        }
    }
    fn bump(&mut self, force_a: &str, force_b: &str, height: u64) {
        let before = self.participants.clone();
        let next_a = if force_a.is_empty() { before[0].clone() } else { force_a.to_owned() };
        let mut next_b = if force_b.is_empty() { before[1].clone() } else { force_b.to_owned() };
        if !next_a.is_empty() && next_a == next_b {
            if !before[0].is_empty() && before[0] != next_a {
                next_b = before[0].clone();
            } else if !before[1].is_empty() && before[1] != next_a {
                next_b = before[1].clone();
            }
        }
        if before == [next_a.clone(), next_b.clone()] {
            return;
        }
        if before[0] == next_b || before[1] == next_a {
            self.participants.swap(0, 1);
            self.balances.swap(0, 1);
            // Source DOES NOT swap clearLists, payEnabled or channelPubkeys.
            if self.participants[0] != next_a || self.participants[1] != next_b {
                if !next_a.is_empty() {
                    self.participants[0] = next_a;
                }
                if !next_b.is_empty() {
                    self.participants[1] = next_b;
                }
            }
        } else {
            // Unsafe source behavior: overwrite owner labels while retaining
            // balances on those column indices, even if occupied.
            for (i, next) in [next_a.as_str(), next_b.as_str()].iter().enumerate() {
                if !before[i].is_empty() && before[i] != *next
                    && self.balances[i].values().any(|v| *v > 0)
                {
                    self.possible_crowded_displacement = true;
                }
            }
            self.participants = [next_a, next_b];
        }
        for (i, side) in [Side::A, Side::B].iter().enumerate() {
            if before[i] != self.participants[i] {
                self.participant_changes.push(SourceParticipantChange {
                    column: *side,
                    previous: before[i].clone(),
                    next: self.participants[i].clone(),
                    height,
                    reason: "Rotated (bumpColumnAssignment)",
                });
            }
        }
    }
    fn assign(&mut self, sender: &str, cp: &str, height: u64) {
        let column = source_last_char_column(sender, 1);
        if self.participants.iter().all(String::is_empty) {
            self.record_change(column, sender, height, "Initial assignment");
        } else if self.source_resolve_column(sender).is_some() {
            // Already committed, keep its column.
        } else if self.participant(column).is_empty() && self.participant(column) != cp {
            self.record_change(column, sender, height, "Assigned via open slot (no conflict)");
        } else if self.participant(column) == cp
            || (!self.participant(column).is_empty() && self.participant(column) != sender)
        {
            let tie = source_tie_break(sender, cp, column)
                .expect("validated caller strings");
            let old_a = self.participants[0].clone();
            let old_b = self.participants[1].clone();
            let force_a = if column == Side::A { &tie.winner } else { &tie.loser };
            let force_b = if column == Side::B { &tie.winner } else { &tie.loser };
            let new_a = if force_a.is_empty() { old_a.as_str() } else { force_a };
            let fallback = if !old_a.is_empty() && old_a != new_a {
                old_a.as_str()
            } else {
                old_b.as_str()
            };
            let new_b = if force_b.is_empty() { fallback } else { force_b };
            self.bump(new_a, new_b, height);
        }
    }
    /// The result is a purely synthetic state observation. The caller can
    /// assert a positive source pubkey preflight; this cannot prove it.
    /// Rejected inputs do not partially mutate the Rust projection.
    pub fn apply_claimed_commit(
        &mut self,
        claim: ClaimedCommit<'_>,
    ) -> Result<Side, ChannelSourceError> {
        if claim.channel != self.address {
            return Err(ChannelSourceError::BadChannel);
        }
        if !actor(claim.sender) {
            return Err(ChannelSourceError::BadSender);
        }
        if !actor(claim.claimed_txid) {
            return Err(ChannelSourceError::BadTxid);
        }
        if claim.property_id == 0 {
            return Err(ChannelSourceError::InvalidProperty);
        }
        if claim.height == 0 {
            return Err(ChannelSourceError::MissingHeight);
        }
        if claim.token_atoms == 0 || claim.token_atoms > MAX_EXACT_ATOMS {
            return Err(ChannelSourceError::InvalidAmount);
        }
        if claim.declared_clear_lists.len() > MAX_CLEARLIST_IDS {
            return Err(ChannelSourceError::TooManyLists);
        }
        if self.commits.len() >= MAX_CLAIMED_COMMITS {
            return Err(ChannelSourceError::TooManyCommits);
        }
        if claim.alleged_pubkey_result != ClaimedPubkeyPreflight::ClaimedMatch {
            return Err(ChannelSourceError::ClaimedPubkeyMismatch);
        }
        let mut next = self.clone();
        let cp = [Side::A, Side::B]
            .iter()
            .map(|s| next.participant(*s))
            .find(|p| !p.is_empty() && *p != claim.sender)
            .unwrap_or("")
            .to_owned();
        let _predicted = next.source_predict_column(claim.sender);
        let owners_before = next.participants.clone();
        let balances_before = next.balances.clone();
        next.assign(claim.sender, &cp, claim.height);
        // Even the source's "one-sided swap" may replace the former
        // opposite-column owner with the incoming sender, leaving the old
        // owner's tokens attached to the newcomer after the swap/align.
        for (i, previous_owner) in owners_before.iter().enumerate() {
            if !previous_owner.is_empty()
                && !next.participants.contains(previous_owner)
                && balances_before[i].values().any(|atoms| *atoms > 0)
            {
                next.possible_crowded_displacement = true;
            }
        }
        let side = next.source_resolve_column(claim.sender)
            .ok_or(ChannelSourceError::ColumnUnresolved)?;
        let i = side.index();
        let total = next.balance_atoms(side, claim.property_id)
            .checked_add(claim.token_atoms)
            .filter(|v| *v <= MAX_EXACT_ATOMS)
            .ok_or(ChannelSourceError::Overflow)?;
        next.balances[i].insert(claim.property_id, total);
        if !claim.declared_clear_lists.is_empty() {
            next.clear_lists[i] = claim.declared_clear_lists.to_vec();
        }
        if claim.pay_enabled {
            next.pay_enabled[i] = true;
        }
        next.commits.push(SourceCommitRecord {
            sender: claim.sender.to_owned(),
            property_id: claim.property_id,
            amount_atoms: claim.token_atoms,
            column: side,
            height: claim.height,
            claimed_txid: claim.claimed_txid.to_owned(),
            pay_enabled: claim.pay_enabled,
            commit_clearlist_id: claim.commit_clearlist_id,
        });
        next.last_commitment_time = Some(claim.height);
        next.last_used_column = Some(side);
        if next.participant_changes.len() > MAX_CLAIMED_DELTAS {
            return Err(ChannelSourceError::TooManyChanges);
        }
        *self = next;
        Ok(side)
    }
    /// Source getChannelBalancesForAddress: first check participants, else
    /// inspect latest committer's column. A displaced committer can therefore
    /// still be shown someone else's current column balance.
    pub fn source_balance_rows(&self, address: &str, filter: Option<u32>) -> Vec<SourceBalanceRow> {
        let needle = address.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return vec![];
        }
        let last_mine = self.commits.iter().rev().find(|record| {
            record.sender.to_ascii_lowercase() == needle
        });
        let side = if !self.participants[0].is_empty()
            && self.participants[0].to_ascii_lowercase() == needle {
            Some(Side::A)
        } else if !self.participants[1].is_empty()
            && self.participants[1].to_ascii_lowercase() == needle {
            Some(Side::B)
        } else {
            last_mine.map(|record| record.column)
        };
        let Some(side) = side else { return vec![] };
        self.balances[side.index()].iter()
            .filter(|(property, amount)| **amount > 0 && filter.is_none_or(|pid| pid == **property))
            .map(|(property, amount)| SourceBalanceRow {
                column: side,
                property_id: *property,
                amount_atoms: *amount,
                counterparty: self.participant(side.other()).to_owned(),
                last_commitment_time: self.last_commitment_time,
                commit_count: self.commits.len(),
                sender_ever_pay_enabled: self.commits.iter().any(|record| {
                    record.sender.to_ascii_lowercase() == needle && record.pay_enabled
                }),
            })
            .collect()
    }
    pub const fn require_authenticated_spend(&self) -> Result<(), ChannelSourceError> {
        Err(ChannelSourceError::ChannelSpendUnverified)
    }
    pub const fn require_canonical_signed_revision(&self) -> Result<(), ChannelSourceError> {
        Err(ChannelSourceError::CanonicalRevisionUnspecified)
    }
    pub const fn require_trusted_settlement(&self) -> Result<(), ChannelSourceError> {
        Err(ChannelSourceError::TrustedSettlementUnavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedQueuedWithdrawal {
    pub block_height: u64,
    pub sender: String,
    pub channel: String,
    pub property_id: u32,
    pub amount_atoms: u128,
    pub withdraw_all: bool,
    pub side: Side,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnverifiedWithdrawalKeys {
    records: BTreeMap<String, UnverifiedQueuedWithdrawal>,
}
impl UnverifiedWithdrawalKeys {
    /// Pinned savePendingWithdrawalToDB key omits channel and property.
    pub fn source_save(&mut self, row: UnverifiedQueuedWithdrawal)
        -> Result<Option<UnverifiedQueuedWithdrawal>, ChannelSourceError> {
        if !actor(&row.sender) || !actor(&row.channel) {
            return Err(ChannelSourceError::BadSender);
        }
        if row.block_height == 0 {
            return Err(ChannelSourceError::MissingHeight);
        }
        Ok(self.records.insert(
            format!("withdrawal-{}-{}", row.block_height, row.sender),
            row,
        ))
    }
    pub fn persisted_count(&self) -> usize {
        self.records.len()
    }
    pub fn source_lookup(&self, block: u64, sender: &str) -> Option<&UnverifiedQueuedWithdrawal> {
        self.records.get(&format!("withdrawal-{block}-{sender}"))
    }
    pub fn source_delay_elapsed(&self, block: u64, sender: &str, now: u64) -> bool {
        self.source_lookup(block, sender).is_some_and(|record| {
            record.block_height.checked_add(7).is_some_and(|due| now >= due)
        })
    }
    pub const fn require_verified_withdrawal(&self) -> Result<(), ChannelSourceError> {
        Err(ChannelSourceError::TrustedSettlementUnavailable)
    }
}
