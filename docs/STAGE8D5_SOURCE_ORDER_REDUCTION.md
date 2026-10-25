# Stage 8D5: TXID duplicate lookup and one-allocation Merkle, experimental

**Baseline:** `main@c7a37eb90b1ea335051bfc3cb7712116b372cfb7`, after merged Stage 8D1b (#132). Stage 8D2 static worker shards (#133), dependency wave barriers (#134), Bitcoin Core RPC cost (#135), bounded workers (#136) and parent UTXO snapshot probes (#137) are separate research branches. This change is isolated to the existing read-only raw Bitcoin-block TXID/carrier preindex. No third-party Rust crates, unsafe code, network access, chain/UTXO consensus or TradeLayer economic authority.

## Observed inefficiencies

The already-merged one-team CPU scan avoids repeated worker creation but reduces every `TransactionObservation` sequentially through `BTreeMap<[u8;32],usize>` for duplicate-TXID detection. It then builds a TXID `Vec` from the report, clones it again in the Merkle function and allocates another vector for every tree level. Those allocations and lookups happen after hash workers complete.

Stage 8D5 isolates the two changes to determine whether either matters to actual scanner throughput.

1. **Hash-index reducer:** `inspect_parallel_one_team_hash_seen` retains the existing worker team and reference Merkle. A preallocated `std::collections::HashMap` stores TXID -> first original block position. It uses the standard randomized hash builder (no custom adversarially-weak hasher), checking duplicates in exact block order only after workers join. The baseline tree-reducer implementation remains the default.
2. **In-place Merkle:** `inspect_parallel_one_team_inplace_merkle` retains the original tree-based duplicate reducer and collects observations once into a mutable leaf vector. Every parent overwrites an already-consumed child; there are no further per-level allocations. `merkle_root_unverified_inplace` is also exported as a read-only standalone parity helper. Existing `merkle_root_unverified` stays untouched as the reference. The unusual Bitcoin Merkle rule is preserved: an adjacent pair of identical *actual siblings* aborts with `InvalidTxidMerkleTree`, whereas an unmatched odd final node may be implicitly duplicated for the SHA256d calculation.
3. **Combined:** `inspect_parallel_one_team_fast_reduce` uses both experimental changes. All four one-team variants return the same `UnverifiedRawBlockIndex` and still reject any attempt to gain Bitcoin chain, script/UTXO or TradeLayer ledger authority.

All allocations are bounded by the existing maximum of 200,000 transactions and 32MiB of raw transaction bytes. The HashMap reserves space for at most the input transaction count. No per-transaction serialized buffers are copied; these changes only impact scanner-result metadata and Merkle scratch.

## Benchmark design

The existing release `tradelayer-block-ingest-bench` measures the full *prepared-only* one-team variants under identical input, workload, worker count and median timing:

- `one_team_prepared_only_median_ms` baseline tree duplicate index + allocating Merkle
- `one_team_hashed_prepared_only_median_ms` HashMap + original Merkle
- `one_team_inplace_merkle_prepared_only_median_ms` BTreeMap + in-place Merkle
- `one_team_fast_reduce_prepared_only_median_ms` both changes
- `fast_reduce_vs_one_team_ratio` (baseline / combined; above 1 means faster)

The four one-team variants are timed in forward order on even repetitions and reverse order on odd repetitions; CI executes four repetitions per workload to counterbalance variant position.

For attribution, the benchmark additionally times precomputed TXIDs through duplicate-only `BTreeMap` vs `HashMap` lookup and allocating vs in-place Merkle, using the same raw digest order and `7 * repeats` same-process sweeps per workload. The reported isolated ratios are informative for the reducer only; end-to-end one-team timing decides whether any proposed production change is useful. The release workflow checks that outputs exist but **never** imposes a CI speedup threshold.

Measure the existing 1,000-TX 0%-SegWit/1-worker, 4,000-TX 50%-SegWit/4-worker and 10,000-TX 95%-SegWit/8-worker synthetic workloads; then test real Bitcoin blocks and a stable host before promoting either variant. Do not extrapolate these timings to Bitcoin Core signature checking, chain state transitions, UTXO IO or network RPC.

## Adversarial semantic checks

`tests/stage8d5_reduction_merkle.rs` compares source-order scanner equality for 1/2/4/8/16 workers, multiple windows, 0/50/95% SegWit, the four-TX independent fixture and 10,000 TX stress. Merkle vectors include 1/2/3/odd/large leaf counts, a repeated *actual* sibling pair, a duplicated higher subtree, an allowed odd-level duplicated leaf, zero leaves and an above-bound count. A repeated transaction at original positions 1 and 3 must produce the identical first/second index error under all reducers. Invalid configs and permanently denied consensus gates remain active.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen --test stage8d5_reduction_merkle
cargo test --frozen
cargo build --all-targets --release --frozen
target/release/tradelayer-block-ingest-bench 1000 1 0 4
target/release/tradelayer-block-ingest-bench 4000 4 50 4
target/release/tradelayer-block-ingest-bench 10000 8 95 4
```

**Promotion gate:** only after exact-head frozen tests, lint, release and required strict `check` pass; require a consistent prepared-only end-to-end win on a representative same-host real-block corpus. Otherwise retain this as a measured negative result in draft, preserving the merged default and immutable scanner authority boundaries.
