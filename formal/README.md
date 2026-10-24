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

The `finite_extend_zero_preserves_validity` and `finite_move_to_new_recipient_preserves_validity` theorems handle a recipient absent from the formal support by adding a zero-valued slot before an authorized transfer, leaving totals and bounds unchanged. Rust regression `newly_seen_recipient_and_zero_pruning_change_only_column_support` checks that the actual BTreeMap omits the exhausted sender, creates the recipient, and maintains supply and snapshot correctness. Lean's `finite_drop_zero_head_preserves_validity` additionally proves that dropping an exhausted, unique zero-valued head entry preserves finite-support validity. General ordered-map erasure still needs an exact correspondence between the erased Rust key and its place in the abstract support, together with an arbitrary-key permutation argument.

## Stage 6: arbitrary-key canonical storage

A new production `canonical_columns::put_nonzero` helper consolidates the exact delete-on-zero, insert-on-positive behavior previously duplicated in wallet and channel escrow maps. Rust tests cover middle-key deletion, replacement, ordering, and a multi-property history where independent sends are replayed in different orders. Kani checks the same branch logic through a bounded two-key store using symbolic `u128` values; standard-library `BTreeMap` allocation and node mechanics remain trusted.

Lean's `finite_erase_zero_anywhere_preserves_validity` and `finite_insert_zero_anywhere_preserves_validity` extend the earlier head-only support proofs to **any prefix/suffix index**. The theorems preserve duplicate-free support, conservation, zero outside the support, individual `u128` bounds, and total-supply bounds. Insertion starts with a fresh zero-valued column. Erasure requires that the selected old column has reached zero. The key encoding and exact `BTreeMap` collection semantics are still separate implementation correspondence obligations.

Lean's `column_total_permutation` and `column_move_conserves_under_reordering` prove that sorting or otherwise permuting the finite support has no effect on conservation. A new Kani harness checks arbitrary selection of either of two symbolic columns through the production canonical-update control flow, preserving the unrelated column's exact Option value. This remains a bounded backing-store proof with actual `BTreeMap` behavior exercised by the Rust tests.

## Stage 7: concrete network-qualified column identity

The Rust read-only `SupplyColumnProofKey` encodes four exact fields: declared network tag (Bitcoin=1, Litecoin=2, Dogecoin=3), nonzero property ID, source tag (available=0, escrow-A=1, escrow-B=2, withdrawal=3), and original identifier bytes. Wallet/channel identifiers use the validated ASCII bytes already stored in `AccountId`; withdrawal requests use the exact 32-byte `TxId`. `SupplyColumnRow::proof_key_unverified` and `Ledger::supply_proof_keys_unverified` provide these keys for state audits. The network tag is explicitly **caller supplied**, so this does not authenticate transaction provenance or active chain state.

Lean's matching typed-source encoder (see `TypedColumnKey`) proves decoding the structured representation recovers the original network, property and exact identity bytes, implying no collisions between supported typed keys. The `typed_column_total_preserved_by_encoding` and `encoded_column_move_preserves_total` theorems establish total correspondence and conservation for arbitrary finite typed supports. Lean also proves exact conservation after relabeling finite typed support, including transfers between any two distinct typed keys. Rust regression tests verify all four source tags, byte-identical identifiers across source domains, repeated property IDs across three TradeLayers, and the exact nine heterogeneous wallet/channel/withdrawal columns of a genuine ledger before and after checkpoint restoration.

## Stage 9: production sorted-vector storage and ordered-map proofs

`Ledger.available` and the two `ChannelState.escrow_*` fields no longer use the standard-library `BTreeMap`: they are now `OrderedColumns<K>`, a small `Vec<(K, u128)>` kept in strictly increasing key order. Its `locate` uses `slice::binary_search_by`; its `insert` and `remove` replace or shift entries at the chosen index. All actual balance writes run through the same `canonical_columns::put_nonzero` zero-pruning policy. External checkpoint formats, reserved-withdrawal and journal maps, and trusted-input limitations are unchanged.

