# Stage 8D4: parent-UTXO candidate witness coverage

This stage is an **offline preflight** between the fused Bitcoin raw-block
preindex and [Stage 8D3's unverified dependency-wave probe scheduler]
(STAGE8D3_READONLY_WAVE_PRECHECK.md).

The implementation lives in
[src/parent_utxo_snapshot.rs](../src/parent_utxo_snapshot.rs), with
[adversarial Rust tests](../tests/stage8d_parent_utxo_snapshot.rs).

The sole positive status is
`parent_utxo_witness_coverage_unverified`.

## What a candidate snapshot carries

A caller supplies an exact claimed parent block ID (raw Bitcoin wire
order), its claimed height, a sorted, bounded sequence of outpoint records
and a claimed SHA256d witness-table fingerprint. Each record contains:

- Exact prior transaction ID bytes and output index.
- Claimed amount in satoshis (within Bitcoin MoneyRange).
- Claimed scriptPubKey (at most 10,000 bytes).
- Claimed output creation height and coinbase origin.

The table must contain **exactly one row for every external prevout**
referenced by the target block, and no others. The helper recomputes each
raw transaction observation to ensure this block's preindex is not stale,
including a case where SegWit witness bytes change but the stripped TXIDs
and block Merkle root stay identical.

It first rejects local dependency-DAG faults (same-block double spends,
forward references and impossible outputs), then classifies all inputs
as either external to the block or produced by an earlier transaction in
that block. External inputs require exact table membership. Same-block
children are counted separately and **deferred** until a real verifier
has validated their producers and created the outputs; an external
snapshot cannot authorize them.

A claimed coinbase input is rejected if fewer than 100 blocks have passed
since its claimed creation height, but the source's coinbase flag and height
are still **untrusted claims**, not verified coinbase history.

The resulting counts must match the separately computed DAG's external
input count and same-block dependency edge count. Duplicates, unsorted
witnesses, oversized scripts, out-of-range values, future outputs,
unused entries and missing entries are all rejected.

## Deterministic candidate fingerprint

The canonical encoding begins with domain bytes
`TRDL-PARENT-UTXO-WITNESS-v1\0`, followed by:

1. Claimed parent block hash, raw 32 bytes.
2. Claimed parent height as a little-endian u32.
3. Number of table rows as a little-endian u32.
4. For each row, sorted lexicographically by raw txid then vout: raw
   32-byte txid, LE u32 vout, LE u64 amount, LE u32 creation height,
   1-byte coinbase flag, LE u32 script length, exact script bytes.

The candidate fingerprint is
`SHA256d(canonical_encoding)`.
The total encoded witness table must not exceed 8 MiB, with at most
100,000 external entries. The fingerprint is a **local consistency
checksum**. A liar can change an amount, script or provenance,
recompute the fingerprint and pass. Bitcoin has no universally
authenticated UTXO-set root in its ordinary block headers; the fingerprint
must never be described as a proof of chainstate.

The tests demonstrate a coupled forgery that passes the checksum after
changing an amount/script, while the successful result still refuses
every authenticated-state or economic admission request.

## Composition with Stage 8D3

The convenience wrapper
`inspect_waves_with_candidate_snapshot` performs the entire
snapshot audit first. If it fails, **no wave probe executes**. Only after
all external inputs are accounted for does it invoke Stage 8D3's
wave-barrier scheduler on the same borrowed raw block and preindex.

The wrapper still accepts an **arbitrary untrusted callback**. It
provides no Script engine, signature verifier or UTXO write handle.
In particular, it must not be used to accept or mutate a TradeLayer
ledger, authorize a payment, approve a channel revision or validate a
full Bitcoin block. An application could maliciously construct both
snapshot and commitment; that is explicitly out of scope.

## Acceptance and next steps

~~~sh
cargo test --frozen --test stage8d_parent_utxo_snapshot
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen
cargo build --all-targets --release --frozen
~~~

The standalone test suite checks external coverage, same-block deferral,
missing/extra/duplicate/unsorted records, amount/script/height/coinbase
tampering, mature and immature claimed coinbase inputs, parent-fork
mismatch, block-local double spend, callback cancellation, caller-coupled
fingerprint forgery and exact raw SegWit witness changes under an unchanged
TXID Merkle root.

Actual Bitcoin Script verification requires independently authenticated
and versioned parent chainstate, UTXO amounts/scripts/confirmation heights,
BIP141 witness commitment verification, sighash generation, signature
checks and median-time-past/locktime rules. After all per-transaction
checks, a separate **serial deterministic state applier** with atomic
undo logs and strongest-fork replay must be implemented. None of
these obligations is discharged by this stage.

No Bitcoin Core RPC, VPS worker, third-party Rust crate, wallet key
material or expensive proof generation is used.
