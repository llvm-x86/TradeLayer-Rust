# Stage 8D3: Bitcoin Core RPC ingest comparison (read-only)

This is the missing **real-node RPC latency measurement tool** for
[Stage 8D Issue #130](https://github.com/patrickdugan/TradeLayer-Rust/issues/130).
It compares an identical block fetched via **one** Bitcoin Core
`getblock(hash, 0)` call against **N**
`getrawtransaction(txid, false, hash)` calls. It runs only
against an explicitly configured local Bitcoin Core mainnet node.

The benchmark uses Python stdlib `http.client.HTTPConnection`
on loopback 127.0.0.1 with the cookie you explicitly supply. It reuses
one Python process and one HTTPConnection (which may transparently
reconnect if the server closes a connection). Unlike spawning `bitcoin-cli`
N times, it does not add an avoidable subprocess-startup cost per TX.
The RPC client has an allowlist and cannot invoke write or wallet RPCs.
Nothing touches the Bitcoin zero-knowledge prover or TradeLayer ledger.

## Prerequisites and command

The local Bitcoin Core node must be synchronized on **mainnet**, serve
the target block and support getrawtransaction with a **blockhash**
argument. For older or pruned blocks unavailable to your node, the
tool fails closed rather than silently skipping transactions. The
hosted GitHub Actions workflow runs **mock RPC only**, with no node
access or credentials.

```bash
# Example height only. Choose a block your own node can serve.
python3 scripts/bench_bitcoin_core_rpc_ingest.py \
  --height 850000 \
  --cookie-file "$HOME/.bitcoin/.cookie" \
  --port 8332 --repeats 3 --warmups 1 \
  --max-transactions 10000
```

Output is a JSON report. Preserve the same Bitcoin Core version, node
machine, block height/hash, network setup, cache policy and load when
comparing runs. The output includes the Core software version, block
transaction count, SegWit count, raw block bytes and exact RPC counts.

## Timed operations versus audit

Before timing, the script calls `getblockchaininfo`,
`getnetworkinfo`, `getblockhash` and `getblock(hash, 1)` once to identify
the block and enumerate exact transaction IDs. A preflight raw block
fetch independently parses transaction boundaries, strips SegWit
witnesses, hashes txids/wtxids, rejects mutated Bitcoin Merkle siblings,
checks the header's SHA256d block hash and compares txid order to the
node's verbose response.

Each timed repeat then does both workloads, alternating which one runs
first. The stopwatch ends immediately after the RPC data returns.
**Byte-for-byte comparison happens outside that stopwatch** and
rejects a changing bulk block or individual transaction. This
separates network/RPC fetch cost from local hashing/parsing CPU.

Metrics include `bulk_rpc_fetch_only_median_ms`,
`per_tx_rpc_fetch_only_median_ms` and
`per_tx_over_bulk_fetch_ratio`. The actual HTTP response
byte total and RPC method counts are recorded by the client.

### Cache and network caveats

The verbose preflight and initial bulk fetch **warm the same node
before measured repeats**. Accordingly the JSON explicitly sets
`preflight_makes_cache_cold_claim_invalid=true`.
This is a useful *warm/repeated fetch* comparison, not a genuinely
controlled cold-cache experiment. A separate test harness would need
to control node restarts and disk/RPC cache state across trials; do not
claim that a first run measures cold disk.

The script does not separate TCP round-trip latency from Core's disk
reads, JSON encoding, HTTP processing and Python scheduling. The
per-TX route is **serial** within each round. The benchmark therefore
measures an actionable current RPC adapter workload, not the theoretical
best parallel/batched JSON-RPC transport and not whole Bitcoin block
validation. Node saturation, pruning and indexes can change results.

## Fail-closed and source bounds

The preflight requires exact 80-byte header SHA256d parity to the
reported active block, a mainnet non-IBD node, intact txid order,
bounded canonical CompactSize and BIP144 witness encoding, a locally
matching transaction-ID Merkle root, and a byte-exact correspondence
between bulk transactions and independently fetched individual bytes.

This reuses the existing **research caps**: up to 4,000,000 serialized
block bytes and 1,000,000 bytes per transaction. These are not
blanket assertions about Bitcoin's complete consensus validity.
If a legitimate Bitcoin block uses serialization beyond these
limits, report that the research benchmark cannot inspect it.

Live RPC is restricted to 127.0.0.1, requires an explicit local Bitcoin
Core cookie, and makes read-only calls only. The auth cookie never
appears in output. Tests use a deliberately fabricated four-TX block
from the Stage8C fixture; its coinbase and PoW are **not authenticated**.

A successful report has status:

`bitcoin_core_rpc_ingestion_benchmark_unverified`

No result authorizes Bitcoin script validation, chainwork or UTXO state,
nor any TradeLayer registry, orderbook, insurance, margin or channel
state transition. It is a profiling aid.

## Offline CI

```bash
python3 -m unittest discover -s tests \
  -p 'test_bench_bitcoin_core_rpc_ingest.py' -v
python3 scripts/audit_raw_block_ingest_vectors.py
```

The stand-alone GitHub Actions matrix runs Python 3.11, 3.12 and
3.13 without a Bitcoin node. Tests verify fixed SHA256d vector parity,
mock RPC call counts, SegWit wtxid/txid distinction, whole-block and
per-TX byte equality, malformed CompactSize, reordering, IBD/network
mismatches, invalid node metadata, bounds, and rejection of unsafe
RPC method names.

## Next integration

After a **real node** run, correlate these numbers with release Rust
preindex CPU time, memory allocation/peak RSS, actual transaction
size and SegWit proportion. Then decide whether RPC transport,
structural parsing, SHA256d or script/UTXO work deserves the next
parallelization budget. Keep the state transition serial and
consensus-authorized.