Lean's `sortedVecGet`, `sortedVecPut` and `SortedVecKeys` model ordered lookup and canonical writes with `Nat` keys and amounts. Kernel-checked `sorted_vec_put_reads_written_key` verifies exact selected-key reads after insertion/replacement/deletion for any finite sorted list. `sorted_vec_put_keeps_other_keys` proves framing for any other key. `sorted_vec_write_preserves_lower_bound` and `sorted_vec_write_preserves_order` prove strictly ascending keys remain sorted for arbitrary-length updates. These theorems are independent of the Rust allocator and machine-level binary-search implementation.

The Rust map has a 5,000-update independent `BTreeMap` comparison and a front/middle/end insertion/deletion unit test. All existing ledger, channel, snapshot, checkpoint and 1,200-operation independent Vec-ledger differential tests are also run against the replacement. **One Kani harness now invokes the actual `OrderedColumns<u8>` implementation**, building two fixed keys, symbolically inserting/removing the middle key, checking the untouched neighboring values and final support. Previous Kani proofs of scalar accounting, clone-and-commit and generic canonical write guards are retained. The actual-Vec Kani harness is intentionally limited to a small fixed key geometry with symbolic `u8` values; it does not verify all allocator paths, all key lengths or every possible `Vec` size.

**Remaining trusted refinement boundary:** Rust's `Ord` comparison on native property/account keys, the binary-search and Vec mutation semantics, and their exact relation to the Lean Nat-key ordering model need a machine-checked correspondence. The former BTreeMap-specific obligation is eliminated **only for wallet and escrow amounts**; registries, replay, withdrawals and settlement journals still use their original collections. None of these proofs establishes chain consensus or economic authorization.

## Stage 10: arbitrarily long storage-write refinement

Lean now adds an independent `linearColumnGet` that scans every candidate without sorting-based early exit. `sorted_vec_get_equals_linear` proves the sorted-list lookup has the same result as a complete linear scan for **every key in any strictly sorted finite list**. `SortedColumnRepresents` defines the explicit correspondence between the ordered list and its abstract `Nat → Nat` balance function.

`sorted_vec_write_refines_function` proves that one canonical insert, replacement, or zero-delete preserves the invariant and exactly implements functional assignment. `sorted_vec_write_trace_refines` extends that correspondence to **any finite sequence of operations**, regardless of update order, intermediate support size or key positions. `sorted_vec_empty_history_refines` establishes the empty-wallet starting case. Together with the Stage 9 framing, ordering and two-column-move theorems, this closes the sequential *Lean-model* refinement boundary for the sorted balance representation. It does **not** prove Rust's `Vec::insert/remove` or `binary_search_by` implement those Lean definitions for arbitrary native account and property keys. The actual-Vec Kani harness retains its bounded key geometry; the 5,000/3,000-operation Rust comparisons and full ledger regression suites supply independent executable evidence.

The repository's new `AGENTS.md`, `CLAUDE.md` and `scripts/ci-preflight.sh` are merged from `main` with this stage. The preflight policy asks that Rust source changes be checked locally before pushing. This stage changes only Lean/documentation and those files from `main`, because the working container has no Rust/Lean toolchain or direct GitHub network access. GitHub continues to run pinned Rust CI, Lean and Kani checks; the local-preflight limitation is explicitly recorded rather than silently claimed satisfied.

## Stage 11: canonical-state uniqueness and arbitrary-pivot binary-search semantics

The **production Rust storage implementation has not changed since Stage 9**. This stage strengthens the formal specification of `OrderedColumns` by verifying two additional properties beyond Stage 10's arbitrary-history pointwise assignment theorem.

