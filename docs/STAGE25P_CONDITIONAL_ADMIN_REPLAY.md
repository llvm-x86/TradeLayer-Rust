# Stage 25P: Conditional type-7/8/9 clearlist administrator replay

**Offline historical-source characterization only.** This stage cannot authorize a list change, channel signature, settlement or liquidation. Its Bitcoin header segment uses a caller-selected fake regtest-like anchor and synthetic raw transactions without valid historical UTXOs.

Baseline: [Stage25O main ec11722c](https://github.com/patrickdugan/TradeLayer-Rust/commit/ec11722cf627c2912b8023c9cbe16e36e3f5c853). Pinned source: [tradelayer.js@dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c), notably src/validity.js:881–1098; src/clearlist.js:9–30,85–98,112–170,415–449; src/logic.js:194–205,851–920 and src/txEncoder.js:198–237.

## Historical source findings

Type 7 creates a clearlist by taking the next available ID from the maximum of the loaded list registry. Its encoded transaction carries **no list ID**. The source stores properties called id, admin and backup in the collection named **clearlists**. Its generated ID cannot be established from the raw transaction alone.

Type 8's validation instead calls ClearList.getList(id), which reads **attestations**, and checks for properties named adminAddress and backupAddress. An ordinary Type7 list is in **clearlists**, with properties called admin and backup. The Type8 update then writes clearlists via ClearList.updateAdmin. This is an unresolved source-level validator / updater discrepancy. Even a correctly authenticated Type8 raw transaction and matching intended administrator might have been rejected by this source revision. Stage25P only projects what a rotation *would* do if an externally validated source rule admitted it and always flags such a transition.

Type 9 checks the list's current primary admin via ClearList.getClearlistById (which reads clearlists), using adminAddress or admin. Backup administrator status alone does not satisfy the Type9 admin rule. A revoke also requires direct active membership. Historical activation, source sender from the first input's parent UTXO, list-zero country/BANLIST and derived xpub attestations remain separate.

## Implementation

New src/channel_admin_replay_audit.rs composes the existing Bitcoin relative-header and Merkle verifier, strict raw transaction OP_RETURN parser and Stage25M as-of direct-list checker. It accepts an explicit **untrusted initial next-list ID**, claimed original senders, claimed active type flags and a bounded set of source-like raw Type7/8/9 events under one locally linked caller-anchored header segment. A Type4 or Type19 target is independently checked in that segment.

- Type7 creates a projected list with an externally declared exact ID; missing or incorrect next IDs refuse.
- Type8 checks that exactly the whitelist target was selected and that the *claimed* sender matches the projected primary or backup. A projected update records a **Type8 lookup ambiguity** instead of claiming historical validation.
- Type9 checks that the claimed sender matches the projected current primary; it records direct issue/revoke events, rejecting revocations with no previous active matching membership.
- Wrong source type, malformed or nested payload, wrong Merkle proof, wrong height/tx index/block ID, future or duplicate source transaction, incorrect ordering, failed reported activation and cross-network attempts refuse.
- Type4/19 projected direct-list queries reuse Stage25M's source rule characterization. Neither they nor the raw Merkle evidence proves historical TradeLayer validity.

The returned UnverifiedProjectedAdminHistory stores only private-field observations and explicitly marked projections. Its require_authenticated_historical_admin() always yields HistoricalSourceAuthorityUnavailable, require_verified_type8_transition() always yields Type8ValidatorLookupUnresolved, and require_canonical_signed_channel_revision() always yields CanonicalSignedRevisionUnspecified. Stage25G and all trusted type3/23/31 economic authorization remain default-deny. No Ledger, clearing, reserve or channel execution API is modified.

## Frozen synthetic fixture

fixtures/channel_admin_789_merkle_synthetic.tsv contains seven fabricated Bitcoin-style transactions in one fake regtest-like block at claimed height 101, anchored to a caller-selected hash of 0x66 bytes.

| TX index | Payload | Purpose |
|---|---|---|
| 0 | tl7backupB,ListOne,https://example.invalid,note | Conditional creation of list 1 by claimed adminA |
| 1 | tl90,1,alice,OK | Conditional attestation of Alice by claimed adminA |
| 2 | tl40,1 | Opaque Type4 target before projected rotation |
| 3 | tl8adminC,1,0,0,1,0 | Hypothetical primary rotation from adminA to adminC |
| 4 | tl91,1,alice,OK | Conditional revocation by claimed adminC |
| 5 | tl40,1 | Opaque Type4 target after projected revoke |
| 6 | tl90,1,alice,OK | Another synthetic candidate for adversarial source checks |

The fake inputs and simple address labels make this a **source-expression and ordering fixture**, not Bitcoin or TradeLayer consensus-valid history. The independent Python stdlib test recreates all raw bytes, TXIDs, odd-leaf Merkle siblings and the fake header/PoW and exercises tampering. Node independently reproduces the Type8 source lookup inconsistency with mock clearlists/attestations maps. Rust tests reject wrong list ID, wrong claimed sender role, missing revoke predecessor, stale or inactive event, bad fork/position and Merkle substitution. Zero new third-party Rust crates.

## Required next proof

Authenticate real historical Bitcoin/Litecoin raw parent UTXOs, scripts and first-input owners; full accepted block/transaction order; Type7/8/9 activation and list state at that exact history; and the source-compatible Type8 acceptance/version rule using real fixtures. Country, xpub, banlist, admin changes and revocations need independently replayable state. Full Bitcoin PoW/chainwork/full-block/BIP141/UTXO evidence remains separate. The channel's signed state bytes, monotone revision, supersession and invalidation dispute/reorg rules need an explicit ratified protocol before any economics.

### Offline checks

    python3 scripts/audit_type789_merkle_vectors.py
    node scripts/audit_type789_source_admin.cjs
    cargo test --frozen --test channel_admin_replay_audit
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen
    cargo build --all-targets --release --frozen
