# Stage 8D2: static SHA256d-work partition experiment

Base: \`main\` at c7a37eb90b1ea335051bfc3cb7712116b372cfb7 after Stage 8D1b (#132). This is optional, read-only Bitcoin-style raw transaction CPU indexing, with no external Rust crates, unsafe code, network fetch, Bitcoin Core script/UTXO consensus or TradeLayer economic authority.

## Evidence prompting this experiment

On post-merge main CI run 40202976099, the release synthetic \`tradelayer-block-ingest-bench\` reported a repeated median for *prepared-only* windowed vs one-team indexing, using identical parsed raw block input and local TXID Merkle checking:

| Transactions | Witness | Workers | Windowed | One team | Ratio (windowed / one team) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000 | 0% | 1 | 2.230 ms | 2.206 ms | 1.011 |
| 4,000 | 50% | 4 | 8.613 ms | 7.553 ms | 1.140 |
| 10,000 | 95% | 8 | 27.508 ms | 21.307 ms | 1.291 |

These single-run, runner-dependent data suggest a scoped worker team avoids substantial per-window spawn/join overhead. The present PR tests whether the atomically allocated dynamic microclaims and per-TX OnceLock slot writes in one-team mode impose avoidable overhead.

## Static balanced byte work

\`inspect_prepared_block_weighted_shards\` validates the same worker/window limits, TX count, individual raw-TX bounds and total raw byte cap before scheduling. On the single fused structural parse, it estimates each job's SHA256d data processed: raw byte count for legacy; raw plus stripped byte count for SegWit, whose TXID differs from wTXID. It computes up to 16 deterministic *contiguous* source-order ranges with an equal-work target; each range is owned by one scoped CPU thread. Every worker hashes and classifies its slice of immutable prepared transactions and returns thread-local observations. There is no global atomic fetch_add, per-TX OnceLock or shared writable worker output. Only after all joins does the coordinator reduce source order, check duplicate raw TXIDs and compute local Merkle exactly as before.

An error still aborts the entire report at its earliest source index. A worker retains the preceding observations on scan failure, so a lower-index duplicate detected by the coordinator takes precedence over a later per-TX scan failure. Every worker joins even when another worker failed or panicked. The public method is opt-in \`UnverifiedRawBlockView::inspect_parallel_weighted_shards\`; existing \`inspect_parallel\`, \`inspect_parallel_one_team\`, serial and standalone owned-Vec APIs are untouched.

Memory is bounded by the existing maximum block transaction count and input-byte cap, with one returned observation per scanned transaction plus worker-local result vectors; unlike default windowed mode, it can retain a whole block's results before reduction. The partition is a heuristic for SHA256d bytes, not an exact cost model for candidate payload classification or heterogeneous CPU behavior. A massive single TX or imbalanced carrier work may make a static shard slower than dynamic scheduling. The current benchmark is synthetic, performs no Bitcoin Core RPC or authenticated chain verification, and has no performance acceptance threshold.

## Regression gates and decision

- Pinned four-transaction raw block parity across 1/2/4/8/16 workers and window configurations 1/7/64/256/512.
- Synthetic 1,023-TX blocks at 0/50/95% witness and 10,000-TX 95% witness stress, with strict TXID/wTXID/OP_RETURN carrier/candidate source ordering, local Merkle and duplicate-TXID error index equality.
- Existing fail-closed Bitcoin chain, UTXO/script and TradeLayer state gates stay permanently unavailable through this method.
- Full frozen \`cargo fmt --all -- --check\`, clippy, tests, release build and required-check aggregate.
- The release benchmark prints \`weighted_shards_prepared_only_median_ms\` and \`weighted_shards_vs_one_team_ratio\` in each existing workload. Promote this scheduler only after repeated measurements demonstrate a sustained end-to-end improvement on representative real raw Bitcoin blocks, with peak RSS and latency tails tracked.

\`\`\`sh
cargo test --frozen --test stage8d_weighted_shards
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen
cargo build --all-targets --release --frozen
target/release/tradelayer-block-ingest-bench 1000 1 0 3
target/release/tradelayer-block-ingest-bench 4000 4 50 3
target/release/tradelayer-block-ingest-bench 10000 8 95 3
\`\`\`

A valid optimization here affects *preindexing only*. Trusted UTXO snapshot binding, witness commitments, PoW/chainwork, signature/script validation, ordered Bitcoin state apply and atomic reorg rollback remain separate engineering requirements.
