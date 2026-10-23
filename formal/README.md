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

## Production arithmetic refinement stage

The proofs apply to **successful two-column moves** and treat `u128` overflow as a rejected transition. The Rust entry point retains its original funds check and self-transfer no-op semantics. The entire ledger still commits through `apply_batch`, which stages and validates a clone before replacing the public state. The checker has not formally established the BTreeMap update semantics or this clone-and-commit behavior.

The production `src/checked_amounts.rs::move_atoms` function computes checked changes for two distinct columns. Actual production call sites now use it in direct transfers, channel commits, cross-channel transfers and the Type-31 King settlement debit. Same-account transfers remain a validated no-op in `Ledger::execute`.

`TradeLayerFormal.checkedMove` models its three exact rejection guards followed by its checked integer updates, and proves accepted moves preserve total units and u128 bounds. The separate Kani harnesses in that **production Rust module** use unconstrained symbolic `u128` inputs and prove guard correspondence and conservation when total supply fits `u128`.

CI pins `kani-verifier 0.68.0` and runs the real Rust proof harnesses. This strengthens the Rust implementation boundary for arithmetic while leaving map ownership, channel authorization, full ledger cloning/rollback and chain authentication outside the checker.

## Stage 3: atomic multi-transaction batches

The real `Ledger::apply_batch` now calls `src/atomic_batch.rs::stage_validate_commit`, which clones the complete state, evaluates its validator, and writes the candidate back only when all work succeeds. The validator still calls `execute` for each transaction and `verify_conservation` before publication. Its transaction insertion order and replay semantics are unchanged.

Two Kani harnesses symbolically check the *production generic commit function* instantiated with a scalar `MiniLedger`. They cover failures after staged mutations and duplicate replay markers, plus successful publication of multiple fields. This demonstrates the control-flow guarantee for that concrete instantiation. A general proof for arbitrary `Clone` implementations and the Rust `BTreeMap` storage is still open.

Additional Rust integration tests run the **actual ledger** and assert byte-for-byte state equality after a duplicate txid late in a batch, an overflow late in a managed mint sequence, and a panic in the supplied iterator after a valid staged operation (with unwinding enabled). Previous replay markers remain unchanged. External side effects of caller iterators are outside the ledger's rollback contract.

Lean's `CertifiedBatchTx` states the refinement premise each accepted individual operation must meet; `stagedBatch` iterates these operations and rejects duplicate txids before publication. Kernel theorems prove arbitrary finite accepted batch histories preserve conservation and bounds across the three separate TradeLayers, while a rejected batch restores every balance/replay field to the original live state. `valid_prefix_then_rejection_rolls_back` applies even when earlier operations modified the staged candidate. These are abstract proofs whose mapping to the actual Rust `Ledger` collections still requires a refinement argument.

The concrete `certifiedDirectSend` operation now derives its `AccountingStep` witness from `checkedMove` rather than taking it as a free external premise. A chain-origin/actor validator and the actual Rust map projection must still be connected for an end-to-end theorem. The `nonadjacent_duplicate_txid_rejects_batch` theorem covers repeats separated by an arbitrary list of operations, provided their network namespace and transaction identifier match. The proof uses monotonic replay-marker retention across successful intermediate steps.

## Stage 4: per-property maps and single-use journals

The mathematical state is now indexed by arbitrary property IDs through `PropertyLedgerView`. Lean establishes that applying one certified accounting step to a property preserves every other property's balances, alongside conservation and `u128` bounds across all property IDs. This extends the earlier single-property four-column projection to arbitrarily many properties. The number of balance columns within each property is still limited by the original abstract model.

`JournalView` captures pending and swept membership keyed by claim transaction identity. A single successful `journalMove` changes only the selected entry, moves it from pending to swept and preserves the disjoint-membership invariant. A second consumption and attempts at absent claims are rejected.

