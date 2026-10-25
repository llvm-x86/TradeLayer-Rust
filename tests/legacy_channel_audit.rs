//! Module 3/49: pinned channels.js direct-path, ONLY unverified projection.
//! The 2-signer signed-state revision and actual multisig verification DENY.
use tradelayer_core::legacy_channel_audit::{
    source_last_char_column, source_tie_break, ChannelSourceError, ClaimedCommit,
    ClaimedPubkeyPreflight, Side, UnverifiedLegacyChannel, UnverifiedQueuedWithdrawal,
    UnverifiedWithdrawalKeys, MAX_EXACT_ATOMS, MAX_CLEARLIST_IDS,
};

fn claim<'a>(
    sender: &'a str,
    token_atoms: u128,
    pay_enabled: bool,
    lists: &'a [u32],
    height: u64,
) -> ClaimedCommit<'a> {
    ClaimedCommit {
        channel: "p2wsh-fake-channel",
        sender,
        property_id: 7,
        token_atoms,
        pay_enabled,
        declared_clear_lists: lists,
        commit_clearlist_id: Some(99),
        height,
        claimed_txid: "synthetic-txid",
        alleged_pubkey_result: ClaimedPubkeyPreflight::ClaimedMatch,
    }
}
fn empty() -> UnverifiedLegacyChannel {
    UnverifiedLegacyChannel::new("p2wsh-fake-channel").unwrap()
}

#[test]
fn last_character_and_tie_breaker_preserve_pinned_ascii_rule() {
    for name in ["Alice1", "EvenA", "3", "c", "X9"] {
        assert_eq!(source_last_char_column(name, 1), Side::A);
    }
    for name in ["Bob2", "Nope0", "v", "x", "B"] {
        assert_eq!(source_last_char_column(name, 1), Side::B);
    }
    assert_eq!(source_last_char_column("", 1), Side::A);
    assert_eq!(source_last_char_column("abc", 4), Side::A);
    assert_eq!(source_last_char_column("abc", 0), Side::A);
    assert_eq!(source_last_char_column("Alice1", 2), Side::A);
    assert_eq!(source_last_char_column("Alice1", 5), Side::B);
    let tie = source_tie_break("Mike3", "Alice1", Side::A).unwrap();
    assert_eq!(tie.winner, "Mike3");
    assert_eq!(tie.loser, "Alice1");
    assert_eq!(tie.winner_column, Side::A);
    assert_eq!(tie.loser_column, Side::B);
    let reverse = source_tie_break("Eve3", "Alice1", Side::A).unwrap();
    assert_eq!(reverse.winner, "Alice1");
    assert_eq!(reverse.loser, "Eve3");
    let undecided = source_tie_break("C3", "A1", Side::A).unwrap();
    assert_eq!(undecided.winner, "C3");
}

#[test]
fn independent_direct_commits_fill_a_and_b_even_though_both_parties_helper_is_dead() {
    let mut s = empty();
    assert_eq!(s.apply_claimed_commit(claim("Alice1", 200_000_000, true, &[7], 100)), Ok(Side::A));
    assert_eq!(s.participant(Side::A), "Alice1");
    assert_eq!(s.participant(Side::B), "");
    assert_eq!(s.last_used_column(), Some(Side::A));
    assert_eq!(s.apply_claimed_commit(claim("Bob2", 300_000_000, false, &[11], 101)), Ok(Side::B));
    assert_eq!(s.participant(Side::A), "Alice1");
    assert_eq!(s.participant(Side::B), "Bob2");
    assert!(!s.source_both_parties_committed_helper());
    assert_eq!(s.source_resolve_column("Bob2"), Some(Side::B));
    assert_eq!(s.source_predict_column("Alice1"), Side::A);
    assert_eq!(s.source_predict_column("Bob2"), Side::B);
    assert_eq!(s.balance_atoms(Side::A, 7), 200_000_000);
    assert_eq!(s.balance_atoms(Side::B, 7), 300_000_000);
    assert_eq!(s.commits().len(), 2);
    assert_eq!(s.participant_changes().len(), 2);
    assert_eq!(s.participant_changes()[0].reason, "Initial assignment");
}

