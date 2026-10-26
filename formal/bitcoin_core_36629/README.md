# Bitcoin Core PR #36629: Lean execution verification

Upstream: https://github.com/bitcoin/bitcoin/pull/36629
Pinned upstream commit: cf9b981fcc7ae52c7864beae35df5d60b5c322b4
Toolchain: Lean 4.34.1, dependency-free Lake package.

## Reproduction

From formal/bitcoin_core_36629, run:

    lake build --wfail
    python3 scripts/check_cpp_parity.py

The dedicated GitHub Actions workflow builds every Lean target, rejects
proof placeholders (sorry, admit, axiom), and compiles the pinned upstream
C++ encoding header against a minimal byte stream shim. The source file's
Git blob is checked before a 4,123-case C++/Lean parity comparison.

This is a manually extracted functional C++ model. A passing Lean build
does not establish full bitcoind binary correctness or a C++-to-Lean
refinement theorem. Source-to-model fidelity is still an audit obligation.

## Checked properties and source mapping

### IndexModel.lean
Source: src/index/txoscriptindex.cpp.

- Exact full-script-hash filtering after truncated-prefix lookup is sound
  and complete for modeled available-block transaction bodies.
- Flattened (prefix, height, transaction offset) rows correspond to outputs.
- In-range retrieval is complete conditional on a synchronized active index
  and available transaction bodies.
- Fresh-height append/erase restores previous index membership.
- A prefix collision can cause a spurious pruned-block candidate.
- Accounted work can be smaller than total physical reads.

### OrderedEncoding.lean
Source: src/index/ordered_varint.h and src/test/ordered_varint_tests.cpp.

- Universal encode/decode roundtrip for every value from 0 through
  34,630,287,487 inclusive, covering all uint32 values.
- Concrete byte encodings at every width cutover, 4,000,000,000,
  uint32 maximum and the five-byte maximum.
- Adjacent width cutovers have the required lexicographic order.
- Bad prefix and truncated examples are rejected.

### FixedDigits.lean and UniversalKeyOrder.lean
Source: src/index/ordered_varint.h.

- Every fixed-width base-256 digit representation from two through five
  bytes is strictly ordered by its underlying natural-number value.
- Each width class of the exact OrderedVarInt reference encoder preserves
  strict numeric order.
- The kernel-checked strict_order theorem proves strict lexicographic
  byte ordering for ALL x < y with y <= 34,630,287,487, including arbitrary
  cross-width comparisons. This closes the pure encoding-order obligation.

### DBKeyOrder.lean
Source: src/index/txoscriptindex_key.h.

- rowKey models the serialized 's' discriminator byte, five-byte big-endian
  salted hash prefix and OrderedVarInt block-height suffix.
- same_prefix_height_order and seek_boundary_order prove increasing block
  heights yield strictly increasing complete LevelDB keys.
- different_prefix_order proves all keys with a smaller five-byte prefix
  sort before all keys with a larger prefix, irrespective of height.
- The theorem assumes the standard source BigEndianFormatter<5> layout and
  unsigned-byte lexicographic LevelDB key comparison. Correspondence to
  the compiled C++ DBKey methods is a remaining obligation.

### Pinned C++ source execution oracle
Source: upstream/ordered_varint.h, exact Git blob
fe7cf1f5239aea3bf98f301c12953a0c8f402a8e.

- Compile and run the unchanged upstream OrderedVarIntFormatter against
  cpp_stubs/serialize.h, a deliberately narrow stream IO shim.
- scripts/check_cpp_parity.py verifies that pinned Git blob, compiles C++
  with -Wall -Wextra -Werror and compares 4,123 deterministic output rows
  byte-for-byte against the Lean executable reference.
- The C++ oracle independently checks truncated data, 0xf8..0xff
  prefixes, oversize encoding and uint32 decoding overflow.
- These samples support source-model fidelity. They do not constitute
  a mathematical proof of C++/Lean equivalence for every input.

### OffsetCodec.lean
Source: src/index/txoscriptindex_key.h, DBValue.

- Delta offsets roundtrip; successful decoding produces positive,
  strictly increasing offsets within the block bound.
- Boolean well-formedness agrees with the logical predicate.
- Checked integer-delta decode and complete value serialize/deserialize
  roundtrip under the block and maximum-count assumptions.
- Reject zero deltas, block overflows and excessive count.

Actual Bitcoin Core VARINT byte codec needs a source refinement proof.

### BlockLayout.lean
Source: src/index/txoscriptindex.cpp, DB::WriteEntries.

