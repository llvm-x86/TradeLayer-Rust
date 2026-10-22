# TradeLayer Rust

A staged Rust rebuild of the [TradeLayer protocol](https://github.com/patrickdugan/tradelayer.js).

**Status:** draft wire format and deterministic in-memory ledger prototype. This repository cannot process real funds, accept network transactions, validate chain consensus, or handle wallet keys.

## Quick start

Install [Rust](https://rustup.rs), clone this repository, then run:

```sh
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
```

The toolchain is pinned in `rust-toolchain.toml` and the crate has **zero third-party dependencies**. `Cargo.lock` is committed.

## Wire parser (current step)

The legacy `tradelayer_core::wire` module remains a **read-only diagnostic parser** for historical `tl` ASCII payloads. It must never silently dispatch transactions to the new consensus engine.

The separate `tradelayer_core::wire_v1` module proposes an unambiguous binary `TLR` + version + numeric type-byte envelope. It implements type-2 direct sends and send-all with integer atomic units. See [proposed wire v1](docs/WIRE_V1.md). **The format is not activated on any chain.**

The decoders return typed **syntax** only. Validity checks, activation, money semantics, and OP_RETURN script parsing are separate follow-up work. The legacy parser flags double-prefix ambiguity; v1 rejects legacy ASCII input altogether.

[Legacy wire-format analysis](docs/WIRE_FORMAT.md) describes the historical JavaScript discrepancies. [The fixture generator](scripts/generate_vectors.mjs) creates the repository's byte-level test vectors with Node's built-in APIs and no npm packages:

```sh
node scripts/generate_vectors.mjs --check
node scripts/generate_v1_vectors.mjs --check
```

The legacy vectors reconstruct pinned historical source expressions. The new v1 vectors test a proposed serialization and are not evidence of historically accepted transactions.

## In-memory accounting prototype

The independent [`tradelayer_core::ledger`](src/ledger.rs) module now models fixed/managed property supply and exact available balances, with checked `u128` atomic-unit arithmetic. Ordered batches commit atomically, and replay IDs are tracked. It has no signing, chain authentication, persistent storage, or live consensus integration.

See [ledger invariants and limits](docs/LEDGER.md) and the [integration tests](tests/ledger_invariants.rs). Draft wire-v1 direct sends can be decoded and explicitly translated to a ledger transfer by a **trusted caller**; the module itself never infers sender authorization from a payload.

## Working rules

- Port one verified protocol rule at a time from the [JavaScript reference](https://github.com/patrickdugan/tradelayer.js).
- Treat legacy JavaScript output as historical evidence, not an unquestioned specification.
- Use checked integer units for financial quantities; avoid floats in consensus.
- Reject unsupported transactions explicitly.
- Keep consensus independent of networking, storage, wallets, and RPC.
- Require adversarial vectors and state invariants before expanding scope.

See [MIGRATION.md](MIGRATION.md) for milestones. License selection and protocol compatibility require review before production development.

## Authenticated block progression (prototype)

`tradelayer_core::chain::ChainState` accepts candidate blocks only through a `TrustedChainAdapter` implementation, whose job is to verify chain headers, raw transaction bytes, and the authorized actor for each ledger operation. The library enforces origin/operation binding, sequential blocks, atomic application, replay protection, reversible snapshots, and new-file checkpoint saves. It **does not provide a real adapter, signature verification, or a production consensus engine**.

See [chain-state design and remaining security work](docs/CHAIN_STATE.md).

## Offline transaction replay (experimental)

The dependency-free `read_only` module parses Bitcoin-style raw transactions, checks txid/wtxid fingerprints, and identifies single-push OP_RETURN messages with exact `tl` or `TLR` markers. It deliberately does **not** implement `TrustedChainAdapter`, validate chain inclusion, or authorize a state transition.

```sh
cargo run --locked --bin tradelayer-scan -- fixtures/real_transactions.tsv
cargo run --locked --bin tradelayer-scan -- fixtures/synthetic_transactions.tsv
```

The real-byte corpus includes the Bitcoin genesis transaction, a published non-SegWit Bitcoin transaction, and a Bitcoin regtest SegWit OP_RETURN example. Only the last carries OP_RETURN, and none is established as a TradeLayer transaction. Synthetic fixtures explicitly exercise the TradeLayer markers. See [READ_ONLY_REPLAY.md](docs/READ_ONLY_REPLAY.md) for sources, limitations, and the eventual authenticated chain-replay requirements.

## Litecoin archival collection and differential replay (research)

The standard-library-only Python collector reads confirmed transactions from an
**unpruned local Litecoin Core node**. It verifies transaction fingerprints,
full-block transaction Merkle roots, parent linkage, raw OP_RETURN scripts,
and active-chain membership at the end of a bounded scan. The node supplies
chain consensus verification; the collector itself cannot authenticate
signature ownership or approve a TradeLayer transaction.

```sh
python3 scripts/collect_litecoin_rpc.py --network test \
  --start-height 3348377 --end-height 3348528 \
  --output archive/testnet.tsv --manifest archive/testnet.json
cargo run --locked --bin tradelayer-scan -- archive/testnet.tsv
cargo run --locked --bin tradelayer-legacy-fields -- archive/testnet.tsv > archive/rust-fields.tsv
```

The optional [legacy JavaScript comparison](docs/ARCHIVE_REPLAY.md) executes
`src/txDecoder.js` from a trusted, pinned checkout in a separate networkless
environment and reports field-level disagreements. The cited testnet heights
are a **search lead from documentation**, not confirmed on-chain TradeLayer
transactions. This repository does not yet contain validated Litecoin TradeLayer
chain-history fixtures.

## zkTL Cairo / STWO compatibility (read-only)

The `zk_batch` module now independently verifies the **JSON SHA-256 metadata commitments** from [zkTL](https://github.com/patrickdugan/zkTL), using a fixture derived from its 26-receipt checkpoint. It checks ordered receipt hashes, transition roots, batch hashes and state-root chaining. `src/protocol.rs` inventories all 36 historical transaction IDs and records the unresolved type-34 collision. **Metadata consistency does not verify any Cairo/STWO proof or economic transition.**

```sh
python3 scripts/export_tlzk_batch.py ../zkTL/artifacts/cairo_inputs/batch_checkpoint_latest.json --out /tmp/tlzk.tsv
cargo run --locked --bin tradelayer-zk-inspect -- /tmp/tlzk.tsv
```

See [Cairo integration and blocked trust boundaries](docs/ZK_CAIRO_INTEGRATION.md).

## Registry transitions (isolated Rust prototype)

The dependency-free [`registry` module](src/registry.rs) ports bounded state
transitions for legacy transaction types 0, 7, 8, 9, 13, 14 and 15:
activations, whitelists/admin rotation, attestations, and oracle lifecycle.
It uses monotonic block heights, checked IDs, exact 1e-4 oracle quote units,
replay protection, and atomic batches. All transaction actors and heights
**must come from a future authenticated chain adapter**; these APIs do not
verify signatures, UTXO ownership, or proof-of-work. The module does not yet
persist or compose with the balance ledger.

[Registry design and incompatibilities](docs/REGISTRY_PORT.md) explains
unported self-certification, banlist/xpub logic, token-admin migration and
ZK proof-verification gates. Type 34 stays blocked by the legacy/zkTL conflict.
Tests: `cargo test --locked --test registry_transitions`.

## Unified registry and ledger state (simulation-grade)

The [`execution` module](src/execution.rs) coordinates activation,
fixed/managed token issuance, sends, mint/burn, whitelists, attestations,
and oracle state under **one transaction replay set** and atomic batches.
This is a local, typed protocol execution model for tx0,1,2,7–9,11–15.
Its input is explicitly named `UnverifiedTx` because it supplies claimed
actors, heights and operations; **nothing here verifies signed chain input**.
The coordinator is separate from `ChainState` and carries no persistent
checkpoint format yet.

[Implementation and trust boundaries](docs/UNIFIED_EXECUTION.md).
Run `cargo test --locked --test protocol_execution` for cross-module
rollback and global replay checks.

## Observed Bitcoin-mainnet header forks (Stage 18, audit-only)

The dependency-free `bitcoin_forks` module now retains a bounded graph of
individually validated Bitcoin-mainnet headers. It can select the greatest
cumulative-work tip **among observed headers**, preserve equal-work incumbent
choices, and report the common ancestor and attach/detach paths when a locally
stronger branch appears. A complete segment commits atomically after Stage 17
SHA256d PoW, mainnet difficulty, MTP and exact chainwork checks.

The root's provenance stays explicit. A caller-supplied checkpoint is still
untrusted, and an observed greatest-work tip is **not** evidence that Bitcoin
Core considers it the active best chain. Block economics, UTXOs, scripts,
external clocks, Scrypt chains and Cairo/STWO verification remain outside this
module. The settlement gate stays default-deny.

```sh
cargo test --locked --test bitcoin_forks
python3 scripts/audit_bitcoin_forks.py
```

See [fork selection and trust boundary](docs/BITCOIN_FORK_SELECTION.md).

## Stage 19: offline spot exchange (type 5 and 6 research subset)

The new `spot` module models queued token-for-token orders, deterministic
same-block ordering, per-order simulated escrow, rational bid/ask price
comparison, checked atomic two-asset fills and owner-scoped cancellation.
It verifies exact reserve balances against the existing `Ledger` conservation
invariant and runs 8 independent Node BigInt reference scenarios.

Legacy JavaScript **inverts the sell price's units** relative to the buy side.
This model proposes a consistent integer quote/base convention, explicitly
labelled a migration design change. Type 3 LTC-for-token transfers, post/stop
triggers, restricted assets and unauthenticated order inputs remain blocked.
The engine is isolated from the unified protocol replay and checkpoint
serializer; it must not be connected to real funds.

```sh
cargo test --locked --test spot_exchange
node scripts/audit_spot_vectors.cjs --check
```

See [Stage 19 source-parity findings](docs/SPOT_EXCHANGE.md).

## Unified chain replay with combined checkpoints (experimental)

`tradelayer_core::unified_chain::UnifiedChainState` brings the atomic ledger
and registry prototype under a single reversible block-tip coordinator. It
retains transaction replay IDs, oracle/whitelist/activation state and balances
together during rollback and checkpoint recovery.

The checkpoint formats are versioned binary TLUC/TLPS/TLRG, with canonical
re-encoding checks and CRC32 for accident detection. Loading requires an
independently configured chain ID, anchor, protocol administrator and retention
policy.

A `TrustedProtocolAdapter` interface accepts raw L1 bytes and supplies
authenticated protocol operations. **No real adapter is implemented here.**
Do not use the synthetic tests or supplied IDs as evidence of signer ownership,
a valid block, or a ZK proof. This is still offline prototype code.

[Architecture, security boundaries and test plan](docs/UNIFIED_CHAIN_CHECKPOINTS.md).
