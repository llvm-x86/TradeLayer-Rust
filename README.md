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
