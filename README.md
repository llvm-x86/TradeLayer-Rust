# TradeLayer Rust

A staged Rust rebuild of the [TradeLayer protocol](https://github.com/patrickdugan/tradelayer.js).

**Status:** wire-format reconnaissance plus a proposed versioned binary envelope. This repository does not process funds, accept network transactions, validate consensus, maintain balances, or handle wallet keys.

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

## Working rules

- Port one verified protocol rule at a time from the [JavaScript reference](https://github.com/patrickdugan/tradelayer.js).
- Treat legacy JavaScript output as historical evidence, not an unquestioned specification.
- Use checked integer units for financial quantities; avoid floats in consensus.
- Reject unsupported transactions explicitly.
- Keep consensus independent of networking, storage, wallets, and RPC.
- Require adversarial vectors and state invariants before expanding scope.

See [MIGRATION.md](MIGRATION.md) for milestones. License selection and protocol compatibility require review before production development.
