# Rust crate admission and supply-chain policy

## Decision: zero third-party crates inside the TradeLayer consensus kernel

**Security posture: locked at `tradelayer-core@0.1.0`, Rust 1.90.0,
zero external Rust crates.** This is enforced by the standard-library-only
`scripts/check_rust_supply_chain.py` and `.cargo/config.toml`.

The dependency graph at the time of this change contains only
`tradelayer-core 0.1.0`, and `Cargo.lock` format v4 records no third-party
checksum because it contains no third-party package.

The policy deliberately avoids pretending that a pinned version is risk-free.
Pinning reproducibly selects known bytes; it cannot prove those bytes are
nonmalicious, prevent a toolchain compromise, or authenticate the operating
system image. A user with write privileges can change a validator and its
policy in the same branch. Require repository protection as well as CI.

## Component specification: no external crate needed for the economic port

| Logical component | Approved dependencies now | Reason |
| --- | --- | --- |
| `tradelayer-core`: issue/send/registry/spot | Rust `core`/`std` only | Single owner for ledger and replay; deterministic state |
| `clearing_math`, `funding_audit`, `clearing_cadence` | Rust `core`/`std` only | Checked integer math, explicit rounding, replay scheduling |
| Future derivatives positions and PnL | Rust `core`/`std` only | `i128`/`u128` checked math; 256-bit arithmetic only after a formal range requirement |
| Future matching book and order/cancel routing | Rust `std::collections` only | `BTreeMap`/`BTreeSet`, `Vec`, bounded replay; deterministic total ordering |
| Funding, insurance and liquidation decisions | Rust `core`/`std` only | Atomic state machine, explicit rounding and loss waterfall |
| Event journal, state roots and snapshots | Existing internal SHA-256 and checked codecs | Reuse audited local primitives, preserve cross-chain namespace separation |
| Adversarial test generators | Existing Node BigInt / Python stdlib scripts, no Rust crates | Hermetic fixed seeds, independently checked arithmetic |
| Litecoin/Bitcoin/Dogecoin chain input | Existing independent evidence tools and trusted-node boundary | No network/transaction signing in the economic state machine |

This plan means no `serde`, `tokio`, `rand`, `chrono`, `clap`,
`bincode`, `anyhow`, `thiserror`, `bigdecimal`, `rust_decimal`,
`num-bigint`, or other convenience dependency is necessary for the initial
consensus port. It does **not** suggest writing custom signature verification
or TLS; those are external security primitives and belong outside this
kernel after dedicated review.

### Possible non-consensus adapters (NOT approved or pinned yet)

| Future separate package | Candidate families for evaluation | Trust restriction |
| --- | --- | --- |
| Durable database adapter | `redb` **or** `rusqlite`, not both by default | Benchmark crash recovery, write ordering, rollback and page checksum. Inspect `build.rs`, native libraries and transitive graph before admission |
| Bitcoin-family cryptographic verifier | Established `secp256k1` or `k256` implementations | Prefer trusted full-node verification first; never implement bespoke elliptic-curve code. Audit `unsafe`, FFI, build scripts and timing properties |
| Local authenticated-node transport | `serde_json` plus a deliberately selected local transport, if stdlib cannot meet requirements | Loopback only. No wallet credential logging; keep network code outside state transition crate |
| Developer CLI and telemetry | `clap`, `tracing` or equivalents only if demonstrated necessary | Diagnostic adapter only, never consensus dependency |
| Persistent cryptographic proof verifier | Scheme-specific published verifier, once economic proof format is specified | Full source audit and trusted setup/proof-domain validation. Never equate metadata hashing with a proof |

**No exact versions are specified for these candidates:** none has been
reviewed, admitted, or needed by this repo yet. Publishing arbitrary version
pins would give false confidence. The first approved proposal must carry
reviewed exact crate versions and artifact checksums *as part of its PR*.

## Current hard gates

The validator refuses:

- Extra `Cargo.toml` or `build.rs` files and vendored source while
  the declared zero-dependency policy is active
- Changes to `[dependencies]`, `[dev-dependencies]`,
  `[build-dependencies]`, target-scoped tables, patches, replacements,
  workspaces, features, direct binaries or package build settings
- Anything in `Cargo.lock` beyond the pinned first-party package
- A different `rust-toolchain.toml`, toolchain profile or component set
- Non-offline `.cargo/config.toml`, credential files or additional config
- Actual `cargo metadata --frozen` graphs containing more than the root
  package, a build script or a procedural macro, or using a wrong Rust/Cargo
  version

This is a **deliberately frozen baseline**. Upgrading the package version,
adding a workspace or admitting a dependency is a security-policy change and
must be carried by a reviewed, separate PR.

