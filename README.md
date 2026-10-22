# TradeLayer Rust

A staged Rust rebuild of the [TradeLayer protocol](https://github.com/patrickdugan/tradelayer.js).

**Status:** wire-format reconnaissance. This repository does not process funds, accept network transactions, validate consensus, maintain balances, or handle wallet keys.

## Quick start

Install [Rust](https://rustup.rs), clone this repository, then run:

```sh
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
```

The toolchain is pinned in `rust-toolchain.toml` and the crate has **zero third-party dependencies**. `Cargo.lock` is committed.

## Wire parser (current step)

The library exposes `tradelayer_core::wire::parse_frame` for the historical `tl` ASCII envelope and `parse_typed` for preliminary transaction decoding (types 0, 1, 2, 11, 12).

The decoder returns typed **syntax** only. Validity checks, activation, money semantics, and OP_RETURN script parsing are separate follow-up work. It rejects recognized double-prefix ambiguity instead of silently processing it.

[Wire-format analysis](docs/WIRE_FORMAT.md) describes the historical JavaScript discrepancies. [The fixture generator](scripts/generate_vectors.mjs) creates the repository's byte-level test vectors with Node's built-in APIs and no npm packages:

```sh
node scripts/generate_vectors.mjs --check
```

These vectors were reconstructed from pinned historical source expressions, pending differential testing against the actual legacy runtime and on-chain samples.

## Working rules

- Port one verified protocol rule at a time from the [JavaScript reference](https://github.com/patrickdugan/tradelayer.js).
- Treat legacy JavaScript output as historical evidence, not an unquestioned specification.
- Use checked integer units for financial quantities; avoid floats in consensus.
- Reject unsupported transactions explicitly.
- Keep consensus independent of networking, storage, wallets, and RPC.
- Require adversarial vectors and state invariants before expanding scope.

See [MIGRATION.md](MIGRATION.md) for milestones. License selection and protocol compatibility require review before production development.
