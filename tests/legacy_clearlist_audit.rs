//! Source-pinned legacy clearlist NeDB/Map collision and xpub Merkle parity.
//! Source observation, NEVER historical list/chain/settlement authorization.
use tradelayer_core::legacy_clearlist_audit::{
    source_build_xpub_merkle_tree, source_generate_xpub_merkle_proof,
    source_verify_xpub_merkle_proof, ClearlistSourceError, SourceAttestationStatus,
    SourceAttestationWrite, SourceMerkleSide, SourceRevocation, UnverifiedLegacyClearlistStore,
    DEFAULT_BANNED_COUNTRIES,
};

fn created(id: Option<u32>) -> UnverifiedLegacyClearlistStore {
    let mut store = UnverifiedLegacyClearlistStore::new();
    store.source_create_clearlist(id, "adminA", "backupB", "List One", "about")
        .unwrap();
    store
}

#[test]
fn numeric_create_id_and_string_update_db_shadow_cause_stale_reload() {
    let mut s = created(None);
    assert_eq!(s.source_next_list_id().unwrap(), 2);
    assert_eq!(s.source_get_clearlist_by_id(1).unwrap().primary_admin(), "adminA");
    assert_eq!(s.source_verify_admin(1, "adminA"), Ok(true));
    s.source_update_admin(1, "adminC", false).unwrap();
    // JavaScript Map key 1 is still mutated, but updateAsync inserts DB
    // key "1" as a *distinct* string from the numeric create _id=1.
    assert_eq!(s.observed_cached_numeric_list(1).unwrap().primary_admin(), "adminC");
    assert_eq!(s.observed_string_shadow_list(1).unwrap().primary_admin(), "adminC");
    assert_eq!(s.observed_numeric_persisted_list(1).unwrap().primary_admin(), "adminA");
    // The Type9 getClearlistById helper calls loadClearlists, which reloads
    // numeric DB row _id=1 into Map key number 1, losing the updated value.
    assert_eq!(s.source_get_clearlist_by_id(1).unwrap().primary_admin(), "adminA");
    assert_eq!(s.source_verify_admin(1, "adminC"), Ok(false));
    assert_eq!(s.source_verify_admin(1, "adminA"), Ok(true));
    assert_eq!(s.observed_string_shadow_list(1).unwrap().primary_admin(), "adminC");
    assert_eq!(s.source_get_list_type8_lookup(1), None);
    assert_eq!(s.source_next_list_id(), Ok(2));
    assert_eq!(s.require_consensus_accepted_type8(),
        Err(ClearlistSourceError::AcceptedSourceTransitionUnverified));
}

#[test]
fn update_requiring_prior_load_and_backup_mutation_are_source_behaviors() {
    let mut s = created(Some(4));
    assert_eq!(
        s.source_update_admin(4, "backupC", true),
        Err(ClearlistSourceError::ListNotLoaded)
    );
    assert_eq!(s.source_get_clearlist_by_id(4).unwrap().backup_admin(), "backupB");
    s.source_update_admin(4, "backupC", true).unwrap();
    assert_eq!(s.observed_cached_numeric_list(4).unwrap().backup_admin(), "backupC");
    assert_eq!(s.observed_string_shadow_list(4).unwrap().backup_admin(), "backupC");
    assert_eq!(s.source_get_clearlist_by_id(4).unwrap().backup_admin(), "backupB");
    assert_eq!(s.source_next_list_id(), Ok(5));
    // Explicit duplicate numeric ID upsert overwrites previous data.
    s.source_create_clearlist(Some(4), "newAdmin", "", "New", "").unwrap();
    assert_eq!(s.source_get_clearlist_by_id(4).unwrap().primary_admin(), "newAdmin");
}

#[test]
fn type8_get_list_collection_lookup_does_not_read_type7_created_list() {
    let mut s = created(None);
    assert_eq!(s.source_get_list_type8_lookup(1), None);
    s.source_add_attestation(1, "alice", "OK", None, 10).unwrap();
    assert_eq!(s.source_get_list_type8_lookup(1), None);
    assert_eq!(s.source_get_clearlist_by_id(1).unwrap().primary_admin(), "adminA");
    assert_eq!(
        s.require_verified_clearlist_mutation(),
        Err(ClearlistSourceError::HistoricalClearlistAuthorityUnavailable)
    );
    assert_eq!(
        s.require_signed_channel_revision(),
        Err(ClearlistSourceError::CanonicalChannelRevisionUnspecified)
    );
}

