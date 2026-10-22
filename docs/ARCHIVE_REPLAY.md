# Litecoin archival evidence and JavaScript/Rust comparison

**Status:** research tooling. Neither this collector nor the accompanying
legacy decoder comparison is a consensus implementation. They never authorize
balances, sign transactions, or broadcast to Litecoin.

## Evidence classes

- **Historical fixture**: published Bitcoin transaction bytes used to check
  basic transaction parsing, not TradeLayer on-chain activity.
- **Synthetic fixture**: handcrafted TradeLayer OP_RETURN payload inside a
  syntactically valid Bitcoin-family transaction, never claimed to be mined.
- **Node-backed archival observation**: raw Litecoin transactions returned by
  an unpruned, fully synchronized **locally trusted Litecoin Core** instance.
  Each block is checked against the node's best-chain height, previous hash,
  confirmation depth and reported transaction Merkle root. Each selected raw
  transaction is checked independently against the txid and exact decoded
  OP_RETURN output script. Every observed height is rechecked after scanning.
  The node supplies consensus/PoW verification. These checks do not independently
  prove PoW, signatures, canonical-history stability after collection, or that
  the TradeLayer protocol accepted the payload.

**The repository currently contains no node-backed TradeLayer transactions.**
This is an intentional evidence provenance gate. Supplying a made-up height or
block hash alongside a raw payload cannot promote a synthetic fixture into
historical evidence.

## Collect from Litecoin Core

Prerequisites: an unpruned, fully synced Litecoin Core node with local RPC
enabled; a functioning local authentication cookie or `TL_RPC_USER` and
`TL_RPC_PASSWORD` environment variables. The script accepts only loopback HTTP
endpoints, bypasses environment proxies, rejects redirects, and uses only read
RPCs. Never put RPC secrets in command-line arguments or Git commits.

A **candidate testnet search window** appears in published TradeLayer API
examples at heights 3,348,377 and 3,348,528. Those heights are leads, not
independently confirmed protocol activity. On the appropriate historical
Litecoin testnet, run:

```sh
python3 scripts/collect_litecoin_rpc.py \
  --network test \
  --start-height 3348377 \
  --end-height 3348528 \
  --output archive/testnet-3348377-3348528.tsv \
  --manifest archive/testnet-3348377-3348528.json
```

For mainnet, specify `--network main` and an explicit height window. Runs are
limited to 500 blocks; split larger scans into ordered ranges and keep each
JSON provenance manifest. A block with zero candidates is a valid negative
observation and its manifest records the scanned interval. Files in `archive/`
should generally remain local until their source and privacy implications have
been checked. The collector writes files only after successful validation and
includes a SHA-256 digest of the TSV in its manifest.

```sh
cargo run --locked --bin tradelayer-scan -- \
  archive/testnet-3348377-3348528.tsv
```

The Rust scanner recomputes txids from the raw serialized bytes and identifies
legacy `tl` / proposed `TLR` payloads without modifying the ledger.

On standard Linux installations, the collector looks for
`~/.litecoin/testnet3/.cookie` on Litecoin testnet,
`~/.litecoin/regtest/.cookie` on regtest, or `~/.litecoin/.cookie`
on mainnet. Use `--cookie-file` for alternative data directories.
See [Litecoin Core's data directory documentation](https://github.com/litecoin-project/litecoin/blob/master/doc/files.md).

## Actual JavaScript decoder comparison (opt-in)

First generate Rust's typed observations from the **same** raw corpus:

```sh
cargo run --locked --bin tradelayer-legacy-fields -- \
  archive/testnet-3348377-3348528.tsv > archive/rust-fields.tsv
```

Use a **trusted, pinned, local** historical `tradelayer.js` checkout whose
runtime dependencies are already installed in an isolated offline environment:

```sh
node scripts/compare_legacy_js.cjs \
  archive/rust-fields.tsv \
  ../tradelayer.js/src/txDecoder.js \
  > archive/differences.json
```

The comparator calls the actual `decodeActivateTradeLayer`, `decodeTokenIssue`,
`decodeSend`, `decodeGrantManagedToken` and `decodeRedeemManagedToken` functions.
It reports **match**, **disagree**, and **inconclusive** by transaction. It skips
unsupported bodies and unsafe-to-compare JS number magnitudes, and separately
tracks the known multi-whitelist discrepancy. Results depend on the pinned JS
source and its installed BigNumber version; record their commit and lockfile
hash. Any mismatch requires historical protocol research before implementation
changes. Rust's parser and JS's decoder are independent approximations of
ledger validity, and neither may issue or mint assets directly.

**Security:** loading a CommonJS decoder executes top-level JavaScript from
that checkout. `node:vm` would not make arbitrary untrusted code safe. Execute
this opt-in command only in an isolated, networkless environment with no
wallet keys, RPC credentials, or sensitive mounts. CI uses a small fake decoder
to test comparison plumbing; it deliberately does not install or run the old
511-instance dependency graph.

Stage 15 adds a [cross-root audit inspector](TLPT_ZKTL_ALIGNMENT.md) that can consume declared per-block transaction observations. The existing archival collector has not yet been adapted into an authenticated observation provider, and no collector output is promoted to consensus evidence by the inspector.

## Remaining checks

- Acquire actual chain-backed `tl` payloads and commit only reviewed excerpts
  with provenance. Some testnet branches may no longer be served by current
  network peers; obtain the matching archive before concluding a block is empty.
- Establish issuer/sender provenance by verifying previous outputs, input
  ownership, signatures and consensus-specific sender-selection rules.
- Audit trade settlement, vesting, activation timing, address restrictions and
  every other transaction type separately. The v1 binary envelope is a new
  protocol proposal without activation rules.
- Replace this read-only evidence path with a rigorously verified adapter only
  after cross-implementation replay agrees on the canonical test corpus.

References: [Litecoin Core RPC implementation](https://github.com/litecoin-project/litecoin/blob/master/src/rpc/blockchain.cpp),
[raw transaction RPC](https://github.com/litecoin-project/litecoin/blob/master/src/rpc/rawtransaction.cpp),
[TradeLayer example endpoints](https://docs.tradelayer.org/api_reference/endpoints).
