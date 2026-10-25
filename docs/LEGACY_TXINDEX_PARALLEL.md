# File 8/49: txIndex.js and bounded parallel Rust block pre-indexing

## Goal

Increase TXID hashing and TradeLayer OP_RETURN scan throughput across thousands of Bitcoin-style transactions per block WITHOUT parallelizing state-dependent consensus execution or admitting unverified transactions. This directly addresses a throughput bottleneck: pinned JavaScript src/txIndex.js:161–195 awaits each RPC transaction fetch, decodes it, computes references, writes one document and advances the index height serially. Merely parallelizing GitHub CI does not improve runtime transaction processing.

Baseline: TradeLayer-Rust main f9769f4c after PR #121, pinned original tradelayer.js@dca0888b827c5bc28fa9189bc9e1d507f5c5670c src/txIndex.js (506 lines, original Git blob SHA 3d7bdafbdffe5af7d3250c4dcc80b8358c485597). Historical code archived byte-for-byte in fixtures/legacy_txindex_source_txIndex.txt, checked in independent JavaScript VM regression tests.

## Bounded CPU parallelism implemented

New src/txindex_parallel.rs provides:

- scan_transactions_parallel_unverified(): reads an immutable caller-provided transaction vector, splits the block into fixed-size work chunks, uses scoped std::thread workers and an AtomicUsize fetch_add cursor. Threads independently parse *complete serialized transactions*, compute double-SHA256 TXIDs and wTXIDs, and extract strictly canonical TL OP_RETURN candidate carriers using existing src/read_only.rs. No Rayon, async runtime or third-party Rust crates.
- scan_transactions_auto_unverified(): uses std::thread::available_parallelism() bounded to 16 OS threads. For fewer than 64 transactions uses one thread, avoiding spawn overhead. Config requires 1..16 workers, 1..128 TX per claimed chunk, with default chunk size 16. Requests for 16 workers against two TXs actually use two workers.
- scan_candidate_block_parallel_unverified(): accepts the existing chain::CandidateBlock shape, but **does not call or implement TrustedChainAdapter**, verify block/UTXO chain, or allow this pre-scan to authorize a ledger transition.
- All workers have private output vectors; the coordinator reassembles all results by original tx_index after all threads join. It detects missing results/duplicate claims, then fails on the **lowest-index** malformed transaction or duplicate TXID, never whichever worker happened to finish first. No partial index is published.
- Memory/resource bounds: up to 10,000 transactions/block, 32 MiB total raw serialized data, and 1 MiB per transaction (the existing research parser limit). These are software research guardrails, not Bitcoin consensus size limits or a proof of authentic accepted block weight.
- UnverifiedIndexedBlock keeps one result per original ordered TX, plus an iterator of only TL carrier candidates. Counting, raw bytes and workers used are observable. TXIDs, block membership and carrier identity are *fingerprints and syntax*, not active-chain authentication. The three permission methods (block+UTXO evidence, signed channel state and trusted execution) ALWAYS DENY.

### Critical serial boundary

Parsing and hashing are context-free. Consensus economic transitions are NOT: intra-block type7/8/9 admin/attestation changes, token balances, spot order matching, oracle updates, margin, AMM, insurance, state revisions and liquidation can depend on earlier TXs in the SAME block. The existing ChainState, UnifiedChainState and JointChainState preserve ordered apply_batch/apply_one_unverified and atomic rollback. This PR does NOT change them and does not parallelize their TrustedChainAdapter implementations, which are not required to be Sync or pure.

If future independently verified Bitcoin/Litecoin/Dogecoin nodes supply a canonical active-chain block, the pre-scan can feed a separate authenticated operation pipeline *after* independently matching TXIDs, merkle evidence, UTXO/script and sender, historical activation/list state, and signatures. Only then should the stateful reducer apply actions in the original block order, with end-of-block matching at the existing finalization boundary. Reorg invalidates pre-scan cache/results by authenticated block ID and parent; a cached untrusted txid must not be reused as proof on a different fork.

### Pipeline model

    Immutable ordered raw transaction block
      -> bounded CPU work-stealing scanner: parse + TXID/wTXID + TL carrier candidates
      -> index-preserving barrier and deterministic first-error selection
      -> optional independent full-chain and script/UTXO verifier [NOT IMPLEMENTED]
      -> canonical ordered state transition and atomic reversible block commit [existing prototypes only]
      -> indexed checkpoint tied to verified network/block ID [NOT IMPLEMENTED]

No speculative state writes or parallel ledger mutation; no parallel block height advancement.

## Source txIndex.js compatibility diagnostics

Separate read-only src/legacy_txindex_audit.rs reproduces unusual source behavior **only for source comparison**, NOT as a fallback from the strict Rust scanner. The original JavaScript:

