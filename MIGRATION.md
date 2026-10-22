# TradeLayer JS -> Rust: staged migration

Historical reference: [patrickdugan/tradelayer.js](https://github.com/patrickdugan/tradelayer.js)

This repository is a prototype, not a deployable consensus node. Preserve the JavaScript implementation and its complete transaction history throughout the port. Work in small, reviewable steps.

## Step 0: Bootstrap (complete)

- Dependency-free Rust library with toolchain pinning, committed lockfile, and GitHub CI.
- Type-range wrapper for the 36 transaction IDs dispatched by the JavaScript implementation.
- Unit tests for type range and explicit unsupported values.

## Step 1: Characterize wire format (in progress)

- Completed: source-derived fixture generator for type 0 (activation), type 1 (issuance), type 2 (send), type 11 (managed grant), and type 12 (managed redemption).
- Completed: checked Rust ASCII envelope parser and preliminary typed field decoders.
- Completed: explicit regression cases for nested prefix, fractional-unit marker, and multi-whitelist discrepancies.
- Remaining: actual JavaScript `txEncoder` / `txDecoder` execution in isolation, accepted on-chain corpus, and cross-language differential outputs.

See [docs/WIRE_FORMAT.md](docs/WIRE_FORMAT.md).

## Step 1b: Propose corrected v1 wire encoding (this branch)

- A distinct binary `TLR` + version + numeric type-byte envelope, avoiding the historical base-36 selector and double-prefix bug.
- Exact atomic-unit type-2 send encoding and decoding, with address syntax and checked sizes.
- Deterministic source-independent v1 vectors in `fixtures/v1_vectors.tsv`.
- Legacy `wire` parser retained for offline diagnostics only, with no implicit fallback from v1.
- **Not activated on any network.** Consensus activation and chain-history migration must be settled before use.

See [docs/WIRE_V1.md](docs/WIRE_V1.md).

## Step 2: Exact quantities and ledger (prototype complete)

- Added dependency-free `Ledger` accounting primitives and typed account/property/tx identifiers.
- All amounts use checked `u128` atomic units. This is a **provisional internal representation**, pending reference-ledger and historical replay vectors.
- Implemented fixed/managed issue, authorized mint, issuer-only burn, and transfers over available balances.
- Added per-property supply conservation, duplicate transaction rejection, and all-or-nothing batches (clone-and-swap for now).
- Added deterministic property tests, overflow/rollback adversarial cases, and a draft binary send integration fixture.
- Remaining: reserved/margin/vesting/channel accounting, persistent transactional storage, cryptographic state commitments, reorg undo journals, state snapshots, and independent accepted-chain corpus.

See [docs/LEDGER.md](docs/LEDGER.md).

## Step 3: Limited state transitions

Port activation, fixed/managed issuance, and ordinary sends after the wire format is settled. Compare Rust results with confirmed JavaScript and chain-history fixtures. Isolate native tokens, vesting, policy restrictions, and procedural issuance until specified.

## Step 4: Integration

Add persistence, block ordering, reorg handling, and RPC as separate layers. Continue through channels, derivatives, clearing, and settlement using adversarial testing and review.

## Before any production deployment

A written protocol specification and independent consensus reference vectors must determine behavior. Require fuzzing, chain replay, multiple-implementation comparison, build reproducibility, review of dependencies and build scripts, and key-isolation testing.