#[test]
fn duplicate_sender_commits_append_and_never_clear_policy_on_false_or_empty_lists() {
    let mut s = empty();
    s.apply_claimed_commit(claim("Alice1", 100_000_000, true, &[7, 9], 101)).unwrap();
    s.apply_claimed_commit(claim("Alice1", 250_000_000, false, &[], 102)).unwrap();
    assert_eq!(s.balance_atoms(Side::A, 7), 350_000_000);
    assert_eq!(s.commits().len(), 2);
    assert_eq!(s.commits()[1].sender, "Alice1");
    assert_eq!(s.commits()[1].column, Side::A);
    assert_eq!(s.commits()[1].amount_atoms, 250_000_000);
    assert_eq!(s.commits()[1].pay_enabled, false);
    assert_eq!(s.commits()[1].commit_clearlist_id, Some(99));
    assert_eq!(s.clear_lists(Side::A), &[7, 9]);
    assert!(s.pay_enabled(Side::A));
    assert_eq!(s.last_commitment_time(), Some(102));
    assert_eq!(s.last_used_column(), Some(Side::A));
    assert_eq!(s.participant_changes().len(), 1);
    let rows = s.source_balance_rows("ALICE1", Some(7));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].amount_atoms, 350_000_000);
    assert_eq!(rows[0].commit_count, 2);
    assert!(rows[0].sender_ever_pay_enabled);
    assert!(s.source_balance_rows("stranger", None).is_empty());
    assert!(s.source_balance_rows("Alice1", Some(8)).is_empty());
    assert_eq!(s.require_authenticated_spend(), Err(ChannelSourceError::ChannelSpendUnverified));
    assert_eq!(s.require_canonical_signed_revision(), Err(ChannelSourceError::CanonicalRevisionUnspecified));
    assert_eq!(s.require_trusted_settlement(), Err(ChannelSourceError::TrustedSettlementUnavailable));
}

#[test]
fn crowded_newcomer_can_replace_previous_b_owner_and_keep_b_deposit() {
    let mut s = empty();
    s.apply_claimed_commit(claim("Alice1", 200_000_000, true, &[7], 100)).unwrap();
    s.apply_claimed_commit(claim("Bob2", 300_000_000, false, &[11], 101)).unwrap();
    let outcome = s.apply_claimed_commit(claim("Eve3", 50_000_000, false, &[], 102));
    assert_eq!(outcome, Ok(Side::B));
    assert_eq!(s.participant(Side::A), "Alice1");
    assert_eq!(s.participant(Side::B), "Eve3");
    assert_eq!(s.source_resolve_column("Bob2"), None);
    assert_eq!(s.balance_atoms(Side::B, 7), 350_000_000);
    assert_eq!(s.clear_lists(Side::B), &[11]);
    assert!(s.possible_crowded_displacement());
    // Source latest-commit fallback may display the SAME B balance for a
    // displaced Bob2 and the replacement Eve3, without any signed transfer.
    let former = s.source_balance_rows("Bob2", None);
    let newer = s.source_balance_rows("Eve3", None);
    assert_eq!(former[0].amount_atoms, newer[0].amount_atoms);
    assert_eq!(newer[0].counterparty, "Alice1");
    assert_eq!(s.require_trusted_settlement(), Err(ChannelSourceError::TrustedSettlementUnavailable));
}