1. picks the FIRST output with scriptPubKey.type=nulldata and searches the hexadecimal script text for the lowercase marker "746c" within the first six hex characters, including unusual odd-nibble offsets; if the first nulldata has no marker, a later TL output is ignored;
2. decodes payload with Node Buffer.from(payloadHex, "hex"), which ignores odd final nibbles or stops on invalid pairs, then checks 0x20..0x7e printable ASCII. This can accept noncanonical push scripts and even an empty body, unlike the strict Rust serialized-transaction parser;
3. calls Types.decodePayload(txId, marker, payload) with THREE arguments when that historical callee expects NINE, usually resulting in a returned JavaScript Error object rather than an authenticated decoder result;
4. filters TxUtils.getTransactionOutputs to vout 0 or 1 and maps only {address,satoshis}, discarding the original vout number. Array index cannot be used to recover the original vout when outputs arrive out of order;
5. persists under tx-{height}-{txid}, omitting block hash, network and fork identity. It saves MaxHeight during detected TL transactions, and extractBlockData calls saveMaxHeight at the end without awaiting it. This is not a crash-safe/reorg-safe canonical index or full-node history.

The byte-exact original source is exercised in a Node VM with fake ClientWrapper/RPC, TxUtils, DB and Types callbacks. The test verifies the source scanner and confirms peak in-flight TX fetches are ONE. No external RPC, wallet or real bitcore import executes.

## Correctness and throughput evidence

tests/txindex_parallel.rs constructs up to 10,000 DIFFERENT synthetic serialized Bitcoin-style transactions per block, including mixed non-TL and TL OP_RETURN, different script lengths and caller-chosen fake previous outpoints. It compares serial results with 2/3/4/8/16 worker configurations and 1/3/17/32/128 TX chunks, including the source TXID bytes, carrier candidates and original positions. It tests duplicate TXIDs, malformed low-index TX when a higher index fails first, worker cap, zero/oversized transactions, 32MiB budget, automatic small-block serial fallback and explicit rejection of economic reordering. No Bitcoin UTXO or valid signed transaction is claimed.

New src/bin/tradelayer-txindex-bench.rs is a reproducible offline measurement tool:

    cargo run --release --frozen --bin tradelayer-txindex-bench -- 8000 8

It builds 8,000 fabricated transactions with variable script lengths, verifies serial and multi-worker results are byte-for-byte identical, warms both scanners, then reports median of 5 runs: actual serial vs parallel milliseconds, TX/sec, throughput factor and workers used. No speedup threshold in CI because runner CPU allotment varies, memory/cold caches vary and parallelization can LOSE on small blocks. The CI release job runs a 2,048-TX four-worker smoke benchmark after its frozen all-target build, then asserts only the equality/security flags. Measure on real node hardware before asserting a production gain. The source test benchmark has NO active blockchain traffic, consensus signature checks, chainwork/retarget, JSON RPC network latency, database durability or full UTXO script interpreter. Such metrics remain unmeasured.

## Next throughput iterations

- **Batch full-node RPC I/O:** bounded async or fixed-size RPC fetch pool for missing raw TXs/prevout parents, keyed by (network, verified block hash, TXID/outpoint), preserving immutable block vout ordering and short-circuit on missing data. Never unlimited Promise.all per block or create a thread per TX.
- **TX candidate filtering:** after independent authenticated node block evidence, avoid redundant full transaction reparsing for non-TL outputs; benchmark the cost of SHA256d vs output walking and prioritize actual bottleneck before shortcuts. Never skip required source verification to gain speed.
- **Prepared-operation lane:** parallel CPU decode of *context-free* raw formats; all activation, list, balance, channel, oracle, PnL, spot, clearing validations remain ordered against historical state. Future data structure may partition nonconflicting balance deltas only after a formal commutativity and rollback proof.
- **Block pipeline:** prefetch next block's immutable raw transactions while current block's ordered state reducer executes. Do not commit heights out of order, mix forks/networks, or assume historic admin/root data unchanged across blocks.
- **Benchmark modes:** real Bitcoin Core-like block samples and observed live-node RPC latency, separate CPU-only scan, HTTP fetch, script/witness verification, database/index commits and full end-to-end block propagation. No invented speedup percentage.

## Reproducibility and security

    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test txindex_parallel
    cargo test --frozen --test legacy_txindex_audit
    node scripts/audit_legacy_txindex_source.cjs
    cargo test --frozen
    cargo build --all-targets --release --frozen
    target/release/tradelayer-txindex-bench 2048 4
    python3 scripts/check_rust_supply_chain.py

One new std-only scanner, one source-compatibility diagnostic, 2 Rust test suites, pinned source blob, mock-only Node oracle, reproducible CLI benchmark and CI checks. No external crates, network, private keys, settlement authority, ChainState adapter activation or trust rule change. Issue #80 remains OPEN.

File 8/49 now has real parallel pre-execution scanning primitives, but it is not a complete historical txIndex.js production parity port. Storage migration, source error handling, durable index checkpoint/reorg, verified raw tx fetch/prevout and an actual authenticated chain client remain separate proof and performance obligations.