**1. Canonical representation uniqueness.** Lean's `PositiveColumnValues` defines the absence of stored zero amounts. `sorted_vec_write_preserves_positive` and `canonical_sorted_columns_write_history` prove this invariant survives any number of zero-deletes, insertions, and replacements, when the starting state is canonical. `canonical_sorted_columns_extensional_unique` proves that two finite lists with strict ascending keys, only positive values, and identical results on **every lookup key** must be **the exact same list**. This is an extensional-to-structural uniqueness theorem for the mathematical balance-store representation, rather than just agreement of selected fixtures. It helps isolate the canonical checkpoint representation obligation.

**2. Soundness of arbitrary binary-search pivots.** The standalone `linear_get_pivot_split` theorem proves that when all left-side keys are below a pivot and all right-side keys are above it, searching the correct side yields the result of a full linear scan. `BinaryColumnTree` and `BinaryColumnOrdered` model arbitrary finite recursive decision trees with these partition conditions. `binary_column_read_matches_full_scan`, `binary_column_inorder_is_sorted`, and `binary_column_read_refines_sorted_vec` prove that all such search paths agree with lookup in the represented sorted list. `sorted_vec_any_pivot_has_valid_bounds`, `sorted_vec_any_pivot_subtrees_sorted`, and `sorted_vec_any_pivot_search_sound` establish that **any interior pivot** of a strictly sorted list satisfies the needed comparison partition, including a midpoint selected by a binary-search implementation.

**Proof limits and next concrete target.** These Lean theorems do **not** prove a Rust `std::slice::binary_search_by` implementation, Rust's `Ord` comparator for `(PropertyId, AccountId)` or `PropertyId`, the safety of actual `Vec` index/insert/remove/allocator operations, or their equivalence to Lean's `Nat` keys and sorted-list functions at arbitrary sizes. The already passing Kani harness for actual `OrderedColumns<u8>` covers a bounded fixed-key configuration, and the Rust differential test suite covers deterministic examples. To close the last representation boundary, verify or replace the concrete comparator and binary-search/index mutations; establish the key-encoding relation; then tie the concrete sorted Vec's semantics to these abstract list theorems. Other ledger state maps, transaction authorization and economic validity remain separate obligations.

## Stage 12: bounded native composite-key order correspondence

The Lean model now proves a **restricted concrete-key embedding** for a `(PropertyId, one-byte account ID)` pair. `oneByteColumnCode p a = 256*p + a`, with `a < 256`, has a proved inverse at the equality level and reproduces lexicographic pair ordering exactly. Theorems `one_byte_column_code_injective`, `one_byte_column_code_order`, `one_byte_column_code_equal`, and `one_byte_column_compare_refines_nat` establish the three-way comparison correspondence used by a sorted map. This isolates the key-comparator portion of the Rust-to-Lean refinement without relying on a collision-prone digest.

**Important scope:** real `AccountId` accepts 1–90 printable ASCII bytes, so this one-byte model does **not** prove the comparator for the full Rust account domain, nor binary-search indices, Vec insertion/removal semantics, allocator behavior or the full ledger. The next proof should use a length-delimited lexicographic byte representation for the entire allowed account domain and relate the native `Ord` implementation to it. Rust production source is unchanged; the repository's local Rust preflight has not been claimed run.


## Stage 12: concrete midpoint binary search and precise indices

The earlier arbitrary-pivot tree theorems establish that **any valid pivot** produces the correct search result. Stage 12 now constructs a **specific halving algorithm**, `midpointColumnTree`, which selects `rows.length / 2` at each recursive step. The fuel starts at the number of rows, and no fixed maximum vector length is assumed in Lean.

