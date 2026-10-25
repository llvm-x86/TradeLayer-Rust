//! Deterministic, UNVERIFIED legacy clearlist.js storage behavior.
//!
//! Source: patrickdugan/tradelayer.js@dca0888b, src/clearlist.js:1-574.
//! This module models relevant direct NeDB/MAP key behavior, not a new
//! canonical whitelist. It deliberately exposes source defects:
//! - numeric ID on create vs STRING ID on updateAdmin makes shadow DB rows;
//! - getList() reads attestations, not clearlists (Type8 validator mismatch);
//! - attestations upsert by address, overwriting any previous list for it;
//! - revoke fallback by address can revoke a row from the WRONG list;
//! - getBanlist uses findAsync (array), then reads array.data => undefined;
//! - merkle-root xpub registrations are keyed by root, not by list;
//! - SHA256 merkle tree hashes CONCATENATED LOWERCASE HEX TEXT, not bytes.
//!
//! HD xpub derivation uses external bitcore-lib-ltc / bitcore-lib in the JS
//! and is NOT implemented here. No derived address is ever certified absent
//! based on missing direct evidence. The source proof generator skips
//! siblings at odd upper levels; reproduce for parity, not authority.
//!
//! Mutations represent what callers claim happened in unverified JS memory
//! and NeDB records. They DO NOT require a valid admin signature, a
//! historical chain, authorization, full replay or economic consensus.
//! No ability to construct TrustedProtocolAdapter or a ledger permission.