The production King settlement uses `journal_move::move_pending_once` on the actual `BTreeMap` journal collections. That function checks destination and source presence before mutation, so a duplicate or missing claim fails closed. Kani verifies the **same production `move_checked` control flow** with a three-key `TinyJournal` model and symbolic claim values. This avoids model-checking the internals of `BTreeMap`, whose node allocation and traversal generate excessive proof complexity. The production adapter uses `BTreeMap::contains_key`, `remove` and `insert`. Rust unit tests exercise these real ordered-map operations for successful movement, already-consumed keys and absent claims. Kani's result is conditional on the model faithfully capturing those collection operations.

These additional theorems verify the **abstract map and journal projection**. The full Rust-to-Lean refinement remains open, including account/column ownership, claims with trade references, receipt hashes, batch rollback and authentic economic inputs.

## Stage 5: finite-support accounting and auditable supply partitions

`FiniteSupplyView` models an arbitrary finite set of accounting columns by abstract numeric keys, covering available balances, channel escrow sides, and reserved withdrawal records. Lean proves conservation and u128 bounds after a checked transfer between any two distinct present columns, and conservation after managed mint/burn. It also proves all unrelated columns stay unchanged and that balances outside the finite support remain zero. The finite support list is required to be duplicate-free and complete. Abstract numeric keys need a verified injective encoding of real Rust composite map keys to establish a full refinement.

The live `Ledger::supply_audit_rows_unverified` is a read-only, property-ordered projection of supply and all three ledger accounting partitions, and returns only after `verify_conservation` succeeds. Its totals are recalculated from the live available, channel and pending maps. A separate regression constructs wallet/channel/reserved balances across two properties and checks the audit before and after withdrawal maturity, plus after canonical snapshot roundtrip. The production `checked_supply_add` now handles both existing conservation accumulation paths and the audit with explicit overflow rejection; Kani checks its u128 semantics.

This stage strengthens the connection between the finite-model proof obligations and the Rust ledger's real state, while leaving a machine-checked equivalence between `BTreeMap` iteration and the abstract support list as an open obligation.

The ledger also exports `supply_column_rows_unverified`, a sorted finite support of typed `SupplyColumnSource` keys and their positive atom counts. Its source tags distinguish available wallets, two channel-escrow sides and reserved withdrawal records; entries cannot alias across variants. The Rust test compares the exact nine canonical columns against an independent grouping of available, escrow and pending totals, including a snapshot reload. Lean's `taggedColumn` proof gives a collision-free numerical namespace across four source tags; injective encoding of the underlying Rust account/channel/transaction identifiers into Nat remains an explicit bridge requirement.

## Boundaries for the next phases

1. **Rust-to-Lean refinement.** Build a pure checked-`u128` transition function with a Verus specification and relate it to the current `Ledger::apply` and `calculate_settlement_preview` implementations. Prove failure leaves the ledger unchanged, conservation through all map entries, journal single-use, and finite-`u128` arithmetic. Tests and matching numbers alone are evidence, not a proof of refinement.
2. **Authorized inputs.** Link `AuthenticatedProtocolOperation` to verified bytes, origins, activation rules, signatures, chain inclusion, and Block/Tx identity. Preserve separate Bitcoin, Litecoin, and Dogecoin identities; Bitcoin-only Raito/Cairo support is the existing capability.
3. **Settlement economics.** Prove claims correspond to real trades and actual lock/withdrawal semantics. The Type-31 SHA-256 preview root currently only binds local journal records and cannot authorize economic settlement by itself.
4. **Binary artifact identity.** Connect a reviewed compiler/runtime target and reproducible build to a specific verified executable hash; attach proof metadata, Lean version, trusted assumptions, and a CI attestation. A binary or compiler correctness claim requires a dedicated target-semantics/refinement proof.

This work is a **proof specification and independently checked abstract theorem set**, designed to become one component of a future proof-carrying protocol artifact.