#[test]
fn one_sided_swap_moves_token_balances_but_not_policy_lists_or_pay_flags() {
    let mut s = empty();
    s.apply_claimed_commit(claim("Alice1", 100_000_000, true, &[7], 100)).unwrap();
    s.apply_claimed_commit(claim("Bob2", 500_000_000, false, &[11], 101)).unwrap();
    assert_eq!(s.apply_claimed_commit(claim("Mike3", 200_000_000, false, &[], 102)), Ok(Side::A));
    assert_eq!(s.participant(Side::A), "Mike3");
    assert_eq!(s.participant(Side::B), "Alice1");
    assert_eq!(s.balance_atoms(Side::A, 7), 700_000_000);
    assert_eq!(s.balance_atoms(Side::B, 7), 100_000_000);
    // Pinned bumpColumnAssignment swaps channel.A/B only, not clearLists,
    // payEnabled, pubkeys or already appended commit observations.
    assert_eq!(s.clear_lists(Side::A), &[7]);
    assert_eq!(s.clear_lists(Side::B), &[11]);
    assert!(s.pay_enabled(Side::A));
    assert!(!s.pay_enabled(Side::B));
    assert_eq!(s.commits()[0].column, Side::A);
    assert_eq!(s.commits()[1].column, Side::B);
    assert!(s.possible_crowded_displacement());
    assert_eq!(s.require_canonical_signed_revision(), Err(ChannelSourceError::CanonicalRevisionUnspecified));
}

#[test]
fn invalid_claimer_amount_or_multisig_claim_must_not_partially_mutate_projection() {
    let mut s = empty();
    let original = s.clone();
    let mut bad = claim("Alice1", 1, false, &[], 100);
    bad.alleged_pubkey_result = ClaimedPubkeyPreflight::ClaimedMismatch;
    assert_eq!(s.apply_claimed_commit(bad), Err(ChannelSourceError::ClaimedPubkeyMismatch));
    assert_eq!(s, original);
    assert_eq!(s.apply_claimed_commit(claim("", 100, false, &[], 100)), Err(ChannelSourceError::BadSender));
    assert_eq!(s.apply_claimed_commit(claim("Alice1", 0, false, &[], 100)), Err(ChannelSourceError::InvalidAmount));
    assert_eq!(s.apply_claimed_commit(claim("Alice1", MAX_EXACT_ATOMS+1, false, &[], 100)), Err(ChannelSourceError::InvalidAmount));
    assert_eq!(s.apply_claimed_commit(claim("Alice1", 1, false, &[], 0)), Err(ChannelSourceError::MissingHeight));
    let too_many = vec![7; MAX_CLEARLIST_IDS + 1];
    assert_eq!(s.apply_claimed_commit(claim("Alice1", 1, false, &too_many, 100)), Err(ChannelSourceError::TooManyLists));
    let mut wrong_channel = claim("Alice1", 1, false, &[], 100);
    wrong_channel.channel = "wrong";
    assert_eq!(s.apply_claimed_commit(wrong_channel), Err(ChannelSourceError::BadChannel));
    let mut bad_property = claim("Alice1", 1, false, &[], 100);
    bad_property.property_id = 0;
    assert_eq!(s.apply_claimed_commit(bad_property), Err(ChannelSourceError::InvalidProperty));
    assert_eq!(s, original);
    s.apply_claimed_commit(claim("Alice1", MAX_EXACT_ATOMS, false, &[], 100)).unwrap();
    let before = s.clone();
    assert_eq!(s.apply_claimed_commit(claim("Alice1", 1, false, &[], 101)), Err(ChannelSourceError::Overflow));
    assert_eq!(s, before);
}

#[test]
fn withdrawal_storage_key_collides_across_channels_same_sender_and_height() {
    let mut db = UnverifiedWithdrawalKeys::default();
    let first = UnverifiedQueuedWithdrawal {
        block_height: 400,
        sender: "Alice1".into(),
        channel: "channel-1".into(),
        property_id: 7,
        amount_atoms: 100,
        withdraw_all: false,
        side: Side::A,
    };
    let mut second = first.clone();
    second.channel = "channel-2".into();
    second.property_id = 8;
    assert_eq!(db.source_save(first.clone()), Ok(None));
    assert_eq!(db.source_save(second.clone()), Ok(Some(first)));
    assert_eq!(db.persisted_count(), 1);
    assert_eq!(db.source_lookup(400, "Alice1"), Some(&second));
    assert!(!db.source_delay_elapsed(400, "Alice1", 406));
    assert!(db.source_delay_elapsed(400, "Alice1", 407));
    assert_eq!(db.require_verified_withdrawal(), Err(ChannelSourceError::TrustedSettlementUnavailable));
}
