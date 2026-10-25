# Stage 8B: TXID hot-path optimization for Bitcoin-style block preindexing

Baseline main c873fb286665e87b2b80e2a0f15b331149696704. Current bounded parallel_txindex already parallelizes independent raw transaction reads, SHA256d and carrier extraction across up to 16 CPU workers, 512 TX/window, with deterministic ordered results; no trusted consensus or spend path.

## Problem and exact change

Before this change, read_only::inspect_transaction hashed every non-SegWit raw serialized transaction TWICE: once for txid and again for wtxid, although BIP141 txid and wtxid are **identical** for non-SegWit transactions. Stage8B calculates double_sha256(raw) ONCE and reuses it for both fields.

For SegWit transactions, the txid must hash version + inputs/outputs + locktime, omitting marker/flag and witness stack. Previously code built a temporary contiguous Vec for that stripped serialization. A new dependency-free streaming sha256_parts/double_sha256_parts hashes precisely the three borrowed spans across arbitrary SHA256 compression/padding boundaries. wtxid still independently hashes the full raw SegWit serialization. No cryptographic truncation, alternative digest or heuristic short circuit is introduced.

The code is purely a read-only preindex optimization. It does not claim block acceptance, Merkle anchoring, strongest fork, active UTXO, signature verification or canonical TradeLayer state. Stateful consensus and registry/ledger writes remain serial and cannot be dispatched to CPU workers merely because TXID hashing is parallel.

## Test coverage

- A SHA256 known-answer-compatible streaming test compares flat sha256/sha256d with arbitrarily sliced buffers for lengths 0,1,55,56,63,64,65,119,120,127,128,129,255,256,4095,4096 and 16,384, including empty slices and one-byte fragments.
- 3 source-independent Python hashlib TXID and wtxid vectors are frozen in fixtures/txindex_segwit_hash_vectors.tsv. They include non-SegWit TL carrier and SegWit with exactly the same stripped txid but a different wtxid, plus SegWit without a carrier.
- The Rust test confirms each exact raw digest and 1,047 mixed synthetic transactions produce the same positional result in serial and 1/2/4/8/16-worker modes across 1/7/64/512 window sizes.
- The index bench now optionally generates a chosen deterministic share of SegWit transactions. Historical command defaults to 0% for baseline comparability. CI also exercises the new 50%-SegWit release benchmark without performance thresholds.

## Benchmark interpretation

Baseline GitHub Actions main run 40187560969 on 2026-10-25 measured 8,000 synthetic *non-SegWit* transactions, 512 scriptSig bytes, 4 workers, 512 window, 3 repeats: serial median 35.862ms; parallel median 16.501ms, about 484,825 TX/sec and 2.173x measured speedup versus serial. This is a SYNTHETIC CPU benchmark, not throughput of fully validated Bitcoin consensus, RPC performance or any actual chain. Compare same-run inputs before/after if possible; CPU/runner scheduling and witness shares vary, so NO asserted speedup threshold is appropriate in CI.

CI gates require all original Rust tests, no-crate frozen build, pinned source-vectors, mixed witness benchmark ordered_equivalence=PASS, and both isolated tests and release jobs green. Post-merge main CI must pass. Next real throughput limits to quantify independently: full block deserialization overhead, RPC batching/streaming raw block, allocation of per-TX Vec, witness script verification and coherent UTXO read/write sets. Those would need independently ratified consensus/UTXO adapters before parallel script or ledger validation becomes trusted.
