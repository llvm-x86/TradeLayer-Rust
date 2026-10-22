# TradeLayer Rust

A staged Rust rebuild of the [TradeLayer protocol](https://github.com/patrickdugan/tradelayer.js).

**Status:** bootstrap only. This repository does not yet parse on-chain transactions, validate consensus, maintain balances, connect to a node, or handle wallet keys. Do not use it to process funds.

## Quick start

Install [Rust](https://rustup.rs), clone the repository, and run:

```sh
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
```

The toolchain is pinned in `rust-toolchain.toml`. The starter crate has **zero third-party dependencies**. Commit `Cargo.lock` for repeatable builds when dependencies are added.

## Working rules

- Port one verified protocol rule at a time from the [JavaScript reference](https://github.com/patrickdugan/tradelayer.js).
- Treat the JS implementation as a source of *test fixtures*, not an unquestioned specification.
- Use integer units for financial amounts; avoid floating point in consensus.
- Reject unsupported transactions explicitly.
- Keep consensus independent of networking, storage, wallets, and RPC.
- Require deterministic tests and adversarial fixtures before expanding scope.

See [MIGRATION.md](MIGRATION.md) for the first milestones.

## Repository layout

```text
src/lib.rs          Minimal typed core boundary
Cargo.toml          Library crate, no dependencies
Cargo.lock          Locked dependency graph
rust-toolchain.toml Pinned compiler and tools
.github/workflows/ci.yml  Format, lint, and unit tests
MIGRATION.md        Migration milestones
```

License selection and protocol-compatibility requirements will be recorded before production development.
