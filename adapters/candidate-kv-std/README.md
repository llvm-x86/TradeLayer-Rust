# Native Rust candidate segment store (research, no third-party crates)

**Status: research-only, UNVERIFIED blockchain evidence, not production database.**

This is a separate Rust command-line adapter outside the frozen
tradelayer-core Cargo package. It uses only Rust 1.90 standard library
and imports the repository's existing dependency-free SHA256d module
by source path. No new Cargo manifest, crate, unsafe code, network
access, wallet handling, or protocol/economic state changes are introduced.

This is an append/reorg-oriented immutable object store, NOT LevelDB,
SQLite, a general LSM tree, or a proof of Bitcoin best chain.
TXID and transaction input authorization remain unverified here.
A production-grade redb, rusqlite, or other storage crate would need
a separately reviewed package and dependency approval under
security/RUST_SUPPLY_CHAIN.md.

## Architecture and comparison

Draft PR #144 introduces a Python-standard-library SQLite staging
store for TradeLayer's existing Bitcoin Core raw-block and Rust
candidate streams. This independent research adapter asks if a small
Rust-only filesystem protocol can preserve the same crash and
reorganization invariants without new third-party crates.

For deployment, prefer PR #144's SQLite staging store until this
native experiment receives a separate filesystem crash, performance,
and security audit. Neither system enables live TradeLayer transactions.

## Storage layout

    candidate-root/
      CURRENT                     # atomic manifest, one active generation
      LOCK                        # writer exclusion; crash leaves stale lock
      segments/
        seg-0000000005-<sha256d>.dat
        seg-0000000006-<sha256d>.dat
        .seg-...<pid>.<nonce>.tmp  # interrupted temporary file remnants

CURRENT binds network, caller-supplied (height, block hash) anchor, local
generation, and every consecutively observed block height/hash/parent,
TX count, referenced segment name and full SHA256d digest.

Every immutable segment retains the verified-format candidate frames
for a block, including raw transaction bytes and detected TL output
indexes; zero-candidate blocks have a segment too. This remains an
UNVERIFIED observed-fork cache, with no chainwork/UTXO/script authority.

### Commit and crash sequence

1. Parse complete B/E raw-block and C/END candidate files under bounded
   size and count limits. Independently check 80-byte block-header SHA256d,
   canonical CompactSize count, contiguous heights, and parent linkage.
   The standalone adapter does NOT re-derive each transaction TXID or
   output carrier from raw block bytes. The preceding Rust scanner
   supplies that syntactic observation.
2. Acquire LOCK with OpenOptions::create_new(true), fail closed if present.
   Never automatically break a potentially live writer lock.
3. Read CURRENT. Require the incoming first block to connect to the
   existing prefix or configured anchor. A reorg drops the prior suffix
   from the next manifest, retaining immutable orphan segments.
4. Write each new content-addressed segment to a new fsynced temporary
   file, then rename it atomically to its final filename. Fsync the
   segments directory. A segment name already present can be reused
   only when the bytes match exactly. This supports idempotent replay.
5. Fsync the complete new manifest temporary file, rename over CURRENT,
   and fsync the root directory. This is the sole publication point.
   A crash before it preserves the prior manifest and its segments.
6. Release LOCK. After crash, inspect process termination and stale LOCK
   before manually removing it and retrying. Never garbage-collect
   unreferenced segment files automatically.

TLCAND_TEST_FAILPOINT is a test-only environment variable. Values
before_segment_publish and after_segments deliberately call abort()
at two different prepublication crash points.

### Limits and safety boundaries

- Maximum 256 blocks, 20,000 carriers, and 1 million raw transactions
  per batch. Each raw block <= 4,000,000 bytes.
- Each text input file <= 64 MiB; large historical backfills must be
  chunked and validated across requests.
- Segment digest and manifest parent/anchor linkage checked on verify.
  Status is cheap and does NOT read every segment.
