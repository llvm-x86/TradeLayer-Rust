# Bitcoin Core script-history compatibility and live raw-block indexing

**Scope:** read-only **Bitcoin** candidate discovery. This bridge neither writes
TradeLayer state nor claims a fully verified Bitcoin, Litecoin or Dogecoin chain.
Its frames are evidence, and cannot be consumed as `TrustedChainAdapter`
permissions. Nothing changes registry activation, ledger balances, clearing,
channel authorization or chain checkpoints.

Source revisions: Bitcoin Core `bitcoin/bitcoin#36629`, inspected at
`cf9b981fcc7ae52c7864beae35df5d60b5c322b4`; TradeLayer-Rust main
`c7a37eb90b1ea335051bfc3cb7712116b372cfb7`. The Core PR remains
unmerged. Its RPC name/fields can change during review.

## Why a separate raw-block path is essential

`txoscriptindex` uses a salted SipHash of SHA256 of the **entire** output
`scriptPubKey`. TradeLayer embeds distinct per-transaction bytes in its
`OP_RETURN` output scripts. The Core index cannot answer "all scripts beginning
with OP_RETURN tl". An index query for a wallet address/descriptors also cannot
find every TradeLayer transaction originating elsewhere. Consequently:

* `blocks` mode is a **complete structural OP_RETURN scan of the supplied full
  block bytes**. It uses one `getblock(hash, 0)` per block, not one
  `getrawtransaction` per transaction. Core RPC request batches and a persistent
  HTTP connection amortize latency. The pre-existing Stage8D fused borrowed
  scanner extracts markers and preserves canonical tx positions. A local
  header-hash check, TXID Merkle check, and adjacent previous-hash consistency
  checks are applied; these are NOT PoW/UTXO/active-chain proof.
* `script` mode calls new `getscripthistory(query, 0, options)` for an **exact**
  Bitcoin address, Electrum script hash, or descriptor (including ranged xpubs).
  It supports inclusive height bounds; excludes the mempool and extra spending
  transactions for deterministic historical output-discovery. It bisects
  height ranges only on the PR's 50 MiB lookup-budget errors, validates TXID
  from the returned raw bytes, records matching output numbers, and refuses
  an incomplete result if Core returns `pruned_blocks`. It never presents
  this narrow result as full-chain TradeLayer discovery.

A node with the proposed **~52 GB script index** and pruned blocks cannot
provide an all-history raw-block replay from `getblock`: historical blocks are
unavailable. For a new TradeLayer full-chain historical bootstrap, use an
archival block source or explicit peer block retrieval plus independent
verification, then archive the sparse candidate transactions and their
provenance before pruning. For ongoing forward sync a pruned Core node can
serve the current retained blocks. The suggested total **~65 GB** pruned-node
footprint comes from the author of #36629 and is not benchmarked here.

## Setup

Use a Bitcoin Core build containing #36629 with `-txoscriptindex` for script
mode. `-txospenderindex` is optional for this bridge because it deliberately
sets `include_spending_transactions=false`. A normal Core node with raw block
RPC is sufficient for blocks mode. The commands below assume an accessible
local bitcoind RPC on port 8332, `python3`, and Cargo's pinned Rust toolchain.
For a remote VPS use an SSH loopback port forward. The bridge deliberately
refuses direct non-loopback HTTP so the cookie is not sent to an arbitrary
remote endpoint.

### High-throughput whole-block candidate discovery

```bash
set -o pipefail
python3 scripts/bitcoin_core_rpc_stream.py \
  --mode blocks --network main --cookie-file "$HOME/.bitcoin/.cookie" \
  --start-height 850000 --end-height 850100 --batch-size 4 \
  | cargo run --locked --release --bin tradelayer-core-index-ingest -- --workers 8 \
  > /tmp/tl-candidates.tsv
```

On the first invocation Cargo builds the scanner. For meaningful wall-clock
benchmarks, build once and invoke `target/release/tradelayer-core-index-ingest`
subsequently. Use an archival block source for historical heights beyond the
node's pruning horizon. The `--one-team` switch opts in to the previously
benchmarked single-scoped-worker implementation; leave it disabled unless the
measured throughput and scratch-memory tradeoff justify it.

### Targeted script/address/descriptor history (experimental Core RPC)