use crate::hash256::sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const DEFAULT_BANNED_COUNTRIES: [&str; 6] = ["US", "KP", "SY", "RU", "IR", "CU"];
pub const MAX_SOURCE_LISTS: usize = 4096;
pub const MAX_SOURCE_ATTESTATIONS: usize = 8192;
pub const MAX_SOURCE_XPUBS: usize = 2048;
pub const MAX_SOURCE_MERKLE_DEPTH: usize = 32;
pub const MAX_SOURCE_TEXT: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearlistSourceError {
    EmptyName,
    NonAsciiOrOversize,
    InvalidListId,
    ListIdExhausted,
    ListNotLoaded,
    TooManyLists,
    TooManyAttestations,
    TooManyXpubs,
    InvalidMerkleProof,
    MerkleEvidenceMissing,
    DerivationUnavailable,
    NoAttestationToRevoke,
    /// Unverified source behavior MUST NEVER become authorization.
    HistoricalClearlistAuthorityUnavailable,
    AcceptedSourceTransitionUnverified,
    CanonicalChannelRevisionUnspecified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedSourceClearlist {
    id: u32,
    admin: String,
    backup: String,
    admin_address: Option<String>,
    backup_address: Option<String>,
    name: String,
    description: String,
}

impl UnverifiedSourceClearlist {
    pub const fn id(&self) -> u32 {
        self.id
    }
    pub fn primary_admin(&self) -> &str {
        self.admin_address.as_deref().unwrap_or(&self.admin)
    }
    pub fn backup_admin(&self) -> &str {
        self.backup_address.as_deref().unwrap_or(&self.backup)
    }
    pub fn initial_admin(&self) -> &str {
        &self.admin
    }
    pub fn source_name(&self) -> &str {
        &self.name
    }
    pub fn source_description(&self) -> &str {
        &self.description
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceAttestationStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedSourceAttestation {
    list_id: u32,
    address: String,
    metadata: Option<String>,
    xpub: Option<String>,
    root: Option<String>,
    status: SourceAttestationStatus,
    timestamp: u64,
    revoke_reason: Option<String>,
    /// Legacy clearlist ID field added during revocation: data.id.
    revoked_by_list_id: Option<u32>,
}

impl UnverifiedSourceAttestation {
    pub const fn list_id(&self) -> u32 {
        self.list_id
    }
    pub fn address(&self) -> &str {
        &self.address
    }
    pub fn metadata(&self) -> Option<&str> {
        self.metadata.as_deref()
    }
    pub fn xpub(&self) -> Option<&str> {
        self.xpub.as_deref()
    }
    pub fn merkle_root(&self) -> Option<&str> {
        self.root.as_deref()
    }
    pub fn status(&self) -> SourceAttestationStatus {
        self.status.clone()
    }
    pub const fn timestamp(&self) -> u64 {
        self.timestamp
    }
    pub fn revoke_reason(&self) -> Option<&str> {
        self.revoke_reason.as_deref()
    }
    pub const fn revoked_by_list_id(&self) -> Option<u32> {
        self.revoked_by_list_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceAttestationWrite {
    Added,
    ReplacedSameList,
    OverwroteOtherList { previous: u32, incoming: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceRevocation {
    RevokedMatchingList,
    /// When first query finds no active matching-list row, source fallback
    /// selects an address-keyed row even if its list_id differs. It can
    /// overwrite other list membership and must never be authorized here.
    CrossListFallback {
        requested: u32,
        overwritten: u32,
    },
    ReplayedFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCountryCode {
    pub code: String,
    pub reported_height: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMerkleSide {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMerkleStep {
    /// Exactly 64 lower-case hex ASCII chars from Node crypto digest("hex").
    pub sibling_hex: String,
    pub position: SourceMerkleSide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedSourceMerkleTree {
    pub root: Option<String>,
    pub layers: Vec<Vec<String>>,
}

fn source_text(value: &str) -> Result<(), ClearlistSourceError> {
    if value.len() > MAX_SOURCE_TEXT
        || !value.is_ascii()
        || value.chars().any(|c| !(0x20..=0x7e).contains(&(c as u8)))
    {
        return Err(ClearlistSourceError::NonAsciiOrOversize);
    }
    Ok(())
}

fn source_nonempty(value: &str) -> Result<(), ClearlistSourceError> {
    source_text(value)?;
    if value.is_empty() {
        return Err(ClearlistSourceError::EmptyName);
    }
    Ok(())
}

fn digest_hex(bytes: &[u8]) -> String {
    sha256(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn valid_hex_digest(raw: &str) -> bool {
    raw.len() == 64
        && raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Source SHA256(xpub UTF8), then SHA256(leftHexASCII || rightHexASCII).
/// Its one-leaf build ALSO duplicates that leaf and hashes the pair.
/// No BIP32, Bitcoin transaction or administrative authority is inferred.
pub fn source_build_xpub_merkle_tree(
    xpubs: &[&str],
) -> Result<UnverifiedSourceMerkleTree, ClearlistSourceError> {
    if xpubs.len() > MAX_SOURCE_XPUBS {
        return Err(ClearlistSourceError::TooManyXpubs);
    }
    for xpub in xpubs {
        source_nonempty(xpub)?;
    }
    if xpubs.is_empty() {
        return Ok(UnverifiedSourceMerkleTree {
            root: None,
            layers: vec![],
        });
    }
    let mut leaves: Vec<String> = xpubs
        .iter()
        .map(|xpub| digest_hex(xpub.as_bytes()))
        .collect();
    if leaves.len() % 2 != 0 {
        leaves.push(leaves.last().expect("nonempty").clone());
    }
    let mut layers = vec![leaves.clone()];
    while leaves.len() > 1 {
        let mut next = Vec::with_capacity(leaves.len().div_ceil(2));
        for i in (0..leaves.len()).step_by(2) {
            let partner = leaves.get(i + 1).unwrap_or(&leaves[i]);
            next.push(digest_hex(format!("{}{}", leaves[i], partner).as_bytes()));
        }
        layers.push(next.clone());
        leaves = next;
        if layers.len() > MAX_SOURCE_MERKLE_DEPTH {
            return Err(ClearlistSourceError::InvalidMerkleProof);
        }
    }
    Ok(UnverifiedSourceMerkleTree {
        root: leaves.first().cloned(),
        layers,
    })
}

/// Reproduces the JS proof generator's actual skip on absent sibling.
/// For some 5+-leaf trees this generates a proof that its own verifier
/// rejects. This behavior is an audit finding, not proof-completeness.
pub fn source_generate_xpub_merkle_proof(
    xpubs: &[&str],
    target: &str,
) -> Result<Option<Vec<SourceMerkleStep>>, ClearlistSourceError> {
    let tree = source_build_xpub_merkle_tree(xpubs)?;
    let Some(first) = tree.layers.first() else {
        return Ok(None);
    };
    let target_hash = digest_hex(target.as_bytes());
    let Some(mut index) = first.iter().position(|x| x == &target_hash) else {
        return Ok(None);
    };
    let mut proof = Vec::new();
    for layer in tree.layers.iter().take(tree.layers.len() - 1) {
        let right = index % 2 == 1;
        let sibling_index = if right { index - 1 } else { index + 1 };
        if let Some(sibling) = layer.get(sibling_index) {
            proof.push(SourceMerkleStep {
                sibling_hex: sibling.clone(),
                position: if right {
                    SourceMerkleSide::Left
                } else {
                    SourceMerkleSide::Right
                },
            });
        }
        index /= 2;
    }
    Ok(Some(proof))
}

pub fn source_verify_xpub_merkle_proof(
    xpub: &str,
    proof: &[SourceMerkleStep],
    expected_root: &str,
) -> Result<bool, ClearlistSourceError> {
    source_nonempty(xpub)?;
    if proof.len() > MAX_SOURCE_MERKLE_DEPTH || !valid_hex_digest(expected_root) {
        return Err(ClearlistSourceError::InvalidMerkleProof);
    }
    let mut current = digest_hex(xpub.as_bytes());
    for item in proof {
        if !valid_hex_digest(&item.sibling_hex) {
            return Err(ClearlistSourceError::InvalidMerkleProof);
        }
        current = digest_hex(
            match item.position {
                SourceMerkleSide::Left => format!("{}{}", item.sibling_hex, current),
                SourceMerkleSide::Right => format!("{}{}", current, item.sibling_hex),
            }
            .as_bytes(),
        );
    }
    Ok(current == expected_root)
}

/// This projection maintains the legacy DIFFERENT NeDB ID types and the
/// separate JavaScript Map cache that loadClearlists can overwrite.
/// Everything is in-memory for observational parity; no storage/snapshot
/// or chain adapter exists here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnverifiedLegacyClearlistStore {
    numeric_db_lists: BTreeMap<u32, UnverifiedSourceClearlist>,
    string_db_shadow_lists: BTreeMap<String, UnverifiedSourceClearlist>,
    numeric_cache: BTreeMap<u32, UnverifiedSourceClearlist>,
    string_cache: BTreeMap<String, UnverifiedSourceClearlist>,
    attestations_by_ne_db_id: BTreeMap<String, UnverifiedSourceAttestation>,
    xpubs_by_merkle_root: BTreeMap<String, BTreeSet<String>>,
    source_cached_banlist: Vec<String>,
    source_stored_banlist: Option<(Vec<String>, u64)>,
}

impl Default for UnverifiedLegacyClearlistStore {
    fn default() -> Self {
        Self {
            numeric_db_lists: BTreeMap::new(),
            string_db_shadow_lists: BTreeMap::new(),
            numeric_cache: BTreeMap::new(),
            string_cache: BTreeMap::new(),
            attestations_by_ne_db_id: BTreeMap::new(),
            xpubs_by_merkle_root: BTreeMap::new(),
            source_cached_banlist: DEFAULT_BANNED_COUNTRIES
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            source_stored_banlist: None,
        }
    }
}

impl UnverifiedLegacyClearlistStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Implements loadClearlists() Map.set(_id,data): number 1 and string
    /// "1" are DISTINCT keys, and numeric DB records overwrite the numeric
    /// in-memory version after updateAdmin changed it.
    pub fn source_load_clearlists(&mut self) {
        for (id, list) in &self.numeric_db_lists {
            self.numeric_cache.insert(*id, list.clone());
        }
        for (id, list) in &self.string_db_shadow_lists {
            self.string_cache.insert(id.clone(), list.clone());
        }
    }

    pub fn source_next_list_id(&mut self) -> Result<u32, ClearlistSourceError> {
        self.source_load_clearlists();
        let max_numeric = self.numeric_cache.keys().copied().max().unwrap_or(0);
        let max_string = self
            .string_cache
            .keys()
            .filter_map(|s| s.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        max_numeric
            .max(max_string)
            .checked_add(1)
            .ok_or(ClearlistSourceError::ListIdExhausted)
    }

    /// Source createClearlist accepts an optionally explicit ID, with falsy
    /// zero invoking next-ID allocation. Source URL is NOT persisted.
    /// No activation/admin/sender signature is examined by this low-level
    /// method; callers cannot treat its result as historical acceptance.
    pub fn source_create_clearlist(
        &mut self,
        explicit_id: Option<u32>,
        admin: &str,
        backup: &str,
        name: &str,
        description: &str,
    ) -> Result<u32, ClearlistSourceError> {
        for s in [admin, backup, name, description] {
            source_text(s)?;
        }
        source_nonempty(admin)?;
        source_nonempty(name)?;
        let id = match explicit_id {
            Some(0) | None => self.source_next_list_id()?,
            Some(id) => id,
        };
        if id == 0 {
            return Err(ClearlistSourceError::InvalidListId);
        }
        if !self.numeric_db_lists.contains_key(&id)
            && self.numeric_db_lists.len() >= MAX_SOURCE_LISTS
        {
            return Err(ClearlistSourceError::TooManyLists);
        }
        self.numeric_db_lists.insert(
            id,
            UnverifiedSourceClearlist {
                id,
                admin: admin.to_owned(),
                backup: backup.to_owned(),
                admin_address: None,
                backup_address: None,
                name: name.to_owned(),
                description: description.to_owned(),
            },
        );
        Ok(id)
    }

    /// The source checks the memory Map only (not DB) before update.
    /// Its DB write uses STRING "_id" although create used NUMERIC "_id".
    pub fn source_update_admin(
        &mut self,
        list_id: u32,
        new_address: &str,
        update_backup: bool,
    ) -> Result<(), ClearlistSourceError> {
        source_nonempty(new_address)?;
        let cache = self
            .numeric_cache
            .get_mut(&list_id)
            .ok_or(ClearlistSourceError::ListNotLoaded)?;
        if update_backup {
            cache.backup_address = Some(new_address.to_owned());
            cache.backup = new_address.to_owned();
        } else {
            cache.admin_address = Some(new_address.to_owned());
            cache.admin = new_address.to_owned();
        }
        // Cross-typed NeDB update creates a NEW shadow row.
        self.string_db_shadow_lists
            .insert(list_id.to_string(), cache.clone());
        Ok(())
    }

    /// getClearlistById first calls loadClearlists(), resetting NUMERIC
    /// cached ID to old numeric persisted list, not the string-shadow row.
    pub fn source_get_clearlist_by_id(
        &mut self,
        list_id: u32,
    ) -> Option<&UnverifiedSourceClearlist> {
        self.source_load_clearlists();
        self.numeric_cache.get(&list_id)
    }

    pub fn observed_cached_numeric_list(&self, list_id: u32) -> Option<&UnverifiedSourceClearlist> {
        self.numeric_cache.get(&list_id)
    }

    pub fn observed_string_shadow_list(&self, list_id: u32) -> Option<&UnverifiedSourceClearlist> {
        self.string_db_shadow_lists.get(&list_id.to_string())
    }

    pub fn observed_numeric_persisted_list(
        &self,
        list_id: u32,
    ) -> Option<&UnverifiedSourceClearlist> {
        self.numeric_db_lists.get(&list_id)
    }

    /// getList() selects the attestations collection by NUMERIC list ID,
    /// but create/updates write clearlists; attestation IDs are STRINGS.
    /// Without an independent legacy row injected at a numeric key, this
    /// always returns None, matching the described source branch.
    pub fn source_get_list_type8_lookup(
        &self,
        _list_id: u32,
    ) -> Option<&UnverifiedSourceClearlist> {
        None
    }

    /// Source verifyAdmin checks current cache, not the stored collection,
    /// and will throw for a list that has not been loaded.
    pub fn source_verify_admin(
        &self,
        list_id: u32,
        sender: &str,
    ) -> Result<bool, ClearlistSourceError> {
        let list = self
            .numeric_cache
            .get(&list_id)
            .ok_or(ClearlistSourceError::ListNotLoaded)?;
        Ok(list.primary_admin() == sender)
    }

    pub fn source_add_attestation(
        &mut self,
        list_id: u32,
        address: &str,
        metadata: &str,
        explicit_xpub: Option<&str>,
        height: u64,
    ) -> Result<SourceAttestationWrite, ClearlistSourceError> {
        source_nonempty(address)?;
        source_text(metadata)?;
        if let Some(x) = explicit_xpub {
            source_text(x)?;
        }
        if self.attestations_by_ne_db_id.len() >= MAX_SOURCE_ATTESTATIONS
            && !self.attestations_by_ne_db_id.contains_key(address)
        {
            return Err(ClearlistSourceError::TooManyAttestations);
        }
        // addAttestation detects the exact xpub: prefix (case sensitive).
        let xpub = explicit_xpub
            .filter(|s| !s.is_empty())
            .or_else(|| metadata.strip_prefix("xpub:").filter(|s| !s.is_empty()));
        let row = UnverifiedSourceAttestation {
            list_id,
            address: address.to_owned(),
            metadata: Some(metadata.to_owned()),
            xpub: xpub.map(str::to_owned),
            root: None,
            status: SourceAttestationStatus::Active,
            timestamp: height,
            revoke_reason: None,
            revoked_by_list_id: None,
        };
        let outcome = match self
            .attestations_by_ne_db_id
            .insert(address.to_owned(), row)
        {
            None => SourceAttestationWrite::Added,
            Some(previous) if previous.list_id == list_id => {
                SourceAttestationWrite::ReplacedSameList
            }
            Some(previous) => SourceAttestationWrite::OverwroteOtherList {
                previous: previous.list_id,
                incoming: list_id,
            },
        };
        Ok(outcome)
    }

    /// First query: matching active list/target. Fallback: address-keyed row
    /// WITHOUT list constraint, which may revoke a DIFFERENT list.
    /// An intentionally explicit hazard result accompanies that mutation.
    pub fn source_revoke_attestation(
        &mut self,
        requested_list: u32,
        address: &str,
        reason: &str,
        height: u64,
    ) -> Result<SourceRevocation, ClearlistSourceError> {
        source_nonempty(address)?;
        source_text(reason)?;
        let row = self
            .attestations_by_ne_db_id
            .get_mut(address)
            .ok_or(ClearlistSourceError::NoAttestationToRevoke)?;
        let result =
            if row.list_id == requested_list && row.status == SourceAttestationStatus::Active {
                SourceRevocation::RevokedMatchingList
            } else if row.list_id != requested_list {
                SourceRevocation::CrossListFallback {
                    requested: requested_list,
                    overwritten: row.list_id,
                }
            } else {
                SourceRevocation::ReplayedFallback
            };
        row.status = SourceAttestationStatus::Revoked;
        row.timestamp = height;
        row.revoked_by_list_id = Some(requested_list);
        row.revoke_reason = Some(reason.to_owned());
        Ok(result)
    }

    pub fn source_is_address_directly_attested(&self, list_id: u32, address: &str) -> bool {
        self.attestations_by_ne_db_id.values().any(|row| {
            row.address == address
                && row.list_id == list_id
                && row.status == SourceAttestationStatus::Active
        })
    }

    pub fn source_get_attestations(&self, list_id: u32) -> Vec<&UnverifiedSourceAttestation> {
        self.attestations_by_ne_db_id
            .values()
            .filter(|row| row.list_id == list_id)
            .collect()
    }

    /// The JS getAttestationHistory() reads current DB rows and sorts by
    /// timestamp; it does not store an append-only event log. Same address
    /// upserts normally leave at most ONE record, despite method name.
    pub fn source_get_attestation_history(
        &self,
        list_id: u32,
        address: &str,
    ) -> Vec<&UnverifiedSourceAttestation> {
        self.source_get_attestations(list_id)
            .into_iter()
            .filter(|row| row.address == address)
            .collect()
    }

    pub fn observed_attestation_by_source_key(
        &self,
        key: &str,
    ) -> Option<&UnverifiedSourceAttestation> {
        self.attestations_by_ne_db_id.get(key)
    }

    /// Source country selector accepts active list ID 0 with nonempty
    /// metadata and uppercases it, even if the string is not ISO2.
    /// Returned height uses timestamp || null (zero => None).
    pub fn source_country_code_by_address(&self, address: &str) -> Option<SourceCountryCode> {
        self.attestations_by_ne_db_id
            .values()
            .find(|row| {
                row.address == address
                    && row.list_id == 0
                    && row.status == SourceAttestationStatus::Active
                    && row.metadata.as_ref().is_some_and(|s| !s.trim().is_empty())
            })
            .map(|row| SourceCountryCode {
                code: row
                    .metadata
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_uppercase(),
                reported_height: (row.timestamp != 0).then_some(row.timestamp),
            })
    }

    pub fn source_set_banlist(
        &mut self,
        codes: &[&str],
        height: u64,
    ) -> Result<(), ClearlistSourceError> {
        if codes.len() > MAX_SOURCE_ATTESTATIONS {
            return Err(ClearlistSourceError::TooManyAttestations);
        }
        for code in codes {
            source_text(code)?;
        }
        let list: Vec<String> = codes.iter().map(|s| (*s).to_owned()).collect();
        self.source_stored_banlist = Some((list.clone(), height));
        self.source_cached_banlist = list;
        Ok(())
    }

    pub fn source_cached_banlist(&self) -> &[String] {
        &self.source_cached_banlist
    }
    pub fn source_stored_banlist(&self) -> Option<&[String]> {
        self.source_stored_banlist
            .as_ref()
            .map(|(v, _)| v.as_slice())
    }
    /// The actual JS getBanlist() calls findAsync() and then reads the
    /// ARRAY's .data field. It is undefined on normal arrays, regardless of
    /// whether a stored banlist exists. Do not upgrade this to authority.
    pub const fn source_get_banlist_bug(&self) -> Option<&[String]> {
        None
    }

    pub fn source_store_merkle_root(
        &mut self,
        list_id: u32,
        root: &str,
        admin: &str,
        height: u64,
    ) -> Result<(), ClearlistSourceError> {
        source_nonempty(admin)?;
        if !valid_hex_digest(root) {
            return Err(ClearlistSourceError::InvalidMerkleProof);
        }
        let key = format!("merkle:{list_id}:{root}");
        if !self.attestations_by_ne_db_id.contains_key(&key)
            && self.attestations_by_ne_db_id.len() >= MAX_SOURCE_ATTESTATIONS
        {
            return Err(ClearlistSourceError::TooManyAttestations);
        }
        self.attestations_by_ne_db_id.insert(
            key,
            UnverifiedSourceAttestation {
                list_id,
                address: admin.to_owned(),
                metadata: None,
                xpub: None,
                root: Some(root.to_owned()),
                status: SourceAttestationStatus::Active,
                timestamp: height,
                revoke_reason: None,
                revoked_by_list_id: None,
            },
        );
        self.xpubs_by_merkle_root
            .entry(root.to_owned())
            .or_default();
        Ok(())
    }

    /// The global xpub map is root-keyed; two lists with an identical root
    /// share offchain entries. Proofs bind text to the root but not the
    /// address derivation, list administrator or chain inclusion.
    pub fn source_register_xpub_for_root(
        &mut self,
        root: &str,
        xpub: &str,
        proof: &[SourceMerkleStep],
    ) -> Result<bool, ClearlistSourceError> {
        if !self.xpubs_by_merkle_root.contains_key(root) {
            return Ok(false);
        }
        if !source_verify_xpub_merkle_proof(xpub, proof, root)? {
            return Ok(false);
        }
        let set = self
            .xpubs_by_merkle_root
            .get_mut(root)
            .ok_or(ClearlistSourceError::MerkleEvidenceMissing)?;
        if set.len() >= MAX_SOURCE_XPUBS && !set.contains(xpub) {
            return Err(ClearlistSourceError::TooManyXpubs);
        }
        set.insert(xpub.to_owned());
        Ok(true)
    }

    pub fn observed_merkle_xpubs(&self, root: &str) -> Option<&BTreeSet<String>> {
        self.xpubs_by_merkle_root.get(root)
    }

    /// Any claimed derived-address witness needs a separate vetted BIP32
    /// derivative and script/address-family audit. If source has active
    /// xpub/root rows, absence from the direct DB is NOT a negative verdict.
    pub fn source_direct_or_derived_unverified(
        &self,
        list_id: u32,
        address: &str,
    ) -> Result<bool, ClearlistSourceError> {
        if self.source_is_address_directly_attested(list_id, address) {
            return Ok(true);
        }
        let has_derived_candidates = self.attestations_by_ne_db_id.values().any(|row| {
            row.list_id == list_id
                && row.status == SourceAttestationStatus::Active
                && (row.xpub.is_some()
                    || row.root.as_deref().is_some_and(|root| {
                        self.xpubs_by_merkle_root
                            .get(root)
                            .is_some_and(|xpubs| !xpubs.is_empty())
                    }))
        });
        if has_derived_candidates {
            Err(ClearlistSourceError::DerivationUnavailable)
        } else {
            Ok(false)
        }
    }

    pub const fn require_verified_clearlist_mutation(&self) -> Result<(), ClearlistSourceError> {
        Err(ClearlistSourceError::HistoricalClearlistAuthorityUnavailable)
    }
    pub const fn require_consensus_accepted_type8(&self) -> Result<(), ClearlistSourceError> {
        Err(ClearlistSourceError::AcceptedSourceTransitionUnverified)
    }
    pub const fn require_signed_channel_revision(&self) -> Result<(), ClearlistSourceError> {
        Err(ClearlistSourceError::CanonicalChannelRevisionUnspecified)
    }
}
