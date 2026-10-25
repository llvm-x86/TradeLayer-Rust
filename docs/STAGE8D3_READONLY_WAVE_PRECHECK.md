# Stage 8D3: read-only, wave-barrier precheck scheduling experiment

Branch: `audit/stage8d-readonly-wave-probes`, based on
`main@c7a37eb90b1ea335051bfc3cb7712116b372cfb7`.
This increment is deliberately separate from [Stage 8D2's static
TXID work partitioning](https://github.com/patrickdugan/TradeLayer-Rust/pull/133).

The existing fused raw-block scanner computes strict transaction byte
boundaries and a **candidate** prevout dependency DAG. A DAG alone is
not enough to ensure a future script checker respects same-block
dependencies. The new file
[src/wave_precheck.rs](../src/wave_precheck.rs) experiments with
the **barrier semantics** of concurrent read-only probes, without giving
them Script or UTXO authority.

## Protocol-independent scheduling contract

- A caller supplies the same borrowed block and preindex that already
  passed read-only SHA256d/Merkle inspection. The module recomputes the
  dependency DAG rather than trusting a caller-supplied wave assignment.
- The existing planner rejects malformed coinbases, duplicate input
  outpoints, forward or self references, out-of-range same-block output
  indexes and same-block coinbase spends. External prevouts are explicitly
  **UNVERIFIED**, not authenticated UTXOs.
- A caller supplies a `Fn(index, raw_tx_bytes)` returning
  `NoLocalRejection` or `Reject`. Such a
  closure is only an experiment/hint. It cannot return a Bitcoin Script
  validation proof or authorization capability.
- All work in one dependency wave is finished or denied before any
  later dependency wave can start. One scoped worker team per wave uses
  at most 16 workers. Jobs access immutable borrowed transaction
  slices; the scheduler allocates no per-transaction global result
  slots, and all worker handles are joined.
- Within a wave, errors are reduced by the lowest **source position**,
  independent of thread completion order. Between waves, the earliest
  **barrier** takes precedence even if an unvisited later wave would have
  contained a lower numeric transaction index. A denied wave returns
  no success report and never starts a subsequent wave.
- The returned report is named
  `read_only_dependency_waves_unverified`.
  All calls attempting to promote it to authenticated parent UTXOs,
  valid Bitcoin scripts or an economic state transition return errors.

**Important caveat:** Rust's `Fn + Sync` bound and an immutable
raw-block reference do not forbid a malicious callback from capturing
interior-mutability primitives. The scheduler passes **no** mutable UTXO,
ledger or TradeLayer capability, but the caller must still guarantee its
probe is pure and side-effect-free before using this API. Real consensus
code should use a capability-constrained verifier interface rather than
trust arbitrary closure implementations.

## Tests and security cases

The dedicated test in
[tests/stage8d_wave_precheck.rs](../tests/stage8d_wave_precheck.rs)
covers:

- 1/2/3/4/8/16 worker configurations, 1–512 window policy, and
  exact borrowed-transaction pointers.
- A parent/child/grandchild chain requiring three sequential waves.
  Atomics in **test instrumentation only** confirm the next wave never
  starts before every predecessor-wave probe has finished.
- First failure cancellation and later-wave nonexecution; the lowest
  source index wins within one wave over 100 adversarial scheduling runs.
- A deliberately counterintuitive (wave, source) failure precedence
  when a higher-index independent transaction fails before a lower-index
  dependent transaction's wave executes.
- Panic-catching, concurrent deny-vs-panic priority, a distinct raw
  block preindex mismatch, invalid worker configurations and duplicate
  input spend refusal **before any callback executes**.
- Permanent refusal of Bitcoin Script/UTXO/chain/TradeLayer economic
  authorization even after every probe returns `NoLocalRejection`.

~~~sh
cargo test --frozen --test stage8d_wave_precheck
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen
cargo build --all-targets --release --frozen
~~~

The only CI result this stage can earn is that the scheduler and its
failure semantics conform to these fixtures. It does **not** increase
Bitcoin consensus throughput and makes no speedup claim. There is no
real Bitcoin Core RPC, script interpreter, authenticated UTXO snapshot,
full BIP141 witness commitment, active-chain selection or rollback
capability. The Bitcoin VPS remains untouched.

## Next genuine-consensus obligations

Before any read-only script precheck can feed a real Bitcoin consensus
engine, require a *trusted* previous UTXO set bound to a previous block
hash, accurately decoded prevout amounts/scripts/witnesses/locktimes,
transaction-specific signature-message rules, and independently checked
script commitments. The engine must wait for an authorized predecessor
state to create a child prevout; mere wave ordering does **not** create
or authenticate that state. Only after all validation may a separate
serial state applier atomically mutate the UTXO/TradeLayer ledger with
reorg-safe undo and strongest-fork replay.
