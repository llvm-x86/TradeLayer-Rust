# Durable, reorg-aware candidate evidence storage (staging adapter)

**Trust level: UNVERIFIED.** This is a separately isolated Python-standard-library
SQLite adapter for the *existing* TradeLayer-Rust candidate scanner. It does
not authorize Bitcoin consensus, UTXO spends, TradeLayer registry changes,
ledger credits, clearing, settlement, or wallet access.

Sources: `scripts/candidate_index_sqlite.py` and the independent
`scripts/candidate_block_provenance.py` structural parser. Tests include
`test_candidate_index_sqlite.py`, `test_candidate_block_provenance.py`,
`test_candidate_index_real_scanner.py` and
`test_candidate_index_reorg_scanner.py`. These introduce **zero** new Rust
dependencies and change **no** consensus kernel or Rust scanner code.

## Why SQLite first, not a Rust LevelDB fork

The repo's `security/RUST_SUPPLY_CHAIN.md` explicitly disallows introducing
third-party Rust crates into the frozen `tradelayer-core` package. Neither
`rust-bitcoin` nor a LevelDB implementation exists in its Cargo dependency
graph. SQLite through Python's built-in `sqlite3` supports atomic writer
transactions, rollback journals/WAL, cascade deletes and reliable full-sync
commits without weakening that constraint.

This adapter is a working durability prototype, not a final decision on the
storage engine. A future native Rust store should be a **separate reviewed
package**, with its crate versions, checksums, migration plan, crash tests,
writer ownership and benchmarking assessed under the supply-chain policy.

## Existing producer/consumer contract

The Bitcoin Core RPC bridge outputs raw-block frames:

```text
B<TAB>height<TAB>blockhash_display_hex<TAB>full_raw_block_hex
...
E<TAB>BLOCKS<TAB>block_count
```

The existing Rust `tradelayer-core-index-ingest` consumes that stream and
emits syntactic carrier observations:

```text
C<TAB>BLOCK<TAB>height<TAB>blockhash<TAB>tx_position<TAB>txid<TAB>vout<TAB>format<TAB>type<TAB>syntax_status<TAB>raw_tx_hex
...
END<TAB>COMPLETE_UNVERIFIED<TAB>BLOCKS<TAB>block_count<TAB>tx_count<TAB>carrier_count
```

Both completed streams are required. The block frames preserve headers even
for blocks with **zero TL candidates**; carrier-only records could not prove
contiguous chain progression or detect all reorgs.

**Independent payload binding is mandatory in the CLI ingestion path.**
After the first bounded block scan, the store re-reads all raw-block frames
and pins their SHA-256 content hashes against the first pass. A separate
standard-library parser walks every legacy or SegWit transaction, recomputes
TXIDs (including stripped SegWit serialization), checks the complete
transaction Merkle root and rejects any undeclared trailing bytes. Every
Rust candidate must match the exact raw transaction at its claimed absolute
block position, its TXID and an existing canonical single-push TradeLayer
OP_RETURN output with the claimed carrier format. Even blocks containing
no TradeLayer candidates receive a full Merkle check.

Storage-state unit tests use an explicitly named private synthetic-fixture
bypass because their tiny fabricated records are not Bitcoin transactions.
The CLI exposes **no** such bypass. Only the tests that compile and run the
actual Rust scanner establish source-byte binding.

The adapter deliberately rejects `SCRIPT` mode because its candidate rows
lack the ordered block-height and empty-block information needed to maintain
a canonical-height *observation* index.

## Reproduce and use with a local Bitcoin Core node

For a chosen historical range, initialize the candidate store with an
**externally supplied, independently reviewed** parent anchor. Example
below is an illustration, not a claim that the block hash is authenticated
or that the node is on the best-work chain:

```bash
set -euo pipefail
umask 077

mkdir -p ./candidate-data
python3 scripts/candidate_index_sqlite.py init \
  --db ./candidate-data/evidence.sqlite \
  --network main \
  --anchor-height 849999 \
  --anchor-hash "$(bitcoin-cli getblockhash 849999)"

cargo build --locked --release --bin tradelayer-core-index-ingest
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

python3 scripts/bitcoin_core_rpc_stream.py \
  --mode blocks --network main \
  --cookie-file "$HOME/.bitcoin/.cookie" \
  --start-height 850000 --end-height 850100 --batch-size 4 \
  > "$work/blocks.tsv"

target/release/tradelayer-core-index-ingest \
  < "$work/blocks.tsv" > "$work/candidates.tsv"

python3 scripts/candidate_index_sqlite.py ingest \
  --db ./candidate-data/evidence.sqlite \
  --network main \
  --blocks "$work/blocks.tsv" \
  --candidates "$work/candidates.tsv"

python3 scripts/candidate_index_sqlite.py status \
  --db ./candidate-data/evidence.sqlite

# Offline SQLite quick_check, foreign key checks, and active observed-chain
# parent linkage from the supplied anchor (not a Bitcoin consensus check).
python3 scripts/candidate_index_sqlite.py audit \
  --db ./candidate-data/evidence.sqlite
```

A historical bootstrap requires an archival Bitcoin block source. A pruned
Bitcoin Core node cannot return its removed historical block bytes.

The example writes a bounded temporary copy of raw block frames in order to
cross-check both sources before committing. This *increases* temporary I/O
versus the existing one-pass pipeline. Direct streaming to an authenticated
Rust-native storage adapter is a separate throughput optimization.

