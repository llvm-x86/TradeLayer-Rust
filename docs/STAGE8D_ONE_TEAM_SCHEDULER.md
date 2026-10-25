# Stage 8D1b: one scoped CPU worker team per raw block

This work is an **experimental performance variant**, not a change to Bitcoin or TradeLayer consensus. Baseline main 7dffdadc after 8D1 merged fused structure parsing. Frozen Rust CI and no new external Rust crates, unsafe code, signed state or real network.

## Why this may improve throughput

The original \`parallel_txindex::work_window\` creates and joins scoped threads for every 512-transaction window. For 10,000 transactions with eight workers and 512-window mode it spawns up to 160 short-lived worker threads. Each worker hashes serialized Bitcoin-style raw bytes, classifies OP_RETURN candidates and writes results to its own deterministic index slot. Bitcoin source transaction positions must remain ordered for UTXO and TradeLayer consensus.

The opt-in \`inspect_parallel_one_team()\` instead uses **one thread::scope per block**, at most 16 worker threads, a global atomic index and bounded microclaims of 1–16 TX indices per worker. Every worker only examines immutable prepared raw transactions; it cannot verify scripts or mutate UTXOs/ledger/clearing. Workers place read-only results in per-position OnceLock slots; after **all threads join**, the coordinator sequentially constructs the existing ordered report, detects duplicate TXIDs, and returns the earliest failing block index.

### Explicit memory tradeoff

The existing default design bounds transient completed window observations to 512. One-team mode allocates one result slot for each TX upfront (maximum 200,000 indexed transactions with the existing 32MiB input byte cap) and retains all completed results until reduction. This is **linear bounded scratch memory**, not constant scratch memory. Therefore the existing windowed \`inspect_parallel\` remains the default; one-team is opt-in until an apples-to-apples throughput/memory benchmark demonstrates stable benefit. No background cache across blocks/forks and no hidden executor authority.

## Tests and benchmarks

Use the independent Python frozen four-TX Bitcoin-style block and existing Stage8C 10,000 mixed witness/non-witness synthetic block, plus in-block parent/child deterministic read-only dependency waves. Verify one-team matches windowed and serial exactly for 1/2/4/8/16 workers, window size 1/7/64/256/512, local Merkle and candidate positions, source error ordering, duplicate TXIDs, and all permanent-denial UTXO/script/ledger gates.

The release \`tradelayer-block-ingest-bench\` preserves original outputs and additionally reports:
- \`one_team_prepared_only_median_ms\` — one shared TX layout pass already completed, single scoped worker team, ordered reduction and Merkle.
- \`one_team_vs_windowed_ratio\` — old prepared-only divided by one-team prepared-only, equal work, same input, same core logic except worker scheduler.
- \`one_team_extra_result_slots\` — number of block TXs buffered by the experimental path.

Compare 1000/4000/10000 TX and 0/50/95% SegWit; neither the CI workflow nor PR narrative may claim a speedup before actual same-run evidence. A benchmark on GitHub runners is not a valid proxy for Bitcoin Core RPC latency, UTXO disk lookups, script signature verification or live consensus.

## Next work

Even a faster CPU preindex is NOT full Bitcoin consensus processing. The next substantive stage requires independently authenticated parent UTXO snapshots, same-block dependency-aware read-only script verification, correct coinbase/witness/PoW/chainwork handling, deterministic serial state application and atomic reorg rollback. State changes to TradeLayer registry/clearlist/channel/margin and economics remain serial and default deny until source-validated.

    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test stage8d_one_team
    cargo test --frozen
    cargo build --all-targets --release --frozen
    target/release/tradelayer-block-ingest-bench 10000 8 95 3

Merge only with entire frozen tests+release+required check green on the exact PR head, then post-merge main verification.
