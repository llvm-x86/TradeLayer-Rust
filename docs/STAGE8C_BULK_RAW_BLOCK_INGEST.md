# Stage8C — bulk raw-block zero-copy TXID ingestion

This is a **read-only Bitcoin-style raw block preindex**, not a Bitcoin Core node, wallet, chain verifier, UTXO processor, or TradeLayer settlement authority. It advances the actual user goal of handling thousands of transactions per block without thousands of per-transaction RPCs or raw byte clones. Baseline is Stage8B merged at d19bbbc6, no new Rust crates or unsafe code.

## Processing path

1. Input is one externally supplied raw-block byte buffer. The optional requested header hash is checked against the SHA256d of its first 80 bytes, but neither supplied hash nor local comparison authenticates mainnet chainwork.
2. The existing private read_only Cursor parses the canonical block CompactSize transaction count and locates exact transaction boundaries (version, BIP144 marker/flag when present, vin/vout, scripts, witness stacks, locktime). It returns full and stripped lengths without copying raw transaction bytes. The block view stores a bounded vector of Range offsets into the original buffer.
3. The source-like raw block is bounded to 4,000,000 bytes, individual source transaction to the existing 1,000,000-byte research cap, and BIP141 block weight to 4,000,000 units: three times total stripped block bytes plus total serialized block bytes, including header and count.
4. The existing parallel_txindex worker scheduler now shares its implementation between owned Vec transactions and borrowed slice transactions. Max 16 workers and 512 slots per window; independent SHA256d hashes, OP_RETURN candidates and structural parsing run concurrently. Original transaction positions, earliest errors and duplicate TXID rejection remain serial-deterministic.
5. After preindexing, Bitcoin-style pairwise SHA256d TXID Merkle root is computed in original order and compared with the supplied header root, rejecting adjacent duplicated subtree hashes. A matching header Merkle root is a LOCAL byte-consistency check, not block acceptance.
6. The caller-provided read-only fetch_and_inspect_unverified_block callback is invoked **once** with the requested block hash. It can represent a Bitcoin Core getblock(hash,0) fetch without adding any live RPC/network dependency. Transaction scan uses borrowed slices from the one returned buffer.

The parser produces no trusted state change. Real UTXO/script checks, cumulative chainwork, accepted fork, witness commitments, coinbase subsidy, in-block dependencies, fees, activation and all TradeLayer Ledger/clearing/insurance/channel effects remain outside this work. Methods requesting those permissions always return explicit errors.

## Independent fixture and adversarial testing

A synthetic 370-byte four-transaction raw block is frozen in fixtures/raw_block_ingest_synthetic.hex. It contains one coinbase-shaped transaction, one normal TradeLayer-style carrier, a SegWit carrier and a SegWit non-carrier. Its parent block is fabricated, and nBits and witness scripts do not demonstrate consensus. scripts/audit_raw_block_ingest_vectors.py rebuilds the bytes independently using Python stdlib, verifies the raw hashes, local Merkle root, stripped size 357, raw size 370 and weight 1441; witness-only mutation changes wtxid without changing stripped txid.

The Rust tests prove the returned transaction pointers borrow from the original block, exact digest/Merkle parity, serial versus parallel equality, one-callback mock fetch, earliest malformed transaction index, wrong header hash/root, noncanonical CompactSize, unsupported SegWit flag, transaction-count/truncation/trailing data, duplicate TXIDs, raw size and weight limits. A 10,000-transaction fabricated block is compared with the previous owned-Vec pathway under multiple worker counts and window sizes; every trust request still fails closed.

The release benchmark tradelayer-block-ingest-bench (default 4,000 tx, four workers, 50% SegWit) compares old per-TX Vec cloning and parallel scanning with a single bulk mock fetch and borrowed parallel scanning, including the local Merkle comparison. It prints wall time and synthetic TX/sec alongside 1 mock bulk RPC call versus N hypothetical per-TX calls. No artificial speed threshold is imposed in CI because runner CPU and mock callbacks do not measure real Bitcoin node latency.

## Limits and next work

The existing preindexer explicitly caps raw individual transactions to 1,000,000 bytes: larger otherwise-valid Bitcoin consensus transactions can be outside this research subset and are not declared globally invalid. No full Bitcoin consensus is implemented. A future production adapter needs authoritative raw-block source, PoW/chainwork/genesis or trusted checkpoint anchoring, timestamp/difficulty, full coinbase and BIP141 witness commitment, spent/created UTXOs, per-input script checks and deterministic rollback.

For *parallel consensus* Stage8D, compute a same-block prevout / read-write dependency graph and pin UTXO snapshot identities before any worker verifies independent scripts. State application must remain in canonical transaction order with deterministic conflict handling and atomic block/reorg rollback. It is unsafe to parallelize contract/ledger writes merely because TXID hashing is parallel.

### Reproduce

    python3 scripts/audit_raw_block_ingest_vectors.py
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test raw_block_ingest
    cargo test --frozen --test parallel_txindex
    cargo test --frozen
    cargo build --all-targets --release --frozen
    target/release/tradelayer-block-ingest-bench 4000 4 50 3

Merge only after exact-head full tests, frozen release, required check aggregator, and post-merge main CI succeed.