#[test]
fn address_keyed_attestation_across_lists_replaces_old_membership() {
    let mut s = created(None);
    s.source_create_clearlist(None, "adminD", "", "Second", "").unwrap();
    assert_eq!(s.source_add_attestation(1, "alice", "OK", None, 10),
        Ok(SourceAttestationWrite::Added));
    assert!(s.source_is_address_directly_attested(1, "alice"));
    assert_eq!(s.source_get_attestations(1).len(), 1);
    assert_eq!(s.source_add_attestation(1, "alice", "NEW", None, 11),
        Ok(SourceAttestationWrite::ReplacedSameList));
    assert_eq!(s.source_get_attestation_history(1, "alice").len(), 1);
    assert_eq!(
        s.source_add_attestation(2, "alice", "OTHER", None, 12),
        Ok(SourceAttestationWrite::OverwroteOtherList {
            previous: 1, incoming: 2
        })
    );
    assert!(!s.source_is_address_directly_attested(1, "alice"));
    assert!(s.source_is_address_directly_attested(2, "alice"));
    assert_eq!(s.source_get_attestations(1).len(), 0);
    assert!(s.source_get_attestation_history(1, "alice").is_empty());
    assert_eq!(s.observed_attestation_by_source_key("alice").unwrap().metadata(),
        Some("OTHER"));
}

#[test]
fn wrong_list_revocation_fallback_mutates_other_lists_address_key() {
    let mut s = created(None);
    s.source_add_attestation(1, "alice", "OK", None, 10).unwrap();
    assert_eq!(
        s.source_revoke_attestation(99, "alice", "wrong-list", 11),
        Ok(SourceRevocation::CrossListFallback {
            requested: 99, overwritten: 1,
        })
    );
    assert!(!s.source_is_address_directly_attested(1, "alice"));
    let row = s.observed_attestation_by_source_key("alice").unwrap();
    assert_eq!(row.list_id(), 1); // source row listId stays 1
    assert_eq!(row.revoked_by_list_id(), Some(99)); // new source data.id=99
    assert_eq!(row.status(), SourceAttestationStatus::Revoked);
    assert_eq!(row.revoke_reason(), Some("wrong-list"));
    assert_eq!(
        s.source_revoke_attestation(1, "alice", "again", 12),
        Ok(SourceRevocation::ReplayedFallback)
    );
    assert_eq!(
        s.source_revoke_attestation(1, "bob", "not-present", 12),
        Err(ClearlistSourceError::NoAttestationToRevoke)
    );
}

#[test]
fn correct_list_revoke_and_reissue_keep_only_latest_db_row() {
    let mut s = created(None);
    s.source_add_attestation(1, "alice", "first", None, 10).unwrap();
    assert_eq!(
        s.source_revoke_attestation(1, "alice", "revoked", 11),
        Ok(SourceRevocation::RevokedMatchingList)
    );
    assert_eq!(s.source_get_attestation_history(1, "alice").len(), 1);
    assert!(!s.source_is_address_directly_attested(1, "alice"));
    assert_eq!(s.source_add_attestation(1, "alice", "reissued", None, 12),
        Ok(SourceAttestationWrite::ReplacedSameList));
    assert!(s.source_is_address_directly_attested(1, "alice"));
    assert_eq!(s.observed_attestation_by_source_key("alice").unwrap().revoke_reason(), None);
}

#[test]
fn source_country_self_cert_is_overwritten_by_same_address_other_list() {
    let mut s = created(None);
    s.source_add_attestation(0, "alice", " us ", None, 0).unwrap();
    let cc = s.source_country_code_by_address("alice").unwrap();
    assert_eq!(cc.code, "US");
    assert_eq!(cc.reported_height, None);
    s.source_add_attestation(1, "alice", "OK", None, 14).unwrap();
    assert_eq!(s.source_country_code_by_address("alice"), None);
    s.source_add_attestation(0, "alice", "NZ", None, 15).unwrap();
    assert_eq!(s.source_country_code_by_address("alice").unwrap().reported_height, Some(15));
    s.source_revoke_attestation(0, "alice", "revoked", 16).unwrap();
    assert_eq!(s.source_country_code_by_address("alice"), None);
}

#[test]
fn cached_banlist_changes_while_buggy_persisted_getter_remains_undefined() {
    let mut s = UnverifiedLegacyClearlistStore::new();
    assert_eq!(
        s.source_cached_banlist().iter().map(String::as_str).collect::<Vec<_>>(),
        DEFAULT_BANNED_COUNTRIES
    );
    assert_eq!(s.source_stored_banlist(), None);
    assert_eq!(s.source_get_banlist_bug(), None);
    s.source_set_banlist(&["GB", "KP"], 200).unwrap();
    assert_eq!(s.source_stored_banlist().unwrap(), ["GB", "KP"]);
    assert_eq!(s.source_cached_banlist(), ["GB", "KP"]);
    assert_eq!(s.source_get_banlist_bug(), None);
}