```sh
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
cargo metadata --frozen --format-version 1
cargo test --frozen
cargo clippy --all-targets --frozen -- -D warnings
cargo build --all-targets --release --frozen
```

`--frozen` is `--locked` plus `--offline`; both are needed, not
merely `--locked`. The repository also sets `[net] offline = true` and CI
exports `CARGO_NET_OFFLINE=true`. Cargo builds and resolution cannot fetch
new crates, but this **does not** prevent tools outside Cargo (rustup,
`apt`, Node scripts, arbitrary build scripts if admitted in the future)
from accessing the network. The validator blocks `build.rs` in this baseline.

## How to admit the first dependency, if truly necessary

1. **Separate the crate**: keep `tradelayer-core` dependency-free where
   feasible. Create the smallest adapter API; passing parsed data into the
   economic state machine must not authorize a ledger operation.
2. **Demonstrate necessity**: assess a standard-library alternative and an
   external-library alternative with runtime/memory/complexity comparisons.
   For crypto, prefer an independently reviewed implementation.
3. **Review the full transitive closure**, not just top-level dependencies:
   name, exact version, published .crate SHA-256, license, maintainer,
   provenance, optional/default features, `unsafe` and FFI inventory,
   `build.rs`/proc-macro execution and target-specific dependencies.
4. **Pin exact versions** in Cargo manifests (`=X.Y.Z`, no caret, star,
   branch, tag or unpinned git ref), choose explicit features with
   `default-features = false`, and commit the fully resolved `Cargo.lock`.
   A lockfile alone covers normal `cargo build --locked`, but a manifest
   equality pin makes the intent visible and protects alternate resolution.
5. **Vendor reviewed source** with a pinned Cargo tool:
   `cargo vendor --locked vendor`, commit `vendor/` and configure
   `[source.crates-io] replace-with = "vendored-sources"` and
   `[source.vendored-sources] directory = "vendor"`. Check registry
   `Cargo.lock` checksums and archive hashes against *independently
   obtained trusted digests*. `.cargo-checksum.json` checks accidental
   modifications but is **not** itself a malicious-source defense.
6. **Policy review**: submit the crate approval manifest listing **every**
   transitive package and checksum, review selected versions and source,
   licensing, RustSec advisories, `cargo-vet` audits and
   `cargo-deny` source restrictions. Never `cargo install` an unpinned
   auditing utility in the release CI job. Run review tools in a separate
   audited, version-pinned tooling environment.
7. **Reproducible build**: the reviewed revision must pass
   `cargo test --frozen`, `cargo build --frozen --release`, and
   a separately provisioned, network-isolated clean-machine build with
   an independently confirmed toolchain and OS/container image digest.
   Record an SBOM, source commitment, release binary hashes and observed
   build provenance.
8. **Security approval**: update the now-deny-all validator deliberately,
   in the same dedicated admission PR, with negative mutation tests that
   reject unauthorized dependency/version/checksum/feature changes.
   Require a trusted reviewer separate from the contributor when possible.

An admitted crate that changes version, features, license, checksum, owners,
`build.rs` behavior or transitives re-enters the review process. No blanket
Dependabot or automated crate upgrades merge into the consensus branch.

## Repository controls outside Cargo

The workflow uses a commit-SHA-pinned `actions/checkout`, sets
`persist-credentials: false`, requests only `contents: read`, and avoids
unreviewed `cargo install` steps. The `.github/CODEOWNERS` file identifies
critical paths; it is **not enforceable by itself**. Enable required status
checks and code-owner approvals in the repository's protected-branch/ruleset
settings, restrict force pushes and administrator bypass where practical,
and never supply signing or wallet secrets to PR workflows.

`ubuntu-24.04` is a mutable GitHub runner image, and `rustup toolchain
install 1.90.0` still downloads a toolchain. An actual release-grade build
needs a digest-pinned builder image or independently verified toolchain
artifacts and separate provenance attestation. A green CI result alone
cannot eliminate compromise of CI providers, repository owners, the
standard library/toolchain, or reviewed source.

The baseline minimizes one major class of risk rather than claiming to
remove all supply-chain risk. No production funds or keys are in scope.

**Reference:** Cargo [`--frozen`](https://doc.rust-lang.org/cargo/commands/cargo-build.html),
[vendoring](https://doc.rust-lang.org/cargo/commands/cargo-vendor.html),
[source replacement checksums](https://doc.rust-lang.org/cargo/reference/source-replacement.html),
and [RustSec's advisory database](https://github.com/RustSec/advisory-db).
