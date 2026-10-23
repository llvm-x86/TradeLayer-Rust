# Lean proof contract: TradeLayer-Rust

This is an **abstract kernel-checked protocol safety model**. The Lean definitions mirror a deliberately narrow projection of the existing Rust code. It is **not** a verified compiler, a binary proof, an authenticated chain adapter, a Cairo/STWO proof, a formal audit of SHA-256, or a complete Rust refinement.

## Traceability to existing code

| Abstract theorem / constructor | Rust implementation / checks |
| --- | --- |
| `AccountingStep.send` | `src/ledger.rs` `Operation::Transfer`; `tests/ledger_invariants.rs` |
| `commitA`, `commitB`, `withdrawA`, `withdrawB` | `src/ledger.rs`; `src/ledger_channels_impl.rs`; `tests/channel_escrow.rs` |
| `settleA`, `settleB` | `src/ledger_settlement_impl.rs` `calculate_settlement_preview` and settlement debit; `tests/channel_settlement.rs` |
| `managedMint`, `managedBurn` | `src/ledger.rs`; `tests/ledger_invariants.rs` |
| `accepted_tx_cannot_be_replayed` | `src/ledger.rs` `Ledger::apply` and per-network replay markers; `tests/ledger_invariants.rs` |
| `marking_preserves_other_chain`, `same_property_id_on_distinct_mainnets_is_distinct` | `src/network_profile.rs`, `src/network_bound.rs`; `tests/network_profiles.rs` |

The model reduces one property to two account balances and one bilateral channel's two escrow columns. Its `Nat` amounts carry explicit `u128` addition limits; subtraction requires sufficient funds. It proves conservation and boundedness over arbitrarily long **admitted** traces, while managed mint/burn change supply and circulating amounts by the same amount. The journal claim and commitment-root semantics, transaction signatures, checksums, channel ownership, future block heights, database snapshots, rollback, and property-map iteration remain outside this model.

The `Engine.tryCommit` command additionally couples a checked accounting-step witness to a fresh replay marker. Its theorems prove that a successful local commit preserves conservation and bounds in **all three networks**, marks its txid, and leaves the other two ledgers unchanged. This logical command is a candidate refinement target, not a function exported to Rust.

The replay proof treats a txid as `Nat`. The actual Rust type is `TxId([u8;32])`, and each TradeLayer instance must bind its replay namespace to verified genesis and chain history. Running the exact same local property ID on three networks is legal. Cross-chain asset transfers are outside the architecture.

### What the theorems establish

* `accounting_step_conserves` and `every_reachable_state_conserves`: arithmetic conservation over accepted transactions.
* `accounting_step_stays_bounded` and `every_reachable_state_bounded`: `u128` column bounds hold if the guards were established and the starting state was bounded.
* `accepted_tx_cannot_be_replayed`: once a txid is admitted, a second application in that network fails.
* `marking_preserves_other_chain`: marking Bitcoin cannot alter Litecoin or Dogecoin replay histories.
* `same_property_id_on_distinct_mainnets_is_distinct`: chain identity is part of the asset key.
* `committed_conserves_every_network`, `committed_bounds_every_network`, `committed_marks_same_chain_replay`, `committed_leaves_other_ledger_untouched`: one admitted atomic transition preserves global safety and changes only its own network.

### Run

Requires [Lean 4](https://lean-lang.org/) (pinned to `v4.35.0`).

```sh
cd formal
lake build
lake env leanchecker TradeLayerFormal
```

The `lean.yml` GitHub Actions job compiles the proof and requests an independent kernel replay with `leanchecker`, plus an axiom audit. It runs independently of the main Rust test suite.

## Boundaries for the next phases

1. **Rust-to-Lean refinement.** Build a pure checked-`u128` transition function with a Verus specification and relate it to the current `Ledger::apply` and `calculate_settlement_preview` implementations. Prove failure leaves the ledger unchanged, conservation through all map entries, journal single-use, and finite-`u128` arithmetic. Tests and matching numbers alone are evidence, not a proof of refinement.
2. **Authorized inputs.** Link `AuthenticatedProtocolOperation` to verified bytes, origins, activation rules, signatures, chain inclusion, and Block/Tx identity. Preserve separate Bitcoin, Litecoin, and Dogecoin identities; Bitcoin-only Raito/Cairo support is the existing capability.
3. **Settlement economics.** Prove claims correspond to real trades and actual lock/withdrawal semantics. The Type-31 SHA-256 preview root currently only binds local journal records and cannot authorize economic settlement by itself.
4. **Binary artifact identity.** Connect a reviewed compiler/runtime target and reproducible build to a specific verified executable hash; attach proof metadata, Lean version, trusted assumptions, and a CI attestation. A binary or compiler correctness claim requires a dedicated target-semantics/refinement proof.

This work is a **proof specification and independently checked abstract theorem set**, designed to become one component of a future proof-carrying protocol artifact.