#[test]
fn source_xpub_merkle_uses_ascii_hex_pairs_and_duplicates_single_leaf() {
    let singleton = source_build_xpub_merkle_tree(&["xpub-synthetic-0"]).unwrap();
    assert_eq!(singleton.root.as_deref(),
        Some("82f8fcd70bcdf26565adb613a0dfe04955ee27054ab30f03e23160219aa73de9"));
    assert_eq!(singleton.layers.len(), 2);
    assert_eq!(singleton.layers[0].len(), 2);
    let proof = source_generate_xpub_merkle_proof(&["xpub-synthetic-0"], "xpub-synthetic-0")
        .unwrap().unwrap();
    assert_eq!(proof.len(), 1);
    assert_eq!(proof[0].position, SourceMerkleSide::Right);
    assert!(source_verify_xpub_merkle_proof(
        "xpub-synthetic-0", &proof, singleton.root.as_deref().unwrap()).unwrap());
    assert!(!source_verify_xpub_merkle_proof(
        "xpub-other", &proof, singleton.root.as_deref().unwrap()).unwrap());
    assert_eq!(source_build_xpub_merkle_tree(&[]).unwrap().root, None);
    assert_eq!(source_generate_xpub_merkle_proof(&[], "x").unwrap(), None);
}

#[test]
fn odd_upper_layer_proof_generator_has_a_source_verification_gap() {
    let names = [
        "xpub-synthetic-0", "xpub-synthetic-1", "xpub-synthetic-2",
        "xpub-synthetic-3", "xpub-synthetic-4",
    ];
    let tree = source_build_xpub_merkle_tree(&names).unwrap();
    assert_eq!(tree.root.as_deref(),
        Some("a9a5833ec88f79b211ba29e8d3bc936c02dba0ffaeaf2a48149304b679b63f3a"));
    let good = source_generate_xpub_merkle_proof(&names, names[0]).unwrap().unwrap();
    assert_eq!(good.len(), 3);
    assert!(source_verify_xpub_merkle_proof(
        names[0], &good, tree.root.as_deref().unwrap()).unwrap());
    // For five xpubs, the last leaf's parent is at index 2 of three
    // parents. The JS generator SKIPS the missing sibling, while its
    // verifier has no implicit-padding rule, so this *fails* to verify.
    let missing = source_generate_xpub_merkle_proof(&names, names[4]).unwrap().unwrap();
    assert_eq!(missing.len(), 2);
    assert!(!source_verify_xpub_merkle_proof(
        names[4], &missing, tree.root.as_deref().unwrap()).unwrap());
    assert_eq!(source_generate_xpub_merkle_proof(&names, "not-here").unwrap(), None);
}

#[test]
fn root_registration_uses_global_root_map_and_does_not_validate_bip32_derived_addresses() {
    let mut s = created(None);
    let xpub = "xpub-synthetic-0";
    let tree = source_build_xpub_merkle_tree(&[xpub]).unwrap();
    let root = tree.root.as_deref().unwrap();
    let proof = source_generate_xpub_merkle_proof(&[xpub], xpub).unwrap().unwrap();
    assert_eq!(s.source_register_xpub_for_root(root, xpub, &proof), Ok(false));
    s.source_store_merkle_root(1, root, "adminA", 22).unwrap();
    assert_eq!(s.source_register_xpub_for_root(root, xpub, &proof), Ok(true));
    assert_eq!(s.source_register_xpub_for_root(root, xpub, &proof), Ok(true));
    assert_eq!(s.observed_merkle_xpubs(root).unwrap().len(), 1);
    assert_eq!(s.source_direct_or_derived_unverified(1, "derived-address"),
        Err(ClearlistSourceError::DerivationUnavailable));
    // Root itself is registered under a separate source attestation key.
    let id = format!("merkle:1:{root}");
    let row = s.observed_attestation_by_source_key(&id).unwrap();
    assert_eq!(row.merkle_root(), Some(root));
    assert_eq!(row.list_id(), 1);
    s.source_store_merkle_root(2, root, "adminC", 23).unwrap();
    assert_eq!(s.observed_merkle_xpubs(root).unwrap().len(), 1);
    assert_eq!(s.source_direct_or_derived_unverified(2, "different-derived"),
        Err(ClearlistSourceError::DerivationUnavailable));
}

#[test]
fn inline_xpub_is_a_derived_candidate_but_never_an_authorization() {
    let mut s = created(None);
    s.source_add_attestation(1, "xpub-owner", "xpub:synthetic", None, 100).unwrap();
    let row = s.observed_attestation_by_source_key("xpub-owner").unwrap();
    assert_eq!(row.xpub(), Some("synthetic"));
    assert_eq!(s.source_direct_or_derived_unverified(1, "xpub-owner"), Ok(true));
    assert_eq!(s.source_direct_or_derived_unverified(1, "derived-child"),
        Err(ClearlistSourceError::DerivationUnavailable));
    assert_eq!(s.source_direct_or_derived_unverified(4, "not-listed"), Ok(false));
    assert_eq!(
        s.require_verified_clearlist_mutation(),
        Err(ClearlistSourceError::HistoricalClearlistAuthorityUnavailable)
    );
    assert_eq!(
        s.require_consensus_accepted_type8(),
        Err(ClearlistSourceError::AcceptedSourceTransitionUnverified)
    );
    assert_eq!(
        s.require_signed_channel_revision(),
        Err(ClearlistSourceError::CanonicalChannelRevisionUnspecified)
    );
}
