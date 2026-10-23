# Stage 23B/23C: Litecoin replay evidence, legacy JS parity, replay gate

Tracking issue: [patrickdugan/TradeLayer-Rust#28](https://github.com/patrickdugan/TradeLayer-Rust/issues/28).
Builds on [Stage 23A](LITECOIN_SOURCE_TRACE.md).

## Status of this stage

**No Litecoin chain data has been collected yet.** The branch was produced
in a cloud container: no Litecoin Core binary, data directory or RPC port
(9332, 19332, 19443) existed there, and the environment's egress policy
refused the public Litecoin explorers. `litecoin_node_inspect.py` therefore
failed with a missing cookie for both networks. No block interval was
scanned and no genuine TradeLayer candidate was observed. Zero here means
*not looked*, not *none found*.

Every count below comes from **synthetic** inputs (fake RPC, headers without
proof of work), labelled as such. Nothing in this stage asserts real-chain
parity. The node-backed steps are written so that they can be run unchanged on
the workstation with the synchronized node.

## Trust model

| Claim | Who establishes it | Strength |
|---|---|---|
| Block is on the active chain, scripts were valid | local Litecoin Core | node-attested; no Scrypt, retarget or chainwork check here |
| Header hash, parent link, Merkle root | `collect_litecoin_rpc.py`, `verify_litecoin_evidence.py`, `litecoin_evidence.rs` | recomputed from raw 80-byte headers |
| Header meets its own nBits target | `verify_litecoin_evidence.py` (Scrypt N=1024,r=1,p=1) | per-header sanity check only |
| Canonical tx position | Merkle branch + tx count (self-pairing only at the right edge) | recomputed; tx count is node-reported |
| txid, outpoints, values, scripts | raw transaction bytes (SegWit and MWEB flags) | recomputed |
| Sender/reference addresses | Rust: script bytes via Litecoin Core 0.21 `DestinationEncoder`; Python/JS: node JSON | cross-checked, never a signer proof |
| JS acceptance, state | pinned `tradelayer.js` modules, offline | observation of legacy behaviour only |

`ACCEPTED_FOR_PARITY`, `MATCH` and `DECIDED` never grant replay authority.

## Reproduction on the workstation

Paths: this repository as `.`, the pinned legacy checkout at
`../tradelayer.js` (commit `dca0888b827c5bc28fa9189bc9e1d507f5c5670c`).
`archive/` is git-ignored; keep node-backed outputs there.

### 0. Node suitability (read-only)

```sh
python3 scripts/litecoin_node_inspect.py --network test   # testnet4 data dir
python3 scripts/litecoin_node_inspect.py --network main
```

Exit 0 only when chain, pinned genesis, unpruned, not in IBD, blocks equal
headers, and txindex synced to the tip. The report records OS, the
`litecoin-cli` path (never executed), endpoint, version/subversion and
txindex status; it never reads wallets, peers or local addresses. Cookie
lookup uses `~/.litecoin/testnet4/.cookie` on testnet (Stage 23A wrongly
used `testnet3`), or `TL_RPC_USER`/`TL_RPC_PASSWORD`.

### 1. Resolve historical leads to heights

`fixtures/litecoin_historical_leads.tsv` holds 16 txids found in the pinned
JS repository (two with heights), each with `file:line` and the basis for
its claimed network. Only leads of the node's own network are queried.

```sh
python3 scripts/litecoin_resolve_leads.py --network test \
  --leads fixtures/litecoin_historical_leads.tsv --output archive/leads-test.json
python3 scripts/litecoin_resolve_leads.py --network main \
  --leads fixtures/litecoin_historical_leads.tsv --output archive/leads-main.json
```

`NOT_FOUND_ON_THIS_CHAIN` is a valid negative. `suggested_windows` gives
<=500-block intervals starting at each resolved lead. The published
3,348,377-3,348,528 testnet window remains a separate, unverified lead.

### 2. Collect and source-trace

```sh
python3 scripts/litecoin_source_trace.py --network test \
  --start-height 3348377 --end-height 3348528 --min-confirmations 6 \
  --output archive/ltc-testnet-candidates.tsv \
  --manifest archive/ltc-testnet-source-trace.json
```

Selection is the union of Rust's strict canonical OP_RETURN rule and the
pinned JS predicate (first `nulldata` output, `746c` at nibble 0..6); each
candidate carries `carrier_class`. Manifests now hold the raw headers of
every scanned block, a Merkle branch and tx count per candidate, and the
sender parent's raw bytes, header and branch.

A **complete prior state** needs hash-linked coverage that contains the
pinned legacy genesis (`GENESIS_BLOCK_HEIGHTS.LTC`: testnet 3,082,500,
mainnet 2,819,000). For a testnet lead near 3,432,676 that is about 700
consecutive runs:

```sh
for s in $(seq 3082500 500 3432999); do
  python3 scripts/litecoin_source_trace.py --network test \
    --start-height "$s" --end-height "$((s + 499))" \
    --output "archive/t-$s.tsv" --manifest "archive/t-$s.json" || break
done
```

### 3. Independent verification

```sh
python3 scripts/verify_litecoin_evidence.py \
  --pair archive/ltc-testnet-candidates.tsv archive/ltc-testnet-source-trace.json \
  --recheck-rpc --output archive/verification.json
```

Repeat `--pair` in ascending height order for multi-run coverage. Each
candidate becomes `ACCEPTED_FOR_PARITY`, `INCONCLUSIVE` (unresolved sender or
reference address: `modern-only`, `schema-conflict`, `ambiguous`,
`unresolved`) or `REJECTED` (any byte, header, Merkle, parent, nulldata
typing or reorg failure). `coverage.prior_state` is
`COMPLETE_FROM_LEGACY_JS_GENESIS` or `INCOMPLETE`.

### 4. Rust syntax tools

```sh
cargo run --locked --bin tradelayer-scan -- archive/ltc-testnet-candidates.tsv
cargo run --locked --bin tradelayer-legacy-fields -- \
  archive/ltc-testnet-candidates.tsv > archive/rust-fields.tsv
node scripts/compare_legacy_js.cjs archive/rust-fields.tsv \
  ../tradelayer.js/src/txDecoder.js > archive/differences.json
```

### 5. Legacy JS replay (networkless)

```sh
(cd ../tradelayer.js && npm ci --ignore-scripts)   # only networked step
unshare -rn node scripts/legacy_js_replay.cjs --js ../tradelayer.js \
  --manifest archive/ltc-testnet-source-trace.json \
  --corpus archive/ltc-testnet-candidates.tsv \
  --verification archive/verification.json \
  --rust-fields archive/rust-fields.tsv --output archive/js-replay-1.json
python3 scripts/litecoin_record_rpc.py --network test \
  --replay archive/js-replay-1.json --output archive/rpc-recordings.json
unshare -rn node scripts/legacy_js_replay.cjs ... \
  --rpc-recordings archive/rpc-recordings.json --output archive/js-replay-2.json
unshare -rn node scripts/legacy_js_replay.cjs ... \
  --rpc-recordings archive/rpc-recordings.json \
  --ordering canonical-counterfactual --output archive/js-replay-cf.json
```

The harness refuses to start with any non-loopback interface, a checkout
other than the pinned commit, a dirty worktree, or a `src/main.js` whose
SHA-256 differs from `006d59ba...4009`. It records the lockfile digests and
Node version. Reports are kept separate: `syntax`, `actor_reference`,
`decisions`, plus per-block state digests, `intra_block_order_divergence`
and `rpc_requests`. The recorder answers only `validateaddress` and keeps
only address-derived fields.

### 6. Rust authentication cross-check

```sh
python3 scripts/export_ltc_evidence.py --manifest archive/ltc-testnet-source-trace.json \
  --corpus archive/ltc-testnet-candidates.tsv --output archive/export.tsv
cargo run --locked --bin tradelayer-ltc-authenticate -- archive/export.tsv > archive/rust-auth.tsv
python3 scripts/export_ltc_evidence.py --compare archive/rust-auth.tsv \
  --manifest archive/ltc-testnet-source-trace.json
```

Rust re-derives sender and reference addresses from script bytes and
compares them with the node's `addresses[0]`.

## Legacy behaviour found in the pinned code

All at `tradelayer.js@dca0888b`. Each is recorded, not normalized.

1. **Carrier search is permissive.** `src/txIndex.js:241-292` examines only
   the first `nulldata` output and accepts `746c` at hex offsets 0..6
   (`:257`), including odd nibble offsets, with no push-length check and a
   printable-ASCII filter. PUSHDATA2 carriers (marker at offset 8) are
   rejected, though Rust's strict reader accepts them.
2. **Indexer passes the wrong arguments.** `processTransaction` calls
   `Types.decodePayload(txId, marker, payload)` (`src/txIndex.js:312`)
   against the signature `(txId, type, marker, encodedPayload, ...)`
   (`src/types.js:149`), so its stored `decodedParams` is always
   `Error('Invalid payload')`; validation happens later in `main.js`.
3. **Intra-block order is txid order.** `constructConsensusFromIndex`
   (`src/main.js:277`) groups NeDB `findAsync({})` results, which NeDB
   1.8.0 returns in `_id` order (`node_modules/nedb/lib/datastore.js:96`);
   `_id` is `tx-<height>-<txid>`. Funding types 4/20 go first, then txid hex
   order, not block position. In the synthetic activate/issue/send block the
   send is processed first and rejected (`propertyId not found`), while
   block-position order accepts it and leaves different balances.
4. **Catch-up and real-time modes differ.** Only real-time `processTx`
   rejects `blockHeight < activationBlock` (`src/main.js:530`); catch-up
   `processTxSet` (`:404`) does not. The oracle reports
   `realtime_mode_differs` per transaction.
5. **Activation applies within its own block.** In the synthetic replay an
   issue in the activation block was valid; Stage 22 Rust rejects it
   (`JointError::ActivatedInSameBlock`).
6. **`referenceUTXO` is always NaN.** `valueData.reference.amount / COIN`
   (`src/main.js:428`) reads `.amount` of an array.
7. **Sender errors are returned, not thrown.** `getSender`
   (`src/txUtils.js:170`) returns an `Error` object when `addresses` is
   absent (P2PK, or RPC schemas without `addresses`); after NeDB storage it
   becomes `{}` and `senderAddress` is `undefined`. Litecoin Core 0.21.5
   (`core_write.cpp` at ec1b6489) emits `addresses[]` only.
8. **Testnet bech32 addresses always fall back to RPC.** `validateAddress`
   (`src/validity.js:3748`) iterates the top-level `networks` (`:3667`);
   testnet parameters are nested under `networks.testnet` (`:3682`), so
   `tltc1...` never matches and `validateSend` calls node `validateaddress`
   (`:247`). Testnet send validity is node-dependent.
9. **`TxUtils.init` is undeclared in callers.** Methods call a bare
   `init()` when `this.client` is unset (`src/txUtils.js:18` and others),
   which throws `ReferenceError`.
10. **Encoder drops the decimal flag.** `?? '0' + '~'` binds `'~'` only to
    the fallback (`src/txEncoder.js:21`, `:65`), so fractional amounts are
    encoded as scaled atoms without `~` and decode 1e8 times larger.
11. **`isActivated` always returns true** (`src/validity.js:83-93`).
12. **`client.getRawTransaction` ignores `blockHash`** (`src/client.js:171`);
    legacy indexing always required txindex.

## Litecoin-specific parsing added in this stage

Litecoin Core (`src/primitives/transaction.h` at ec1b6489) adds
serialization flag `0x08`: an MWEB blob before `nLockTime`, a single `0x00`
for the block's HogEx. The txid covers canonical fields only; the wtxid
excludes MWEB. Python and Rust parsers now handle both flags, keep MWEB
payloads opaque and refuse MWEB-only transactions. Before this, any HogEx
parent (for example a peg-out) failed the whole scan.

## Diagnostic counts (SYNTHETIC only)

From `tests/synthetic_ltc_chain.py` through the real collector, verifier,
Rust tools and pinned JS (`TL_LEGACY_JS=../tradelayer.js node
tests/test_legacy_js_replay.cjs`):

| Report | Result |
|---|---|
| evidence | 3 `ACCEPTED_FOR_PARITY`, prior state complete (synthetic block at 3,082,500) |
| syntax | 3 `MATCH` |
| actor/reference | 3 `MATCH`; Rust script-derived addresses 3/3 equal to node JSON |
| decisions, pass 1 | 2 `DECIDED`, 1 `INCONCLUSIVE` (unrecorded `validateaddress`) |
| decisions, pass 2 | 3 `DECIDED`: activate valid, issue valid, send **invalid** |
| canonical counterfactual | send valid; state digest differs |
| intra-block order divergence | 1 block |

Real-chain counts: **none**, no evidence collected.

## Unresolved historical assumptions

1. Which Litecoin network carried the historical TradeLayer activity; the
   3,348,377-3,348,528 testnet window and all 16 repository txids are leads.
2. `GENESIS_BLOCK_HEIGHTS.LTC` (3,082,500 / 2,819,000) is commented
   "Replace with actual" in `src/main.js:70`; the `Main` constructor also
   sets `genesisBlock = 600000`, and `const genesisBlock = 3082500` exists.
3. Which Litecoin Core version and RPC schema the legacy indexer ran
   against (`addresses[]` versus `address`); the tests use port 18332,
   Bitcoin's testnet default, on a Litecoin client.
4. Whether historical nodes answered `validateaddress` identically to the
   node used for recording.
5. Whether the legacy operator ran catch-up or real-time mode for a given
   height (finding 4), and whether any operator restarted mid-block
   (`checkIfTxProcessed` skips).
6. Whether NeDB `_id` order is the intended consensus order (finding 3).
7. Whether same-block activation (finding 5) is intended semantics.
8. The legacy behaviour of post-block hooks (clearing, vesting, withdrawals)
   on heights divisible by 10,000 with no TradeLayer transactions.
9. Reorg handling: the pinned `reOrg.js` and `persistence.js` were not
   replayed; the oracle has no reorg model.
10. Whether historical payloads contain decimal amounts produced by the
    encoder bug (finding 10).

## Stage 23C authorization gate

`tradelayer_core::litecoin_evidence::stage23c_gate` returns the blockers.
It can only add blockers; summaries it reads are caller-supplied. In this
stage two blockers are unconditional, and no API converts an
`AuthenticatedObservation` into a `UnverifiedJointTx`:

| Blocker | Clears when |
|---|---|
| `NoTradeLayerInstance` | evidence is Litecoin **mainnet** (testnet has no TradeLayer/JointChainState) |
| `NoParityReport`, `ParityReportUnbound`, `EmptyParityCorpus` | a report bound to the authenticated corpus SHA-256 with decided transactions |
| `PriorStateIncomplete` | hash-linked coverage contains the pinned legacy genesis |
| `EvidenceNotAccepted` | every candidate `ACCEPTED_FOR_PARITY` |
| `SyntaxParityNotExact`, `ActorReferenceParityNotExact` | zero non-`MATCH` records, including Rust-vs-node address comparison |
| `DecisionsNotAllDecided` | zero `INCONCLUSIVE`/`UNSUPPORTED_BY_ORACLE` decisions |
| `LegacyIntraBlockOrderDiverges` | an explicitly ratified ordering rule, or no divergent blocks |
| `ProhibitedType(3/23/31)` | never in this stage |
| `TypeOutsideOracle(t)` | the oracle covers type `t` |
| `ActivationSameBlockSemanticsDiffer` | **unconditional**: ratify one rule and align Rust or the record |
| `RustDecisionReplayMissing` | **unconditional**: a Rust replay from authenticated observations into a separate Litecoin `JointChainState`, compared transaction by transaction and block by block (state digests, rollback on late failure, reorgs) |

An empty blocker set would mean *eligible for human review*, never
automatic authority. Type 3 UTXO exchange and types 23/31 settlement stay
default-deny; existing channel obligations still block migration; no
cross-chain balances, no Scrypt/Cairo claims.

## Tests

```sh
cargo test --locked --test litecoin_evidence
python3 -m unittest discover -s tests -p 'test_*litecoin*.py' -v
node tests/test_legacy_js_replay.cjs
TL_LEGACY_JS=../tradelayer.js node tests/test_legacy_js_replay.cjs   # opt-in, needs unshare
```

## Privacy

Manifests contain public chain data, but they link addresses to TradeLayer
activity and to the node operator's choice of windows. Keep `archive/`
local. Before committing any excerpt, review addresses, payload text and
node subversion strings; never commit cookies, RPC credentials or anything
from wallet RPCs (none are called).
