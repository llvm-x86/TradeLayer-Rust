# TradeLayer JS → Rust: first milestones

Historical implementation: [patrickdugan/tradelayer.js](https://github.com/patrickdugan/tradelayer.js)

This is a **migration scaffold**, not a deployable consensus node. Preserve the JS repository and its test vectors during the port. Work on isolated, reviewable changes.

## Step 0: Bootstrap (this commit)

- A Rust library crate, with no external crate dependencies.
- Pinned Rust toolchain, checked-in lockfile, formatter, linter, and CI.
- A type-range wrapper for the 36 transaction identifiers currently dispatched by `src/types.js`.
- Tests verifying supported type numbers and explicit rejection of unknown types.

## Step 1: Wire-format characterization

Inspect `src/txEncoder.js`, `src/txDecoder.js`, `src/types.js`, and existing JS tests. Export canonical input/output fixtures and known edge cases. Clarify the `tl` marker, base-36 type selector, integer/decimal amount encoding, and differences between wrapper and per-type encoders.

Only then add a Rust parser/encoder. Distinguish exact legacy behavior from protocol rules requiring correction.

## Step 2: Deterministic amounts and ledger

Choose canonical integer units through verified vectors. Add checked arithmetic and typed account/property identifiers. Establish total-supply accounting, account-balance invariants, rollback behavior, and replay protection.

## Step 3: Minimal transaction execution

Port activation, fixed/managed token issuance, and ordinary type-2 transfers with adversarial tests. Compare Rust outputs with independently validated JS fixtures. Keep special native tokens, vesting, compliance restrictions, and procedural tokens isolated until their rules are fully specified.

## Step 4: Integration and expansion

Add persistence, transaction/block ordering, reorg support, and RPC as separate layers. Expand to channels, contracts, settlement, and remaining transaction types under property-based and differential testing.

## Before any production deployment

A protocol spec and independently reviewed consensus fixtures must determine behavior. Require security review, chain-replay tests, fuzzing, dependency audits, reproducible build checks, and key-isolation testing. Never infer financial correctness from compilation alone.
