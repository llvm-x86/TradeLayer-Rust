# Read-only historical transaction scanner

**Status:** independent evidence inspection only. This code is deliberately separate from `TrustedChainAdapter` and `ChainState`. Nothing here authorizes transfers or validates a TradeLayer block.

## Interface

`read_only::inspect_transaction(raw)` parses the common Bitcoin-family transaction serialization, checks minimally encoded CompactSize fields, handles legacy and SegWit-v0-style witness records, computes a standard transaction ID and witness transaction ID, and inspects exactly one canonical `OP_RETURN <push>` script per output. A push beginning in lowercase `tl` is passed to the historical **diagnostic** parser. Uppercase `TLR` is handled by the proposed binary v1 parser. Other pushes are reported only in the OP_RETURN count. Nested, unusual, or noncanonical scripts do not generate TradeLayer candidates.

`read_only::replay_tsv(tsv)` verifies each record's expected txid against the supplied bytes and orders observations according to the input rows. It rejects duplicate transactions on the same labeled network and decreases in reported block heights. It treats supplied network names, block heights, and block hashes as **unverified labels**; transaction self-hashes are no proof of inclusion in a canonical chain.

The CLI has no networking or wallet code:

```sh
cargo run --locked --bin tradelayer-scan -- fixtures/real_transactions.tsv
cargo run --locked --bin tradelayer-scan -- fixtures/synthetic_transactions.tsv
```

Columns in TSV: `label`, `network`, `reported_height` (or `-`), `reported_block_hash` (or `-`), `expected_txid_display_hex`, `raw_transaction_hex`. The reader rejects malformed, duplicate, or mismatching evidence.

## Actual public evidence

- `bitcoin-genesis` is the Bitcoin mainnet genesis coinbase (204 bytes); known txid `4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b`, block height zero.
- `bitcoin-rpc-example` is a published 258-byte non-SegWit Bitcoin transaction from [opreturn.net's API documentation](https://opreturn.net/api/docs/tx/), txid `d38ff6587f43cea5a399462bc735920f30073bbe06b5d7df5154196c5dd8d637`.
- `segwit-opreturn-example` is the 246-byte Bitcoin **regtest** transaction in the [Bitcoin Stack Exchange OP_RETURN question](https://bitcoin.stackexchange.com/questions/116869/how-can-i-recognize-an-op-return-transaction-looking-only-at-the-hex). Its independently reported txid is `00fddd900b735a88ab0ef5eeaa3efbc869d4e52493da8a9467ec782355cb39ff`, distinct from witness hash `b4c4817e908d9ba317e9b1a08180b51c0ce77adca946e7f894560fb07447acf8`. The test example is not evidence of any public-chain inclusion.

Where a source supplies no block height/hash, the fixture records `-`. The parser does not infer missing metadata. These first three fixtures establish raw transaction parsing, witness handling, and txid fidelity; **none is a confirmed TradeLayer payload**.

## Synthetic regression cases

`fixtures/synthetic_transactions.tsv` has offline-crafted Bitcoin-serialization envelopes carrying `tl20;alice;5;2`, proposed binary `TLR`, and the double-prefixed legacy form. Their txids are calculated, but these bytes were never claimed to be broadcast or mined.

## Outstanding security work

Implement a **separate** chain-specific reader for Litecoin, Bitcoin and Dogecoin; derive provenance only from a locally verified node's block headers, canonical chain and full transaction index, with tx inclusion and input UTXO ownership checks. Collect confirmed historical TradeLayer transactions from an archival node, including block height/hash, tx position, raw OP_RETURN bytes, and actual old-node validation results. Independently audit the SHA-256 implementation or replace it with a pinned, reviewed hashing crate before building a trusted adapter. Compare legacy `tl` behavior against chain replay before any changes to consensus activation rules. Bitcoin-family witness support here covers common serialization and excludes extensions like Litecoin MWEB and Dogecoin AuxPoW blocks.

The crate remains pure Rust, with no third-party dependencies and no dependencies on network services.