- Filesystem protocol assumes Linux atomic rename and directory fsync.
  Other filesystems, NFS, power loss, and extreme storage exhaustion
  have not been tested against a formal filesystem failure model.
- This is one-writer with a fail-closed lock. Manual stale-lock recovery
  is required; automated crash restart is not yet supported.
- Fast per-height get is implemented. General TXID lookup, prefix seeks,
  compaction, background garbage collection and query pagination are not.
- The input remains untrusted because the store does not independently
  link C candidate raw bytes to the specific transaction body inside B
  after the existing Rust scanner has emitted them. Treat every result
  as UNVERIFIED and prohibit ledger or consensus admission.

## Reproduce

    rustup toolchain install 1.90.0 --profile minimal
    python3 scripts/check_rust_supply_chain.py
    cargo build --frozen --bin tradelayer-core-index-ingest

    rustc --edition=2021 -D warnings -O \
      adapters/candidate-kv-std/main.rs \
      -o target/debug/tradelayer-candidate-kv-std

    TLCAND_RUST_STORE_BIN=target/debug/tradelayer-candidate-kv-std \
    TLCAND_RUST_SCANNER_BIN=target/debug/tradelayer-core-index-ingest \
      python3 -m unittest discover -s adapters/candidate-kv-std \
        -p test_native_store.py -v

No Cargo manifest, lockfile, consensus module or policy file is modified.

### Pipeline with local archival bitcoind

Example is for a read-only observed candidate cache. Heights and parent
anchor must be independently reviewed; bitcoin-cli anchor by itself is
not trusted consensus evidence. A pruned node cannot supply historic
blocks beyond its retention horizon.

    set -euo pipefail
    umask 077
    mkdir -p ./candidate-data

    target/debug/tradelayer-candidate-kv-std init \
      --dir ./candidate-data/native --network main \
      --anchor-height 849999 \
      --anchor-hash "$(bitcoin-cli getblockhash 849999)"

    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT

    python3 scripts/bitcoin_core_rpc_stream.py \
      --mode blocks --network main \
      --cookie-file "$HOME/.bitcoin/.cookie" \
      --start-height 850000 --end-height 850100 --batch-size 4 \
      > "$work/blocks.tsv"

    target/debug/tradelayer-core-index-ingest \
      < "$work/blocks.tsv" > "$work/candidates.tsv"

    target/debug/tradelayer-candidate-kv-std ingest \
      --dir ./candidate-data/native --network main \
      --blocks "$work/blocks.tsv" \
      --candidates "$work/candidates.tsv"

    target/debug/tradelayer-candidate-kv-std verify \
      --dir ./candidate-data/native

    target/debug/tradelayer-candidate-kv-std get \
      --dir ./candidate-data/native --height 850000

Require both upstream RPC and Rust scanner to exit 0; partial files
without their completion sentinels are rejected by this adapter.
Candidate records must never be passed to TrustedChainAdapter or ledger.

## Recovery operator checklist

After an interrupted writer, verify the process is gone, inspect
CURRENT and orphan/temp files. Only then remove stale LOCK, if any.
Run verify against CURRENT and retry from a known parent anchor. A
completed fsynced segment reused after a crash must match byte-for-byte.
Never remove unreferenced segments blindly.

## CI and next gate

The isolated CI workflow pins checkout and Rust 1.90, checks the
zero-dependency policy and builds the frozen Stage8C scanner. It compiles
this standalone adapter with warnings denied. Python black-box tests
cover rollback, reorgs, idempotent segments, parent/network errors,
duplicate/conflicting carriers, corrupted digests, both abort failpoints
and the real four-transaction/two-carrier Rust scanner fixture.

The next significant comparison is against PR #144 SQLite on realistic
raw Bitcoin block data, recording p50/p95 ingestion latency, RSS,
throughput, storage growth, and crash/restart recovery. For production
TradeLayer throughput, prefer a reviewed native KV engine over
indefinitely expanding this bespoke research segment protocol.
