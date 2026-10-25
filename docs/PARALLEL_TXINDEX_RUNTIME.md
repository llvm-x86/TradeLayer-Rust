# Runtime txIndex.js throughput: parallel fetch/scan, ordered consensus boundary

This addresses BITCOIN BLOCK PROCESSING THROUGHPUT, not GitHub CI parallelism. Historical pinned patrickdugan/tradelayer.js@dca0888b, src/txIndex.js:161–196, fetches and decodes every TX sequentially in a loop. Its processTransaction also passes three arguments to the nine-argument Types.decodePayload, and loses original vout references. This PR parallelizes only the read-only work, not authenticated consensus transitions.

## Implemented runtime shape

    caller-supplied ordered raw txids or serialized block transactions
                              |
           bounded 1–16 read-only worker threads
           concurrent independent fetches (optional)
           raw SHA256d txid+wtxid, strict full structural parse
           OP_RETURN carrier extraction, no historical validity claim
                              |
            bounded windows of 1–512 indexed result slots
                              |
        reassemble in ORIGINAL BLOCK TX POSITION, not finish order
                              |
            complete ordered scan report + candidate positions
                              |
        (separately) independently verified Bitcoin block/UTXO chain
                              |
         ONLY THEN strictly ordered state-dependent consensus apply

src/parallel_txindex.rs offers inspect_unverified_block_parallel and inspect_unverified_block_serial on already-fetched serialized Bitcoin-like transactions. Both reuse read_only::inspect_transaction, thus using the exact existing parser rather than a forked binary implementation. The optional fetch_and_inspect_unverified_parallel fetches raw transactions using a caller-supplied thread-safe READ-ONLY callback; it recomputes the locally derived raw SHA256d TXID and REJECTS a callback reply that does not match the original requested raw TXID and absolute tx index. A serial fetch reference exists for differential tests.

Workers get tasks through AtomicUsize and one OnceLock result per input within a window, without locking an order-dependent state map, spawning one thread per transaction, or mutating a Ledger. All threads join before results are emitted in transaction order. Every input error is reported at the EARLIEST BLOCK INDEX even if a later worker reports first in wall-clock time. A failing block returns no partial report or state transition. Caller-side read-only/idempotent RPC and full chain ownership remain external obligations.

## Resource and trust bounds

- Default worker count is min(local available CPUs, 8), configurable 1–16.
- Window count is 1–512 TXs, default 256. Windows bound concurrent fetch and queued results.
- Each raw TX <=1,000,000 bytes per existing parser; total raw bytes <=32 MiB and <=200,000 TXs per indexed block. These are RESOURCE LIMITS, not Bitcoin consensus block rules.
- Repeated raw TXID, empty/oversized/malformed TX, invalid config, callback error, wrong fetched TXID, worker panic and too-large block fail CLOSED. No changes to ChainState::apply_block or TrustedChainAdapter.
- Every report contains ALL observed transactions and absolute original positions, and exposes ordered candidate positions for cheap TL-only downstream interpretation. It does not claim output legitimacy, verified witness, actor, list membership, price, or activated rule.
- require_authenticated_block, require_authenticated_transaction_state and require_canonical_signed_channel_revision always return errors. Indexed output cannot become signed operations or authorization merely by matching a TXID.

The costly original JS RPC-per-TX sequence can now be overlapped in the caller-supplied read-only fetch function. CPU parsing and TXID SHA256d can use multiple cores. This PR does NOT yet replace any production Bitcoin Core event-loop or offer a live node adapter; there is no claim about live blockchain throughput until a real trusted ingestion service calls this API.

## Why consensus application must remain ordered

An intra-block transaction may spend another earlier transaction's output, create a property used by a later transaction, change a whitelist administrator, revoke membership, fund collateral, or alter an orderbook. Independent parsing and TXID hashing can run in parallel, while valid UTXO/script checks must receive appropriately staged immutable snapshots; dynamic ledger, registry and orderbook transitions depend on canonical order. Naively running the existing TrustedChainAdapter's state-dependent authenticate_and_decode or Ledger.apply_batch from worker threads would compromise deterministic replay and reorg guarantees.

A future trusted execution stage can compute a UTXO dependency graph and process independent proof waves with validated historical state, then apply ALL economic actions in original block order (or mathematically proved serial-equivalent schedule), atomically with exact reorg undo. Existing ChainState and Stage25G permission rules are untouched. Full Bitcoin block/chainwork/BIP141/UTXO/witness validation, Litecoin Scrypt and Dogecoin AuxPoW remain separate.

## Adversarial tests and measurable benchmark

tests/parallel_txindex.rs generates 1,047 synthetic, structurally parseable Bitcoin transactions with changing scripts and TL payloads and proves identical serial and parallel full reports under 30 combinations of worker count (1/2/3/4/8/16) and window count (1/7/64/256/512). Additional tests mock concurrent RPC with intentionally scrambled completion, verify raw requested TXID binding and first error in block order, duplicate tx rejection, truncated inputs, malformed resources, bound checks and reinspection on a changed fork. No live node or money.

src/bin/tradelayer-index-bench.rs is an executable offline benchmark of identical synthetic raw transactions in serial versus multicore mode. It prints actual median milliseconds, transactions/second, input bytes, worker/window count, and measured speedup, and checks FULL ORDERED RESULT EQUALITY every run. No speedup threshold is asserted: small workloads or one-core CI runners can be slower with threads. The benchmark excludes network RPC time and Bitcoin signature validation. Tune windows and worker counts only from real measurements.

Local reproduction:

    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test parallel_txindex
    cargo test --frozen
    cargo run --frozen --bin tradelayer-index-bench -- 1500 512 4 256 3
    cargo build --all-targets --release --frozen
    python3 scripts/check_rust_supply_chain.py

## Remaining end-to-end throughput and correctness work

1. Integrate a source-compatible Bitcoin Core raw-block fetch or local blk.dat reader and measure one whole-block request against old per-TX RPC, including request coalescing and parent-output caches.
2. Bind provided block TXIDs to independently validated Bitcoin active-chain Merkle/witness commitments and script/UTXO evidence before interpreting transaction value.
3. Use validated source-only state and intra-block dependency graph for safe parallel signature verification; serialize consensus-sensitive registry, balance, orderbook and clearing application in block order.
4. Bench large real historical blocks on a dedicated worker using CPU utilization, wall-clock block lag, p50/p95, RPC rates, memory and deterministic root replays. Gate any production adoption on exact same ledger roots and reorg recovery as a proven reference.

Do not confuse faster indexing with proof of a real TradeLayer transaction, sender authority, or off-chain co-signed state revision. Issue #80 and issue #122 remain open for source compatibility and chain/settlement proof obligations.
