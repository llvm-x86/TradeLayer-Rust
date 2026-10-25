# 2/49: clearlist.js deterministic source parity

Baseline: [TradeLayer-Rust main 89d5d480](https://github.com/patrickdugan/TradeLayer-Rust/commit/89d5d4805c313baf3096307f56440cda1e4f701a). Pinned source: [tradelayer.js@dca0888b src/clearlist.js](https://github.com/patrickdugan/tradelayer.js/blob/dca0888b827c5bc28fa9189bc9e1d507f5c5670c/src/clearlist.js), 574 lines and 24 named methods.

**Status: deterministic source-behavior reconstruction, NOT a fully authorized historical transaction port.** This new read-only audit does not change trusted Rust registry, Ledger, channels, settlement or liquidation.

## Five source quirks preserved as explicit hazards

1. **List ID key types:** creation writes a numeric NeDB key; updateAdmin writes the same ID stringified. JavaScript Map distinguishes number 1 from string "1". updateAdmin changes the numeric in-memory entry and inserts a separate string-keyed DB row. getClearlistById calls loadClearlists, so the persisted numeric row can overwrite that in-memory change, restoring the old admin.
2. **Type8 validator mismatch:** getList(id) reads the attestations collection, while create/update write clearlists. Type8 expects adminAddress/backupAddress, while created records contain admin/backup. Type8 historical acceptance remains unproven. This separate diagnostic does not grant an admin update.
3. **Attestation collisions and revocation:** addAttestation / addAttestationWithXpub write one NeDB row keyed by address, replacing any earlier row for another list at the same address (including list0 self-cert). revokeAttestation first searches an active matching list, then falls back by address alone; it can revoke a row actually belonging to another list. The diagnostic exposes CrossListFallback rather than legitimizing the mutation. getAttestationHistory reads current rows; it is not an append-only event log.
4. **Banlist getter:** setBanlist updates a stored globalBanlist entry and cached in-memory banlist. getBanlist calls findAsync, which produces an array, then reads array.data (undefined on an ordinary array). This diagnostic keeps cached/stored values separate from the broken getter.
5. **xpub Merkle algorithm:** SHA256 hashes the UTF8 xpub to lowercase ASCII hex, then each parent SHA256 hashes the concatenated HEX TEXT, not binary digests. A singleton leaf is duplicated and paired. For five or six leaves, generateMerkleProof may skip the absent sibling at a higher odd-level node; verifyMerkleProof consequently fails on certain source-generated proofs. The xpub registry is keyed by root, not by list.

## Per-method coverage

| Source method(s) | Reconstructed deterministic behavior | Unverified / missing |
|---|---|---|
| createClearlist, createclearlist | Numeric-key upsert, max+1 ID, admin/backup/name/description data | Alias object overload/JS coercion and actual NeDB import |
| loadClearlists, getClearlistById | Numeric/string DB-key separation, cache reload and stale admin | Persistent storage / reorg truth |
| getList | Type8 wrong collection lookup for source-created rows | External numeric-key legacy attestations |
| verifyAdmin, updateAdmin | Cache-only admin comparison, primary/backup aliases and shadow string DB keys | Source Type8 validation, cryptographic actor |
| getNextId | Maximum ID from loaded numeric and string cache | Arbitrary noncanonical JS parseInt fields |
| addAttestation, addAttestationWithXpub | Address-keyed upsert, list overwrite, optional xpub: prefix | Accepted type9 authority |
| deriveAddressesFromXpub | **Not ported** | Verified BIP32 and exact Litecoin/Bitcoin address family |
| isAddressInClearlistOrDerived | Direct membership; refuses to infer absence when unverified derived candidates exist | Derived BIP32 membership |
| storeMerkleRoot, registerXpubForRoot | Root entry, root-global deduplicated xpub registration | Authenticated root/chain/list admin |
| verifyMerkleProof, buildMerkleTree, generateMerkleProof | Exact ASCII-hex SHA256 tree, odd padding and generator omission | Cryptographic production audit |
| revokeAttestation | Matching direct revoke, plus wrong-list fallback hazard | Original type9 validator authorization |
| getCountryCodeByAddress | Active list0 string metadata, trim+uppercase, zero-height as null | Object metadata variant, validated country |
| setBanlist, setBanList, getBanlist | Cached/persisted banlist and buggy findAsync array-data read | Accepted banlist policy |
| getAttestations, getAttestationHistory, isAddressInClearlist | Current rows filtered by list, address and active status | Lossless historical event journal |

## Implementation boundaries

New src/legacy_clearlist_audit.rs exposes a private-field, in-memory UnverifiedLegacyClearlistStore and read-only observations. It does not authenticate source actors or implement chain adapters. It includes bounded, strict ASCII and lowercase-hex restrictions beyond the permissive JS source; these are research restrictions, NOT historical consensus. Three authorization methods always fail: HistoricalClearlistAuthorityUnavailable, AcceptedSourceTransitionUnverified and CanonicalChannelRevisionUnspecified.

Independent, dependency-free Node source-expression parity script at scripts/audit_legacy_clearlist_source.cjs checks Map/NeDB-like key behavior, the wrong-list revoke, country/banlist getters and ASCII-hex SHA256 root oddities. Rust adversarial tests at tests/legacy_clearlist_audit.rs independently pin one-leaf root 82f8fcd70bcdf26565adb613a0dfe04955ee27054ab30f03e23160219aa73de9 and five-leaf root a9a5833ec88f79b211ba29e8d3bc936c02dba0ffaeaf2a48149304b679b63f3a, source-generated proof failures, deduplication and all three default-deny methods.

Stage25P is still separate and unmerged as a conditional raw type7/8/9 event-history branch. It cannot treat a projected Type8 admin update as source accepted. The already existing stricter Rust Registry deliberately corrects several legacy defects and is unchanged.

### Remaining to close this file as a production port

Authenticated Type7/8/9 raw transactions and historical sender/prevout, full list-id allocation and administration history; versioned resolution of Type8 collection/field/key mismatches; tested lossless NeDB snapshots and reorg rollback; BIP32 xpub derivation with script/network identity; country object metadata and banlist validation; and the separate signed channel-state revision/commitment invalidation protocol. No balance, insurer, oracle or liquidation authority is supplied by this work.

### Reproduction

    python3 scripts/check_rust_supply_chain.py
    node scripts/audit_legacy_clearlist_source.cjs
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test legacy_clearlist_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen
