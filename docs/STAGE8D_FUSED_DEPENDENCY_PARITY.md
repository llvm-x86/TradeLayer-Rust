# Stage 8D1: fused TX layout hashing and read-only UTXO dependency waves

Baseline: TradeLayer-Rust main at 9c9898ada359818fa3e150ace577a3d34753ae0f, after Stage8C raw-block ingestion. This is **actual CPU hot-path work**, not CI scheduling. No third-party Rust crates, unsafe code, live Bitcoin Core node, wallet, valid signature or trusted Ledger/clearing authority.

## Single structural parsing pass per transaction

Stage8C first located raw transaction boundaries by parsing inputs, outputs, scriptSig, CompactSize and witness, then each worker called inspect_transaction() to parse them all again while computing TXID/wTXID and TL carrier positions. Stage8D1 replaces the first pass's length-only return with an immutable crate-private TransactionLayout, including full/stripped size, SegWit marker, input/output counts, Bitcoin stripped-hash spans and exact source TL OP_RETURN payload byte ranges.

read_only::prepare_transaction_prefix parses the raw block's TX suffix once, including witness fields, and records only TL marker candidate payload offsets in a Vec. For ordinary transactions without a TL carrier this Vec stays empty, with no per-TX heap allocation. read_only::inspect_prepared_transaction hashes the original raw bytes and classifies ONLY stored payload ranges: it doesn't reparse vin/vout/CompactSize/witness. Standalone read_only::inspect_transaction now reuses the same parser, enforcing exact raw-TX termination, preserving the earlier error modes. This is **syntactic** parsing and not Bitcoin Script consensus.

parallel_txindex::inspect_unverified_block_generic takes a pure read-only inspection closure. Original owned-Vec and borrowed-slice entry points still use standalone inspect_transaction. New internal PreparedRawTransaction borrows raw slices and their layouts and uses the same 16-worker/512-slot deterministic ordered scheduler, first failing position, duplicate txid detection and source order. No thread ever writes Ledger or trusted consensus state. Raw-block view now keeps a vector of layouts alongside original byte ranges; it does not copy serialized TX bytes. The already-merged independent hash256 streaming SHA256d and SegWit-v0 TXID stripping are reused exactly.

## Offline dependency DAG for potential later parallel script checks

A separate read-only, OPTIONAL pass extracts first-input/outpoint references from already-prepared layout offsets. Raw-block-view plan_dependency_waves requires an UnverifiedRawBlockIndex originating from an equivalent local block header/Merkle and lists all transactions in deterministic source-ordered topological waves, where each wave contains transactions without *intra-wave* same-block parent/child dependencies. Each input outpoint is keyed by raw 32-byte txid plus vout. Duplicate outpoint spending, future/self references, nonexistent same-block outputs, missing/duplicate coinbase-like inputs and same-block coinbase spends fail explicitly at their original transaction index.

External UTXOs remain **UNKNOWN**, even if unrelated input references fall into the same wave. No witness/script validation, coinbase subsidy, Bitcoin header PoW, chainwork, active-chain, snapshot id, timelock, input amount, signature, or TradeLayer economic state is supplied. The wave plan has no script-verification callback or state-writer API; its three authority methods ALWAYS deny actual parent UTXO, script/coinbase and canonical state application. Wavefront computation is only a scheduling hint for a future independently authenticated UTXO snapshot and deterministic serial apply.

## Comparison and truth in benchmarking

The earlier Stage8C benchmark compared an *already partitioned* owned Vec parallel scan with a more expensive one-fetch/one-block-parse/parallel scan/Merkle route. It recorded 4,000 synthetic 50%-SegWit TXs at 5.384 ms owned-only vs 9.331 ms full Stage8C, which is not equivalent work. The benchmark now reports three separate comparisons:

- Old owned-Vec scan only: a lower bound that excludes the block fetch, parsed boundaries and Merkle.
- **Full owned block pipeline**: identical mocked one-block fetch clone, header and tx-boundary check, copy N individual serialized TX buffers, old owned parallel scanner including its second parsing pass, and TXID Merkle check.
- **Full fused borrowed block pipeline**: identical mocked one-block fetch clone and header/tx-boundary check, but use already-prepared layouts, direct raw slice hashing/classification, and TXID Merkle.
- Fused-prepared-only: compare CPU work while excluding first structural parse and fetch.

The benchmark checks source-order equality in all routes and prints medians, TX/s and ratios without an artificial CI speedup threshold. GitHub runner variability and fabricated 80-byte-style TXs are NOT real Bitcoin Core throughput. Without an accessible Bitcoin Core node, a real getblock(hash,0) versus N getrawtransaction benchmark is still pending.

CI adds independent Python SHA256d wave reference against the existing four-TX fabricated raw block, plus Rust adversaries for exact source candidate/witness equality at varying windows/workers, chained same-block parent/child waves [0,0,1,2], previous external unknown waves [0,0,0,0], duplicate prevout, forward reference, nonexistent output and immature/multiple coinbase. Existing 10,000 mixed SegWit, broken serialization, block size/weight and earlier source-parity tests remain required. The final compile/release and aggregator gate stay frozen and fail closed.

## Next production-critical stage

An authenticated UTXO snapshot bound to the **parent block hash** and verified Bitcoin header/PoW/chainwork and witness commitment must precede ANY parallel script precheck. For each wave, bind each input to a consistent state (including outputs created in earlier waves), verify scripts independently but ONLY read-only, then serially apply accepted transactions in original canonical block order with atomic rollback on any failure or reorg. Coinbase maturity, BIP68/112/113, signature verification, fees and the distinction between OP_RETURN and spendable outputs remain unimplemented. TradeLayer ledger, issuer, whitelist and channel-state changes can add separate conflicts even when Bitcoin UTXO inputs are independent: never parallelize their mutations without ratified read/write sets and signed-state supersession.

This stage provides **no trusted consensus**; it only removes redundant structural parsing and prepares a dependency graph. Do not claim full Bitcoin Core validation throughput.

### Offline checks

    python3 scripts/audit_stage8d_dependency_waves.py
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test stage8d_fused_dependency
    cargo test --frozen --test raw_block_ingest
    cargo test --frozen
    cargo build --all-targets --release --frozen
    target/release/tradelayer-block-ingest-bench 4000 4 50 3

Merge only on green exact-head tests, release and required strict check; post-merge main checks mandatory.