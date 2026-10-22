# Deterministic in-memory ledger (prototype)

This implementation is a **pure accounting research component**, not a consensus engine or production financial software. The code is in `src/ledger.rs`; its integration tests are in `tests/ledger_invariants.rs`.

## Primitive model

- Property keys are nonzero `u32` IDs; account keys are opaque printable non-space ASCII strings of up to 90 bytes.
- Quantities use **`u128` atomic units**. For the draft wire-v1 direct send, the parser produces a `u64` atom count; conversion to `u128` is exact.
- The ledger currently models **available balances only**. Vesting, margin, reserved, channel, native, synthetic, procedural, compliance and administrative activation states need separate, verified specifications.
- A property carries a supply counter, a designated issuer identity, and either `Fixed` or `Managed` supply classification. The existing JS implementation has additional behavior that is not included here.
- `CreateProperty` issues an initial positive supply to the issuer. `Transfer` moves atoms without altering supply. For managed properties, `Mint` credits the recipient and increases supply while `Burn` debits the **issuer's own balance** and decreases supply. Each operation requires a positive amount.
- The caller supplies a 32-byte `TxId`. A later chain adapter must derive it from **verified** transaction bytes; accepting arbitrary client-provided transaction IDs would make replay protection meaningless.

## Invariants

For each property ID `p`, always require:

```text
supply_atoms[p] == sum(available_atoms[(p, account)] for every account)
```

All sums and increments use checked arithmetic. No negative balances, absent-property balances, or stored zero entries are permitted. Duplicate transaction IDs are rejected. Self-transfers require sufficient funds, consume a transaction ID, and leave all accounting unchanged.

`Ledger::apply_batch` stages an ordered batch on an independent clone. Any failure aborts the **entire batch**, including earlier created properties and replay markers. This is deliberately simple for now and scales as O(state size) per batch. It is **not** suitable for high-throughput replay until a transactional storage layer and deterministic undo journal are designed and audited.

`Ledger` derives `Clone` and `PartialEq`: tests exercise independent fork views from a checkpoint, while chain reorganization and durable checkpoints remain unimplemented.

## Scope of testing

Tests cover fixed and managed issuance, transfers, admin authorization, issuer-only redemption, overflow, invalid IDs, zero amounts, insufficient balances, duplicate IDs, atomic rollback, supply integrity, order-independent outcomes for commuting transfers, draft wire-v1 direct-send decoding, and 2,001 deterministic randomized transfer attempts. CI runs the Rust suite with `cargo test --locked`.

## Before binding this to a blockchain

The Rust accounting API currently **trusts the `issuer` and `from` arguments**. They are program inputs, not proof of signatures or ownership. A consensus adapter needs to establish network identity, txid provenance, block order, activation rules, the token registry, restricted transfers, exact legacy behavior, reorg rollback, and applicable fees before calling `apply`.

The legacy `tl` ASCII decoder is still read-only. The proposed `TLR` binary format remains unactivated on Bitcoin, Litecoin, and Dogecoin networks.