```bash
set -o pipefail
python3 scripts/bitcoin_core_rpc_stream.py \
  --mode script --network main --cookie-file "$HOME/.bitcoin/.cookie" \
  --script 'wpkh([fingerprint/84h/0h/0h]xpub.../0/*)' \
  --descriptor-range 0:100 --start-height 850000 --end-height 850100 \
  | target/release/tradelayer-core-index-ingest > /tmp/wallet-candidates.tsv
```

Replace the example descriptor with one produced by your wallet. The Core
method accepts raw Bitcoin addresses and 64-hex Electrum scripthashes too.
For outbound wallet activity, this initial implementation will omit spender-
only transactions; support for `txospenderindex` output should be a separate
opt-in mode with its own ordering and deduplication tests.

## Transport contract

One tab-delimited UTF-8 frame per line, terminated by an explicit success
sentinel. Hex blobs contain no whitespace. Each frame is read with a strict
per-line cap before allocation.

```text
B<TAB>block_height<TAB>block_hash_display_hex<TAB>raw_block_hex
S<TAB>txid_display_hex<TAB>block_hash_display_hex_or_dash<TAB>raw_tx_hex<TAB>matching_vout_csv
E<TAB>BLOCKS|SCRIPT<TAB>record_count
```

For block frames Rust verifies advertised hash against raw 80-byte header,
contiguous height and previous hash linkage (across consecutive frames),
serialization bounds/weight, full transaction parsing, TXID Merkle root,
duplicate TXIDs, and ordered TradeLayer candidate extraction. Script frames
independently rehash each raw transaction and reject bad or repeated matching
vout indices. The source transport can contain unverified values even when all
local checks pass.

The Rust consumer writes one `C` line per detected carrier:

```text
C<TAB>BLOCK|SCRIPT<TAB>height_or_dash<TAB>blockhash_or_dash<TAB>tx_position_or_dash<TAB>txid<TAB>carrier_vout<TAB>format<TAB>type_or_dash<TAB>syntax_status<TAB>raw_transaction_hex
```

A successful run ends with:

```text
END<TAB>COMPLETE_UNVERIFIED<TAB>BLOCKS|SCRIPT<TAB>source_records<TAB>scanned_txs<TAB>carrier_count
```

**Downstream consumers must require** this exact success sentinel, zero exit
status on both sides of the pipe, and independent mainnet/UTXO verification
before treating any candidate as admissible to TradeLayer. A truncated or
erroring RPC source emits no `E` frame. The Rust consumer then exits nonzero
and omits the final `END`. Candidate `C` lines before failure are uncommitted
partial observations; do not ingest them into a persistent canonical ledger.
The recommended pipeline should write to a temp file and publish it only after
both processes succeed, rather than directly append to a production DB.

## Measurement and validation

The Python stderr log reports total JSON-RPC methods, HTTP roundtrips, elapsed
wall time, and blocks/s or returned history records/s. Rust stderr reports
transactions scanned per wall second and syntactic TL candidate counts.
The Rust timer includes waiting for producer frames when the scanner is
already listening, so it captures RPC latency and CPU time for a live pipe.
No speedup threshold is asserted from offline fixtures.

```bash
python3 -m unittest discover -s tests -p 'test_bitcoin_core_rpc_stream.py' -v
bash scripts/ci-preflight.sh --full
cargo test --locked --test bitcoin_core_index_ingest
```

Offline Python tests use synthetic RPC replies, including unordered batch
responses, reorgs, 50 MiB bisection and pruned-data failure. Rust integration
tests use the existing `raw_block_ingest_synthetic.hex` fixture and assert
canonical candidate order, raw TXID checks, hash chaining, explicit completion
and fail-closed mismatches. End-to-end timing must ultimately be repeated
against a real Core node with the #36629 branch, and compared with legacy
sequential per-TX RPC and Stage8C/8D block benchmarks at 1k/4k/10k TX sizes.

## Remaining proof and performance work

1. Persist sparse TL candidate bytes, block proof/position and versioned
   checkpoint anchors, with atomic reorg rollback. Retain raw bytes before
   the pruned Core node discards underlying blocks.
2. Bind source block hashes to active-chain work, PoW/difficulty, BIP141
   witness commitments, UTXO and signature checks, and legacy source
   activation before any protocol mutations.
3. Add an optional spender-history channel and an explicit `getblockfrompeer`
   recovery queue; fail until missing block contents are independently verified.
4. Measure live p50/p95 block lag, Core disk/cache pressure, RPC call rates,
   bounded memory and exact serial-equivalent TradeLayer replay/checkpoint
   roots. Current implementation yields a read-only candidate stream, not
   production consensus ingestion.