- Serialized positions start at 80 bytes plus CompactSize(tx count).
- One offset per transaction; all positions strictly increase for
  positive transaction sizes within the enclosing block bound.
- This position recurrence roundtrips through the delta representation.
- Concrete mixed transaction-size layout and delta vectors.

Actual C++ ComputeTotalSize, overflow behavior and file positions remain
external boundary assumptions.

### HistoryExecution.lean
Source: src/index/txoscriptindex.cpp, Find.

- Executable row step charges serialized key before the range boundary,
  then the value, then each candidate transaction on available blocks.
- Prefix false positives consume budget but are eliminated by full-hash
  comparison. Pruned blocks emit potential hashes without exact matches.
- The first boundary key can itself exceed the work budget.
- Candidate body charges satisfy exact accounting.
- End-to-end scan_sound proves every transaction emitted by a completed
  multi-row scan has the requested full script hash.
- End-to-end scan_budget proves its remaining charged budget cannot grow.
- Explicit collision, pruned, range and over-budget execution vectors.

The model assumes rows supplied after LevelDB Seek are faithful, successfully
decoded DB records and that available transaction bodies were read correctly.

### SpenderExecution.lean
Source: src/rpc/blockchain.cpp steps 6-10 and
src/index/txospenderindex.cpp.

- Mempool conflicts mark spent outputs, even when spender details are
  excluded; omitted annotations stay absent.
- Newly resolved spender transactions contain the outpoint as an input.
- Resolution preserves the original spent flag.
- Unspendable and BIP30-excluded outputs are protected by source guards.
- Counted extra spenders are charged only once per (block_hash, txid).
- Confirmed spenders precede mempool spenders, with the source orderings
  by height and tx byte offset or by mempool txid.

Mempool ownership, the actual spender-index database, UTXO consistency,
and transaction signatures are not verified.

### MergeHistory.lean
Source: src/rpc/blockchain.cpp, confirmed by_position merging.

- Inserting another result at the same (height,tx_offset) preserves the
  first transaction body and appends matching output indexes.
- Existing other positions are unaffected.
- If both results refer to the same transaction and agree on the output
  count, valid output bounds are preserved after combining.
- An executable counterexample shows that two different transactions
  at the same position can yield out-of-range output indexes after merging.

The counterexample exposes a necessary stable-position assumption across
Find calls and possible reorg interleavings; it is not proof of a
practically exploitable Bitcoin Core bug.

### MergeSnapshot.lean
Source: src/rpc/blockchain.cpp, step 5 multi-script aggregation.

- StableGroups states the required common immutable mapping from
  (height, transaction offset) to a transaction ID and output count.
- insert_stable and aggregate_sound prove all merged entries retain
  valid output indexes for ANY finite sequence of script-query hits
  satisfying that common-view precondition.
- opposing_snapshots_same_key constructs an explicit witness showing
  why two independently read chain snapshots may violate that invariant.
- Existence of an exploitable reorganization interleaving is not
  established; the stronger source-level snapshot/fork invariant is open.

### RPCGuard.lean
Source: src/rpc/blockchain.cpp, final chain-tip equality guard.

- A changed final tip is rejected, and the same final tip passes.
- Mempool revisions are not tracked by the tip comparison.
- A transient different tip followed by restoration is not detected.

These are specification boundaries for endpoint-only checks. No claim is
made that an atomic mempool snapshot is an existing RPC promise.

## Important unverified obligations

1. A complete C++-to-Lean refinement theorem for the formatter and DBKey
   serializer, beyond the pinned-header C++/Lean sampling check.
2. Actual Bitcoin Core VARINT and CompactSize wire bytes, 32-bit arithmetic
   bounds, C++ exception behavior, on-disk DBValue corruption paths.
3. Concrete C++ refinement for WriteEntries, Find, FindSpender and the
   complete twelve-stage getscripthistory RPC, including error paths.
4. Reorg races, LevelDB snapshots, salt persistence, crash recovery and
   the common chain/transaction identity view used across separate Find calls.
5. Descriptor expansion, actual per-output indexes, spentness/BIP30
   behavior, mempool timing, verbosity-2 undo/block reads and JSON.
6. Whole-node I/O bounds: the nominal 50 MiB limit excludes some
   additional full block and undo reads from charged work.
7. The upstream Bitcoin Core test suite remains to be executed at the
   exact pinned commit for independent behavioral parity.

This proof-only branch changes neither Bitcoin Core C++ nor TradeLayer
Rust runtime and does not change economic policy.
