# Bitcoin Core PR #36629: Lean execution verification

Upstream: https://github.com/bitcoin/bitcoin/pull/36629
Pinned upstream commit: cf9b981fcc7ae52c7864beae35df5d60b5c322b4
Toolchain: Lean 4.34.1, dependency-free Lake package.

## Reproduction

From formal/bitcoin_core_36629, run:

    lake build --wfail

The dedicated GitHub Actions workflow builds every Lean target and rejects
proof placeholders (sorry, admit, axiom).

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

The global pairwise lexicographic monotonicity proof remains open.

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

### RPCGuard.lean
Source: src/rpc/blockchain.cpp, final chain-tip equality guard.

- A changed final tip is rejected, and the same final tip passes.
- Mempool revisions are not tracked by the tip comparison.
- A transient different tip followed by restoration is not detected.

These are specification boundaries for endpoint-only checks. No claim is
made that an atomic mempool snapshot is an existing RPC promise.

## Important unverified obligations

1. Universal lexicographic ordering of exact OrderedVarInt keys,
   including the five-byte hash-prefix and ordered-height composition.
2. Concrete Bitcoin Core VARINT and CompactSize bytes, 32-bit integer
   bounds, on-disk DBValue decoding and corruption paths.
3. C++ source-to-model refinement for WriteEntries, Find, FindSpender,
   the full twelve-stage getscripthistory RPC, and actual serialization.
4. Reorg races, LevelDB consistency, salt persistence, crash recovery,
   pruning windows, and the active-chain snapshot used while scanning.
5. Descriptor expansion, actual per-output indexes, spentness/BIP30
   behavior, mempool timing, verbosity-2 block/undo reads and JSON.
6. Whole-node I/O constraints: the 50 MiB work limit does not cover
   every block and undo read made while building verbose RPC results.
7. Cross-language source oracle and Bitcoin Core regression-suite
   execution against the pinned PR commit.

This proof-only branch changes neither Bitcoin Core C++ nor TradeLayer
Rust runtime and does not change economic policy.
