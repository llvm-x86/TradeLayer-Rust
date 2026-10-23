# Stage 23A: Litecoin Core historical source trace

**Research-only, node-backed source attribution.** The Stage 22 joint ledger is
still fed by `UnverifiedJointTx` and cannot safely consume these observations.
The scope of this PR is a reproducible, read-only source/reference trace that
extends the existing Litecoin archival collector. It deliberately makes
`joint_replay_authorized: false` part of its manifest.

The underlying consensus check is a **locally trusted, fully synchronized,
unpruned Litecoin Core node** with `txindex=1`. This program does not prove
Scrypt PoW independently. No Raito/Cairo path is available for Litecoin;
Dogecoin AuxPoW and Bitcoin remain independently scoped projects.

## Pinned JavaScript interpretation

Pinned TradeLayer JavaScript reference:
[`patrickdugan/tradelayer.js@dca0888b`](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).

Its `src/txUtils.js::getSender` selects **input 0**, follows the previous
transaction's `vout`, then uses the previous output's
`scriptPubKey.addresses[0]`. That is an output-script account convention,
not a demonstration of who signed all inputs. Different input scripts, scripts
without a single recognized address, and newer Litecoin Core responses that
only provide singular `scriptPubKey.address` require explicit treatment.

The legacy code has **two different reference rules**. In
`txIndex.processTransaction`, `getTransactionOutputs` selects spendable
outputs and then retains only `vout == 0 || vout == 1`. In
`txUtils.getReference`, it chooses the **last non-nulldata output**. Both
rules are recorded in the evidence even when their answers differ. Neither is
automatically promoted into a token recipient.

`txIndex.DecodeRawTransaction` also recognizes some noncanonical OP_RETURN
layouts by searching for `746c` near the beginning of a decoded script;
the new Rust reader is deliberately stricter. A full JS acceptance-parity
implementation must include these divergent carrier cases. The source trace
covers the existing collector's canonical candidate subset.

## Collect node-backed evidence

For the published **testnet search lead** (historical inclusion still needs
checking on the matching Litecoin testnet branch):

```sh
# Litecoin Core: unpruned, synced, txindex=1 and local RPC.
python3 scripts/litecoin_source_trace.py \
  --network test --start-height 3348377 --end-height 3348528 \
  --min-confirmations 6 \
  --output archive/ltc-testnet-candidates.tsv \
  --manifest archive/ltc-testnet-source-trace.json

cargo run --locked --bin tradelayer-scan -- archive/ltc-testnet-candidates.tsv
cargo run --locked --bin tradelayer-legacy-fields -- \
  archive/ltc-testnet-candidates.tsv > archive/rust-fields.tsv

# Run in an isolated networkless environment with a trusted pinned JS checkout.
node scripts/compare_legacy_js.cjs archive/rust-fields.tsv \
  ../tradelayer.js/src/txDecoder.js > archive/legacy-diff.json
```

For Litecoin mainnet choose `--network main` and an independently selected
mainnet block window. The script inherits all collector security constraints:
500 blocks per run, 10,000 candidates, loopback-only RPC with cookie or
environment credentials, no private-key or wallet calls, and a confirmation
and active-chain check. Both output paths must differ.

The source-trace extension additionally:

1. Parses the selected transaction's raw first-input outpoint, and checks the
   node's decoded input list against each raw outpoint.
2. Obtains the previous transaction from the local full node, verifies its
   raw txid, claimed containing block, complete transaction Merkle root,
   on-chain position and exact spent output bytes and satoshi value.
3. Checks the script's `addresses[]` and `address` schemas separately.
   `legacy-exact`, `modern-only`, `schema-conflict`, `ambiguous`, and
   `unresolved` are distinct statuses, preserving gaps.
4. Preserves all outpoint references, the selected sender script, and both
   legacy reference-output conventions. Rechecks active-chain membership after
   source-resolution RPCs, including blocks with parent transactions.
5. Binds the trace to the corpus by SHA-256 and records the exact legacy
   JavaScript commit. Credentials are excluded from all evidence.

**Trust and limits:** The trusted full node enforces Litecoin consensus,
including spending scripts under its activated rules. The tool checks that
the output scripts correspond to the exact previous transactions reported by
that node, and retains the addresses only as script descriptors. Those
addresses do not identify a unique signer in general. The two-file publication
is not an atomic pair and the corpus digest is not an authentication signature.
Recheck current canonical status before using historical results.

For offline regression testing:

```sh
python3 -m unittest discover -s tests -p test_litecoin_source_trace.py -v
python3 -m unittest discover -s tests -p test_collect_litecoin_rpc.py -v
```

## Next gate: replay authorization

Complete a **typed JS validity trace** against the same historical network
window, including activation order, property assignment, amounts, reference
selection, outpoint/signature-derived identity, and handling of every
noncanonical legacy carrier. Record accepted/rejected decisions and rejection
reasons from the pinned JS implementation. Build a bounded, independently
cross-checked Rust `TrustedChainAdapter` that authenticates all mandatory
inputs before invoking a **separate Litecoin instance** of
`JointChainState`. Any ambiguity or missing historical state must block that
block's mutation. Type-3 UTXO settlement and type-23/31 economic settlement
remain denied until their distinct authorization rules are implemented.

No live archival Litecoin Core node is available in CI, so the committed
tests are deliberately synthetic RPC fixtures. Neither fixture exercises a
real Litecoin chain or demonstrates historical TradeLayer acceptance.
