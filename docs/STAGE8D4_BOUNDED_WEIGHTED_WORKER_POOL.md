# Stage 8D4: bounded persistent weighted CPU worker pool

Baseline: merged `main@c7a37eb` after Stage8D1b (#132). Stage8D2 weighted static shards (#133), read-only dependency-wave scheduling (#134), and the local Bitcoin Core RPC benchmark (#135) are separate open experiments. This stage modifies **only untrusted CPU TXID/wTXID and TradeLayer carrier preindexing**.

## Objective and existing measured limits

Stage8D1b's one-team path avoids spawning 8 threads for every 512-TX window but allocates a `OnceLock` result slot for *every* TX (up to 200,000 slots). Stage8D2's static byte-work shards remove atomic claims and OnceLocks, but an initial GitHub runner release comparison on 10,000 synthetic 95%-SegWit TXs found one-team 24.369ms vs static 25.454ms. Static assignment can strand a worker on a long job.

The new opt-in `UnverifiedRawBlockView::inspect_parallel_pooled_weighted` combines persistent workers with strictly bounded per-window output and dynamic claim distribution.

## Implementation

- One `thread::scope` of at most 16 workers for the entire raw block. Each worker receives its next contiguous **claim**, selected dynamically as it becomes free. The CPU work estimate is the raw TX byte length plus stripped TX bytes for SegWit. Claims target roughly four times as many chunks as workers, with a minimum byte target and a maximum of 32 TXs per chunk. The producer never dispatches work from more than one block-order window at once.
- Bounded worker task queue (`2 * workers`) and at most `config.window_transactions <= 512` simultaneously processed/result-buffered TX observations, even for 10k/200k-TX inputs. Workers use thread-local vectors, returning one message per claim; there are no per-TX OnceLocks, serialized-TX clones, or hot-path atomic fetch/add claims. `mpsc` and `Mutex` are from the Rust standard library.
- Coordinator collects **all** claim results for a window and reduces in original source order, checking the earliest scan error or duplicate TXID. A worker panic is caught at the claim boundary and reported as `WorkerPanicked`; all workers join before this function returns. No observations from later windows are committed after an error.
- Existing default `inspect_parallel`, `inspect_parallel_one_team`, serial and owned-Vec paths remain unchanged. Source raw transaction bytes and pre-parsed `TransactionLayout` instances are borrowed.
- Trust denials remain unconditional: no authenticated active chain, parent UTXO snapshot, script/witness consensus verification, issuer/registry authorization or TradeLayer economic state is inferred.

## Verification and measurement

The new tests assert exact source-order, TXID/wTXID/Merkle, marker/candidate and duplicate-TXID error parity against the original scheduler for 1/2/4/8/16 workers, window widths 1/7/64/256/512, 1,023-TX blocks at 0/50/95% SegWit and 10,000-TX 95% SegWit blocks, plus permanently denied state transitions and invalid configuration.

The existing offline `tradelayer-block-ingest-bench` now additionally reports:

- `pooled_weighted_prepared_only_median_ms`: CPU work and ordered reduction, excluding the first structural parse.
- `pooled_weighted_vs_one_team_ratio`: same-host, same-run dynamic one-team median divided by pooled weighted median.
- `pooled_weighted_vs_windowed_ratio`: corresponding old bounded-window comparison.
- `pooled_weighted_window_result_cap`: enforced result-buffer window in TX slots.

There is **no CI speedup threshold**. Run at least 1k/4k/10k synthetic mixes and repeat with varied real raw Bitcoin blocks, a controlled physical host and peak RSS tracking before considering production adoption. The `mpsc` task receiver uses a short mutex hold per *claim*, which may still bottleneck the pool. The task's byte-work estimate is approximate because TL carrier decoding and per-job fixed costs differ.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen --test stage8d_pooled_weighted
cargo test --frozen
cargo build --all-targets --release --frozen
target/release/tradelayer-block-ingest-bench 1000 1 0 3
target/release/tradelayer-block-ingest-bench 4000 4 50 3
target/release/tradelayer-block-ingest-bench 10000 8 95 3
```

## Future consensus work

Any future Bitcoin script parallelism needs independently authenticated, parent-block-bound UTXO state, a verified PoW/chainwork and witness commitment chain, verified scripts, and an order-preserving serial state application with atomic rollback. Neither a valid local Merkle hash nor a dependency DAG authorizes a UTXO spend. TradeLayer registry and clearing mutations remain ordered and fail closed.
