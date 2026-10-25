# File 8/49: txIndex.js source parity and runtime scanner micro-batching

Historical pin: patrickdugan/tradelayer.js@dca0888b827c5bc28fa9189bc9e1d507f5c5670c, src/txIndex.js (506 lines; Git blob 3d7bdafbdffe5af7d3250c4dcc80b8358c485597). Exact original file is frozen in fixtures/legacy_txindex_source_txIndex.txt.

## Runtime throughput result already merged in Stage8 baseline

PR #125, merged into main at d19eae07, implemented src/parallel_txindex.rs and a repeatable source-order benchmark. It parallelizes:
- Independent read-only raw transaction fetching, via bounded scoped Rust worker threads and caller-owned immutable idempotent RPC callback, including local SHA256d TXID verification of each fetched result.
- Independent Bitcoin-style wire-serialization parse, TXID/wTXID hashing, strict OP_RETURN classification and TL candidate indexing across the caller-supplied ordered block.
- Deterministic output slots (one per original tx index), 512-transaction maximum windows, total raw input budget 32MiB, per-TX 1MiB, up to 16 OS workers. Duplicate/wrong txid, malformed bytes, fetch error, worker panic, missing output and overruns fail closed.
- A hard barrier before any state mutation. Existing ChainState, UnifiedChainState, JointChainState, Ledger and Stage25G economic trust requirements remain unchanged. Parallelizing consensus state updates without a proven commutative/reversible schedule would be incorrect.

An actual synthetic GitHub Actions debug-mode benchmark from main CI #40186135714 measured 1,500 fake BTC-style TXs, 876,139 raw bytes, serial 119.690ms, four-worker 54.381ms, **2.201x scan speedup**. This does NOT measure real chain consensus, signature/witness execution, actual Litecoin/Dogecoin Scrypt, RPC across a network or DB writes. There is no live node adapter.

## This PR: microbatch work scheduling, without duplicating the runtime

This follow-up does NOT create a second parallel indexing engine. It changes the existing work_window scheduler's atomic TX cursor from one claim per TX to a small dynamic microbatch. For each bounded window:

    live_workers = min(requested_workers, window_length)
    claim_size   = clamp(window_length / (live_workers * 4), 1, 8)
    begin        = atomic_fetch_add(claim_size)
    independently inspect items [begin, min(begin+claim_size, window_end))

There are at least four scheduling opportunities per worker for normal windows. Small 1–7 item windows and asymmetric slow RPC responses retain fine granularity. Each result still goes into an independent OnceLock slot indexed by the original block position. All workers join before publication; the lowest index wins errors, never fastest completion time. Memory/window caps and trusted adapter boundaries are unchanged. This may reduce atomic coordination cost on large cheap-TX scans; any speedup or slowdown must be MEASURED and cannot be assumed.

The additional 10,000-transaction synthetic test checks identical complete serial and parallel results for 2/4/8/16 worker configurations and 32/64/256/512 TX windows. The original already-green regression suite covers duplicate TXIDs, stale/fork reinspection, wrong RPC bytes, out-of-order callback completion, malformed TX and canonical-source-first error. A separate 8,000-TX frozen release-mode smoke benchmark runs without a speedup threshold, checks ordered_equivalence=PASS and prints real throughput. The measured CI job-to-job difference is not a statistically valid production claim.

## Independent original-JS source-characterization audit

New src/legacy_txindex_audit.rs is a separate **non-authorizing source-quirk observer**. It models the pinned JavaScript, NOT the strict modern Rust scanning rules. Historical txIndex.js:
- Reads FIRST scriptPubKey.type=nulldata and looks for literal lowercase ASCII-hex "746c" at nibble offsets at most 6. It accepts noncanonical odd-position marker hex; a later valid TL nulldata output is ignored if the first lacks TL.
- Decodes the following substring with Node Buffer.from(payloadHex, "hex"): odd final nibbles or bytes after the first invalid hex pair may be dropped, then printable 0x20..0x7e is checked. Strict Rust complete-transaction parsing must NOT silently adopt this permissive scan.
- Calls Types.decodePayload(txId, marker, payload) with **THREE arguments**, but Types.decodePayload expects NINE. The source usually gets a returned Error OBJECT rather than an admitted decoded transaction.
- Projects raw TxUtils outputs at vout 0 and 1 to just address/satoshis, discarding the original vout indices. A supposedly "reference[0]" item cannot be assumed to be actual vout=0 if the source ordering differs.
- Stores tx-{height}-{txid} keys without a block hash or network, and saves MaxHeight after every TL candidate but does not await final saveMaxHeight. Historical source key collisions and crash/reorg hazards remain unresolved.

scripts/audit_legacy_txindex_source.cjs checks the original Git blob SHA1 and executes its real code under a fully fake RPC, TxUtils and NeDB sandbox; it verifies the single-in-flight historical JS block loop, marker offsets, first-nulldata choice, and exact three-argument call. tests/legacy_txindex_audit.rs checks matching Rust source diagnostics and denial behavior. Unusual source acceptance is observational and not automatically allowed in the strict Rust parser. No host app, wallet, network credentials or live historical state is imported.

## Trust and next bottlenecks

The source audit reports and parallel indexing reports are not transaction validators. They do not independently prove PoW, chainwork, Merkle linkage, witness/script, first-vin parent ownership, LTC/DOGE source, activation, whitelist or channel agreement. The consensus reducer must remain deterministic and apply all verified protocol mutations at original tx_index in block-height order, with atomic reorg undo and the same ledger state root regardless of worker scheduling. A 2-of-2 L1 ECDSA spend is not an off-chain signed channel-state revision.

After this source/throughput step:
1. Add independent full-node block ingestion: fetch a whole block or use local blk.dat in one request where possible, not per-TX RPC; separately parallelize missing prevout fetches with a bounded byte cache keyed by network/fork and outpoint. Never use a claimed txid as an authority token.
2. Measure CPU SHA256d scan, actual RPC I/O, UTXO/witness validation and sequential TradeLayer state application separately. Once a real trusted adapter exists, schedule dependency-aware signature verification only when source-read sets are independent, then apply transitions in block order.
3. Bind checkpoint/index keys to verified (network, block hash, height, tx_index, txid) and reject rollbacks/reorgs lacking authenticated roots; do not reuse the legacy height+txid keys as consensus state.
4. Resolve the source's incorrect Types.decodePayload argument contract and output-reference vout loss under a versioned historical compatibility rule. Cross-check actual accepted historical carrier bytes before claiming a repaired codec is canonical.

## Offline reproductions

    node scripts/audit_legacy_txindex_source.cjs
    cargo test --frozen --test legacy_txindex_audit
    cargo test --frozen --test parallel_txindex
    cargo test --frozen
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo build --all-targets --release --frozen
    target/release/tradelayer-index-bench 8000 512 4 512 3

Zero new crates, wallet, live nodes, signed transactions, ledger changes or consensus adapter mutation. Source-data parity is a bounded subset; File 8/49 isn't a production-complete indexing or consensus migration.