- `midpoint_split_certificate` proves every nonempty split covers the original list, and both recursive sublists are shorter. `midpoint_empty_suffix_implies_empty` excludes a missing pivot in nonempty input.
- `midpoint_tree_preserves_entries` proves recursive splitting and inorder traversal recover the **exact original list**, not merely the same multiset or sum.
- `midpoint_tree_is_ordered` proves every recursively selected midpoint satisfies the binary-search left/pivot/right ordering invariant.
- `midpoint_column_read_equals_sorted_vec` and `midpoint_column_read_equals_linear` establish exact lookup agreement with the sorted-map specification and an independent full linear scan for **all queries on any finite strictly ordered list**.
- `columnCountBefore` is an independent full-scan specification of the **lower-bound insertion index**: the number of existing keys strictly below the query. `column_count_before_pivot` proves that the index computation obeys the same branching as the midpoint search. `binary_column_lower_bound_matches_count`, `midpoint_column_lower_bound_is_exact`, and `midpoint_lower_bound_within_length` show the returned index is exact and always within the vector's length.
- `columnAt` is a total, explicitly defined zero-based list-index accessor (avoiding assumptions about Lean's library list-index API). `sorted_vec_hit_at_lower_bound` proves that a successful nonzero lookup identifies the **exact record at that insertion index**. `midpoint_find_success_points_to_exact_record` and `midpoint_find_missing_returns_zero` connect this to a `Option Nat` successful/absent-key search interface.

This is a stronger *algorithm-level Lean specification* than Stage 11's arbitrary pivot model: the recursive midpoint choice, index counting, hit position, complete coverage, termination fuel and sortedness preconditions are now all machine-checked. The successful-hit interface interprets a stored zero as missing; that agrees with the production **canonical nonzero balance** requirement, but not with raw malformed snapshot inputs that may contain zero-valued records.

**Trusted implementation boundary remains:** the proof is of the Lean definitions, not a universal proof of Rust `std::slice::binary_search_by`, `Vec::insert/remove`, indexing/allocator behavior, the `usize` representation, or `Ord` for native `PropertyId` and `AccountId`. The actual production sorted-Vec implementation is separately checked by a bounded Kani harness, 5,000/3,000-operation storage-oracle comparisons, and the full ledger differential tests. This stage does not change Rust source. A future concrete refinement proof must relate Rust's comparator, each binary-search iteration and resulting `Result<usize,usize>` index to `midpointColumnLowerBound` and `midpointColumnFind` for the native key domain. A source-to-binary compiler/refinement proof is still independent.

## Stage 13: complete native wallet key ordering and collision-freedom

The previous Stage 12 `oneByteColumnCode` was restricted to one-byte account identifiers. Stage 13 lifts that restriction to the **complete Rust `AccountId` parser domain**: 1–90 bytes, each printable ASCII in 0x21–0x7e. It does so through an explicit mathematical order embedding into `Nat`; there is **no change to the Rust runtime key representation or checkpoint/wire formats**.

**Encoding.** `byteRank width bytes` uses base 257 with every present byte incremented by one, and zeroes after the end of a string. This ensures a proper prefix sorts before its extension without ambiguity. `byte_rank_bounded` proves the suffix rank remains below `257 ^ width` for arbitrary byte sequences of length at most `width`. `byte_rank_digit_dominates` and `byte_rank_digits_compare` establish that the first differing byte dominates every possible suffix. `ByteLexLess` defines direct lexicographic list comparison, and `byte_rank_lt_iff_lex` establishes exact equivalence with rank ordering. `byte_lex_trichotomy` and `byte_rank_injective_on_valid` show different bounded strings never share a rank.

**Composite key.** `compositeByteCode width property bytes` sets aside another base-257 digit for the property ID. `composite_byte_code_order` and `composite_byte_code_injective` prove that encoded keys preserve and reflect the exact lexicographic ordering of `(property, accountBytes)`, and distinct pairs cannot collide. `ValidAccountBytes` constrains the full accepted account domain, with `account_property_key_order_full` and `account_property_key_injective_full` instantiating both results at the real maximum of **90 bytes**. Account identifiers that share prefixes, differ only in later bytes, or lie at the maximum length are covered by the universal Lean theorems, not merely enumerated as samples.

**Connect to the sorted balance abstraction.** `NativeAccountKey` retains separate typed fields for property and account bytes, `NativeAccountLess` models the source's lexicographic order and `encodeNativeAccount` maps it to the verified numeric rank. `native_account_less_iff_rank` and `encode_native_account_injective_on_valid` prove order preservation and collision-freedom. `native_account_sorted_embeds_in_nat` proves any finite native-key wallet book with strictly ascending keys and valid identifiers remains strictly sorted after encoding. `native_encoded_linear_get_matches_original` and `native_encoded_sorted_lookup_refines` show that the encoded sorted-list lookup has exactly the same result as a completely independent native typed-key linear scan, for **every query and every finite sorted valid wallet map**.

These results bridge the *mathematical key representation* from Rust's full account domain to the existing Lean `Nat`-indexed map theorems. **Trust boundary:** no machine-level Rust `Ord`/UTF-8/`String` or `PropertyId` implementation proof was produced, nor a Rust `binary_search_by`/indexing/`Vec` allocator refinement or full `Ledger::execute` refinement. The rank can exceed `u128`; it is a ghost/specification `Nat` representation, **not** a new executable key type. Rust production code is unchanged in this stage. The actual-Vec Kani harness and existing Rust CI remain separate validation.

## Stage 14: exact native wallet-key insertion-index correspondence

Lean now closes the *mathematical* lower-bound index bridge for the entire valid 1–90-byte ASCII account domain. nativeAccountCountBefore is a fresh linear scan over native (PropertyId, AccountId bytes) keys, independent of numeric rank and binary-search code. native_account_lower_bound_count_refines proves that counting native keys strictly smaller than a query yields exactly the same number as counting encoded base-257 keys, for arbitrary finite lists of validated account identifiers. Combining that with Stage 12's midpoint-column lower-bound proof, native_midpoint_lower_bound_matches_native_scan proves the exact **insertion index** in the ranked map is the independent native-key lower-bound count. native_midpoint_lookup_and_index_refine proves both index and read-value correspondence for every query, and native_midpoint_lower_bound_is_within_support proves index safety in the mathematical model.

To test the remaining *Rust implementation* trust boundary instead of merely adding more Lean definitions, CI now runs scripts/audit_native_wallet_order.py. It independently produces Python-sorted (u32 property, raw ASCII bytes) vectors, prefix and 90-byte boundary cases, matching/nonmatching queries, and lower-bound indices. The script generates a **temporary** Rust integration test and executes it with the pinned Rust 1.90 toolchain. This test uses the actual public PropertyId and AccountId types and compares native tuple Ord, an independent `std::slice::binary_search_by` probe and a separately implemented half-open cursor with Python's byte ordering. The production `OrderedColumns::locate` cursor has a distinct full linear-scan Rust unit oracle. It checks pairwise Rust comparisons against Python tuple ordering, insertion indices (including Err(index) on absence), independent partition_point results, and parser rejection of invalid account strings. The temp .rs file is removed after execution; the repository's production Rust code is unchanged. The full local preflight script will also run this oracle when the toolchain is available.

**Scope:** The test is deterministic and differential, not a universal Kani/Lean proof of the Rust standard library or compiler. The Lean rank is a ghost Nat value, not an executable u128 key. Native Rust String ordering semantics, Vec allocator behavior, insertion/removal and a complete state transition refinement remain assumptions requiring separate machine-level evidence. GitHub CI is authoritative for test execution in this connected environment, where local Rust/Lean tools are not available.

## Stage 15: proved lower-bound positional updates for arbitrary finite maps

`indexedColumnPut` is a separate, explicitly index-recursive finite-list specification of what `Vec::insert`, replacement and `Vec::remove` do at a chosen zero-based position. At index zero it removes or replaces the existing key, inserts a missing key, or leaves the vector unchanged on deletion of an absent key. Positive indices traverse and preserve the preceding entries before applying the same operation.

`lowerBoundIndexedPut` supplies that algorithm with `columnCountBefore`, an independent full-scan lower-bound index. Lean theorem `indexed_column_put_equals_sorted_put` proves that **for every finite sorted list, queried key and amount**, inserting/replacing/zero-deleting at this exact index produces the identical entire list as `sortedVecPut`. It handles keys at the front, interior and end, including absent keys and existing keys.

`midpointIndexedPut` then replaces the full-scan index with the specific midpoint binary-search index proved exact in Stages 12–14. `midpoint_indexed_put_equals_sorted_put` establishes **structural equality of the complete resulting list**, not merely equality of selected lookup values. Consequently `midpoint_indexed_put_preserves_order`, `midpoint_indexed_put_reads_written_key` and `midpoint_indexed_put_keeps_other_keys` prove ordered support, exact selected-key value and unchanged unrelated keys.

`midpointIndexedTrace` is a finite sequence of computed-index writes. `midpoint_indexed_trace_equals_canonical` proves that **arbitrarily long histories** are structurally equal to the existing canonical `sortedVecWriteTrace`, using each intermediate state's recomputed insertion index. `midpoint_indexed_trace_refines_function` connects those histories to the pointwise functional assignment model. Combined with Stage 13's full account byte-order embedding and Stage 14's native-key index proof, this is a stronger *end-to-end mathematical abstraction* for wallet balance storage.

**Trust boundary:** These proofs are over Lean `List (Nat × Nat)`, with idealized positional operations and mathematical `Nat` indexes. They do not verify the actual Rust compiler's `Vec::insert/remove`, bounds checking, memory movement/allocation, `usize` overflow or the Rust `binary_search_by` implementation for all key lengths. Stage 14's Python/Rust native comparator oracle and Stage 9's bounded actual-Vec Kani harness are separate executable evidence.

**CI context:** The main PR #23 is blocked by new clearing-math source conflicts with `main`. Stage 15 was therefore validated in a temporary **Lean-only diff** PR #34, targeting the last green Stage 14 proof commit. At commit `9b928d42d0a8e51ed314c9ad099e59c2c3f3c270`, that validation PR passed [Lean CI](https://github.com/patrickdugan/TradeLayer-Rust/actions/runs/39983859208) (1,042 declarations audited) and [Rust CI](https://github.com/patrickdugan/TradeLayer-Rust/actions/runs/39983859484) (223 tests). The existing ten Kani harnesses were **not retriggered** because Stage 15 changed no Rust/Kani files. The repository's required local Rust preflight could not run in this tool environment; no Rust source merge or change was performed here.

## Boundaries for the next phases

1. **Rust-to-Lean refinement.** Connect the new sorted-Vec implementation (native `Ord`, slice binary search, Vec insert/remove) to the Lean sorted-list semantics for arbitrary lengths, then relate all `Ledger::execute` operations and supply sums to the abstract state. Also verify the remaining Rust BTreeMap/BTreeSet registries, claims, pending withdrawals and replay sets. The current bounded Kani harness and differential tests are evidence for, not universal proof of, that correspondence.
2. **Authorized inputs.** Link `AuthenticatedProtocolOperation` to verified bytes, origins, activation rules, signatures, chain inclusion, and Block/Tx identity. Preserve separate Bitcoin, Litecoin, and Dogecoin identities; Bitcoin-only Raito/Cairo support is the existing capability.
3. **Settlement economics.** Prove claims correspond to real trades and actual lock/withdrawal semantics. The Type-31 SHA-256 preview root currently only binds local journal records and cannot authorize economic settlement by itself.
4. **Binary artifact identity.** Connect a reviewed compiler/runtime target and reproducible build to a specific verified executable hash; attach proof metadata, Lean version, trusted assumptions, and a CI attestation. A binary or compiler correctness claim requires a dedicated target-semantics/refinement proof.

This work is a **proof specification and independently checked abstract theorem set**, designed to become one component of a future proof-carrying protocol artifact.


## Stage 18: conditional native-key ordered-Vec implementation contract

Stage 17 added the native-key Rust differential oracle and two more bounded
actual-Vec Kani harnesses, establishing **12 passing harnesses** at the
validated Stage 17 revision. Stage 16 established mathematical equivalence
between full-length native account-key updates and the sorted Nat map.

Stage 18 makes the remaining *executable* assumptions explicit in Lean:

| Contract / theorem | Exact role | What remains externally trusted |
| --- | --- | --- |
| `NativeRowsValid` | All account strings satisfy the 1–90-byte ASCII parser domain | Actual parser/String behavior and object construction |
| `OrderedVecSearchContract.lower_bound_exact` | Rust search returns the count of strictly smaller native keys on every valid finite sorted support | Ord, UTF-8 byte ordering, Rust binary_search_by and usize interpretation |
| `OrderedVecMutationContract.indexed_write_exact` | Given a bounded index, an indexed write produces precisely the full positional insert/replace/zero-delete list | Rust Vec::insert/remove, element movement, memory, failure and compiler assumptions |
| `OrderedVecInvariantContract` | Each admitted step preserves sorted support and valid keys | Correspondence of successful executable intermediate states to the list model |
| `contracted_ordered_vec_write_refines` | Given search and mutation contracts, the entire encoded result equals sortedVecPut | Discharging both executable contracts |
| `contracted_ordered_vec_trace_refines` | Assuming invariant preservation, any finite successful history equals the canonical sorted-map trace | Preservation of intermediate Rust states and the panic/OOM boundary |
| `contracted_ordered_vec_trace_refines_function` | Final encoded state implements the pure functional balance-update trace | The three contract obligations and initial representation |

These theorems are Lean kernel-checkable *implications* with explicit
premises. Actual Rust production code has not been formally translated
into Lean. The contracts make no allocator-totality, panic-free-execution,
compiler-correctness, process-abort atomicity or authenticated-input claim.

**Successful writes and failed allocation:** In the Rust prototype,
`stage_validate_commit` provides a boundary for ordinary returned Err
values after its candidate clone. The Stage 18 proof models successful
storage operations. A panic during cloned-state construction or mutation
requires a separate unwind-state test; process abort/OOM is outside the
rollback theorem. Persistence across a crash is an independent concern.

**Executable checks:** Stage 17's `src/ordered_columns.rs` tests and
`scripts/audit_native_wallet_order.py` provide finite differential
evidence. A separate Stage 18 Rust test targets all 90 prefix-related
account lengths within one property and checkpoint roundtrips.

Further work could formally verify a narrower, self-implemented lower-bound
routine under a specified slice abstraction or integrate an independently
validated Rust verification framework. The frozen zero-third-party-crate
consensus policy remains in force.

## Stage 19: auditable source-level half-open search and pivot-index theorem

The production `OrderedColumns::locate` now uses an explicit half-open
`[left, right)` search loop with midpoint `left + (right-left)/2`, followed
by equality testing at the returned lower-bound position. This replaces
the earlier `std::slice::binary_search_by` call in this one wallet/escrow
backend. Exact native `Result<usize,usize>` results are compared with an
independent complete linear scan in Rust for 1–90-byte account prefixes
and full property-ID boundaries. The Python-generated conformance oracle
also checks native Ord and both independent search formulations against
Python-generated lower bounds. It uses frozen Cargo resolution.

Lean `half_open_predicate_search_exact` proves that the half-open loop
reaches its target for any finite interval under a pivot decision contract,
including strictly shrinking intervals. `sorted_column_pivot_iff_before_count`
and `encoded_pivot_below_iff_lower_count` prove that an indexed pivot
comparison in a strictly sorted mathematical list is equivalent to its
position preceding the independent full-scan lower bound.
`native_encoded_half_open_search_exact` composes this with the complete
native-key rank model. These are kernel-checked mathematical theorems.

The remaining executable trust boundary is **actual** Rust `Vec` indexed
reads, native `String` / `Ord` behavior and `usize` arithmetic matching
the mathematical pivot operation, as well as `Vec::insert/remove` shifting,
allocation and compiler/runtime fidelity. Rust differential tests and
bounded Kani harnesses provide finite executable evidence, not an
unconditional Rust source/binary proof or authenticated chain validity.
