# Module 01/49: pinned activation.js semantics in Rust

Baseline: [TradeLayer-Rust bee4aacf](https://github.com/patrickdugan/TradeLayer-Rust/commit/bee4aacf6da5cb5d9fe64f5409e95b9f21ac68db) and historical source [tradelayer.js@dca0888b src/activation.js](https://github.com/patrickdugan/tradelayer.js/blob/dca0888b827c5bc28fa9189bc9e1d507f5c5670c/src/activation.js).

**Scope: deterministic source data semantics, not authenticated activation or full file migration.** This implementation is deliberately isolated from the existing Rust registry/ledger/clearing trust paths.

## Coverage

| JavaScript method/behavior | Rust status and distinction |
|---|---|
| initializeTxRegistry(): 36 names, initially inactive | **Ported** literally to LEGACY_ACTIVATION_NAMES and default slots. Historical names for 31 and 32 conflict with the actual dispatcher's names and must NEVER select protocol handlers. |
| updateAdminAddress() and getAdmin() | **Ported** all six BTC/LTC/DOGE mainnet/testnet literal addresses to legacy_admin_address. They are only claimed labels, not a signature or chain configuration proof. |
| isSenderGenesisAdmin() | **Ported** historical hardcoded Litecoin-testnet address comparison, even though it does not depend on the current chain. |
| activate(txType, block, codeHash), nonzero types | **Ported for a bounded string-valued, claimed-actor subset**. Known types 1–35 mark active and overwrite the activationBlock on reactivation; supplied nonempty codeHash overwrites, missing/empty codeHash preserves the old value. No canonical JSON/database changes. |
| activate(0, ...): native genesis | **Blocked**: historical implementation calls Vesting.initializeTokens, creates properties, insurance allocations, balances and clearlist defaults, and sets a persistent initialization flag. Never record a genesis activation without reproducing the whole atomic initialization and testing replay/rollback. |
| isTxTypeActive() | **Ported source behavior**: returns the stored active flag without checking block height, unlike the stricter Rust registry's check_active temporal guard. Never use this projected flag as authorization. |
| areActivationsAboveThreshold() | **Ported** strict greater-than 90% over the fixed 36 registry slots: 32 active => false; 33 => true. |
| getActivationBlock() | **Ported** odd JavaScript falsy behavior: activation height 0 returns null/None despite an active flag. |
| checkActivationBlock() | **Ported** distinct undefined-only behavior: stored height 0 returns Some(0). |
| constructor/getInstance singleton, init(chain), client getChain/getTest, delay | **Not production-ported**: the Rust projection uses a chain-specific explicit constructor. The historical global singleton could leak state across three network profiles, so it is not copied to consensus. Chain/client lifecycle and time delays need a separate runtime adapter. |
| saveActivationsList() / loadActivationsList() | **Not ported**: historical code reads/writes JSON under the NeDB activations collection and silently resets missing/broken data to inactive defaults. A versioned authenticated snapshot/migration and rollback contract is still required; reloading a malformed tip database cannot supply historical proofs. |
| isValidJSON, isValidJavaScript, minifyJavaScript | **Not ported as trusted validators**: the latter two historical methods are placeholders that return true or the input string; copying them into a security-sensitive Rust acceptance pipeline would be a bug. |
| getNextTxTypeId(), saveNewTransaction() | **Not ported**: empty legacy stubs with no verified semantic contract. |

## Implemented source-bound diagnostic state

New src/legacy_activation_audit.rs has explicit per-network registry projections with 36 private slots, initial false flags, source-compatible heights/hash replacements, fixed network admin labels and strict 33-of-36 threshold. Inputs are caller-supplied claimed network, sender, height, tx position, block ID, transaction type and optional ASCII hash. Batch mutation clones the state and commits only after *all* source-compatibility checks succeed.

The offline adapter adds extra safety checks absent from the original JavaScript: a bounded event batch, bounded printable ASCII hash fields, specified nonzero block IDs, chronological positions and same-height block consistency. These are **research fail-closed restrictions, not source consensus rules**. Unverified sender strings, block IDs and event ordering are NOT authenticated by this data model.

**Absolute trust boundary**: require_authenticated_legacy_activation() always returns HistoricalActivationAuthorityUnavailable; require_trusted_protocol_activation() always returns TrustedProtocolActivationUnavailable. The source-like active flag never dispatches a transaction or mutates a real Ledger, registry, UTXO, channel, oracle or clearing position. Genesis type zero is refused atomically and cannot be used to allocate new native properties or insurance funds.

The existing src/registry.rs RegistryAction::Activate remains untouched and intentionally differs from legacy source in rejecting already-active types, requiring verified genesis configuration by an external adapter, blocking id 34 semantic conflicts, and using a strict temporal gate for type-specific registry actions. Do not treat the new source diagnostic as a replacement for those stronger conditions.

## Independent parity fixtures

fixtures/legacy_activation_js_names.tsv freezes the 36 pinned source labels. scripts/audit_legacy_activation_parity.cjs independently reproduces the relevant vanilla JS expressions for activation, reactivation, source activation-height getter vs checker, six admin address mappings, names and threshold. tests/legacy_activation_audit.rs asserts all source labels, 6 profiles, 0-height behavior, 33-of-36 boundary, sticky/nonempty hash replacements, reactivation, unknown type, wrong actor, malformed hash, mixed batch atomic rejection, same-height conflicting forks, event ordering, shallow source snapshots and permanent trust denial.

### Reproduce (offline, zero external Rust crates)

    python3 scripts/check_rust_supply_chain.py
    node scripts/audit_legacy_activation_parity.cjs
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test legacy_activation_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen

## What remains to mark activation.js fully ported

1. A genuine independently verified type-0 native initialization transition, exact TallyMap, vesting, property IDs, insurance allocations and no-repeat inflation guarantee with atomic rollback.
2. A versioned, lossless historical activation-registry snapshot/replay importer and persistent storage; reject corrupt/missing history as authority instead of silently resetting.
3. Verified on-chain type-0 actor, activation payload decoding, source-version and fork/height/tx-order rules; network-specific user-signed first-input address/script and historic admin configuration.
4. Decide and ratify which legacy compatibility behavior becomes future consensus, especially repeat activation replacing historical height, type 31/32 name conflicts and the original JavaScript singleton. Preserve stricter Rust core protections unless intentionally revised.

**Next inventory file (not worked in this PR):** adjuster.js in alphabetic source order, or clearlist.js if following the earlier P0 migration table. Only this first module is addressed in the present turn.