## Storage contract and crash behavior

SQLite uses `journal_mode=WAL`, `synchronous=FULL`,
`foreign_keys=ON`, and a single `BEGIN IMMEDIATE` write transaction.

- `metadata` binds schema, network, original anchor height/hash and
  `trust=UNVERIFIED`.
- `blocks` records every observed height, header hash, parent hash and
  transaction count, including blocks with no candidates.
- `transactions` stores candidate raw transaction bytes once per
  `(height, tx_position)`, with their TXIDs.
- `carriers` stores one ordered vout/type/syntax observation per
  `(height, tx_position, vout)` and cascades from its parent transaction.

Before any replacement, the incoming first header must attach by its
`prev_hash` to the exact persisted block at `start_height - 1`, or to
the configured immutable anchor. The stream must be contiguous and
parent-linked. The adapter then deletes the old suffix, inserts the new
blocks and candidate rows and validates the Rust end marker **inside
the same SQLite transaction**. Missing/mismatched/trailing frames,
duplicate/out-of-order carriers, invalid block header hashes, wrong network,
missing parents, wrong TXID, forged raw transaction bytes, incorrect block
position, missing matching output, wrong carrier format, bad Merkle roots,
and incompatible transactions abort the transaction.
On incomplete input SQLite rolls back the entire fork replacement; no
candidate partial success is published.

A repeated identical block range is an idempotent suffix replacement.
A replacement after an earlier common ancestor removes detached descendant
rows and replaces the active *observed* suffix atomically. The caller still
must determine which fork Bitcoin Core actually accepts.

## Resource bounds and validation scope

- At most 4,096 blocks and one million raw transactions per batch.
- At most 200,000 carrier rows per batch.
- Individual raw block frames at most 4,000,000 bytes; candidate raw
  transaction bytes at most 1,000,000.
- Each ASCII frame is read with an 8,000,512-byte cap before allocation.
- Bitcoin double-SHA256 of each 80-byte block header, canonical CompactSize,
  each transaction's TXID, and the block transaction Merkle root are checked
  **independently** of the existing Rust scanner.
- Strict block binding validates the precise transaction position, complete
  raw transaction bytes, TXID, vout bounds, canonical one-push TL script and
  carrier format. Raw block content changes between the two verification
  passes are also rejected before any commit.
- The second full structural parse and temporary input files incur real CPU,
  memory and I/O costs. They are intentional for this staging implementation.
- **Not verified:** Bitcoin PoW, chainwork/active tip, BIP141 coinbase witness
  commitment, UTXO/script signatures, asset validity or the Rust-derived
  status/type semantic classification. No ledger authorization follows from
  these structural checks.

There are no wallet credentials in SQLite. Protect raw data and checkpoint
anchors anyway. SQLite's WAL sidecars are part of a live database; do not
copy just the `.sqlite` file while writers are running. Use SQLite's backup
API or a clean shutdown.

## Testing

```bash
python3 -m py_compile scripts/candidate_index_sqlite.py scripts/candidate_block_provenance.py
python3 -m unittest discover -s tests -p test_candidate_block_provenance.py -v
python3 -m unittest discover -s tests -p test_candidate_index_sqlite.py -v
cargo build --frozen --bin tradelayer-core-index-ingest
python3 -m unittest discover -s tests -p test_candidate_index_real_scanner.py -v
python3 -m unittest discover -s tests -p test_candidate_index_reorg_scanner.py -v
python3 scripts/bench_candidate_sqlite.py --count 16
```

The CI suite currently includes **15 SQLite atomicity/crash tests**, **5
independent Bitcoin serialization/Merkle tests**, and **2 real Rust scanner
integration tests**. The latter cover forged transaction positions, TXIDs,
vouts, formats and raw bytes, mutated Merkle data, and an A → B → A fork
replacement with full candidate eviction and restoration.

The process-death test terminates an actual Python child with `os._exit`
*after* the SQLite transaction has deleted the prior suffix and written the
uncommitted replacement. Upon restart, the original two-block fork and
its exact candidate rows remain intact.

A separate benchmark exercises 16 tiny (370-byte, four-transaction) linked
synthetic raw blocks through Rust and SQLite, prints observed block/s,
candidate/s, commit seconds and database size, and asserts byte binding.
Its throughput is an **offline regression number**, not an extrapolation to
mainnet or a claim of a faster production index.

## Next acceptance gates

1. Confirm the complete Bitcoin Core RPC -> Rust scanner -> SQLite pipeline
   on one real local bitcoind instance, with fixed fixture heights and hashes.
2. Compare median/p95 throughput, peak memory, DB growth and crash/restart
   behavior against the current candidate stream and legacy JS index.
3. Extend the now-passing synthetic A → B → A scanner/SQLite regression to
   an actual Bitcoin Core validation-callback reorg, with independent
   active-chain checking. The endpoint tip-equality guard alone is
   insufficient.
4. Bind external PoW/chainwork/BIP141/UTXO evidence before any trusted
   TradeLayer state transition. Introduce provenance-checkpoint retention
   and recovery for pruned-node operation.
5. After the storage contract is stable, evaluate a separately pinned,
   reviewed Rust-native KV adapter. No LevelDB crate has been admitted.
