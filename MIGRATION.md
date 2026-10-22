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

## Step 2: Exact quantities and ledger

Pin canonical integer scaling with independently validated vectors. Establish checked amounts, account/property identifiers, total-supply conservation, atomic rollback, replay protection, and reorg invariants. Investigate all differences from the existing JavaScript number semantics.

## Step 3: Limited state transitions

Port activation, fixed/managed issuance, and ordinary sends after the wire format is settled. Compare Rust results with confirmed JavaScript and chain-history fixtures. Isolate native tokens, vesting, policy restrictions, and procedural issuance until specified.

## Step 4: Integration

Add persistence, block ordering, reorg handling, and RPC as separate layers. Continue through channels, derivatives, clearing, and settlement using adversarial testing and review.

## Before any production deployment

A written protocol specification and independent consensus reference vectors must determine behavior. Require fuzzing, chain replay, multiple-implementation comparison, build reproducibility, review of dependencies and build scripts, and key-isolation testing.
