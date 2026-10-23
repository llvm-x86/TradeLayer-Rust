# Stage 24C: Clearing call-cadence and funding bucket audit

**Status: offline, diagnostic only.** The pinned source is
[`tradelayer.js@dca0888b`](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c),
including `src/main.js` and `src/clearing.js`. The goal is to determine
*when legacy code attempted clearing*, separately from any economic balance
effect. No Litecoin node or chain history is required for the synthetic
diagnostic vectors.

The existing Rust Stage 24B
[`funding_audit`](../src/funding_audit.rs) describes *attempted account
deltas when the funding method is invoked*. Stage 24C describes whether the
funding method would have been invoked at a given height in the first place.

## Pinned source trace

| Source | Behavior |
| --- | --- |
| `src/main.js:277–380` `constructConsensusFromIndex` | After the last indexed height has been found, scans each height through that last indexed transaction; classifies payload type 4 and 20 into a funding bucket; processing previously unseen transactions makes `skip=false` even if a transaction later rejects |
| `src/main.js:380–410` | On `height % 10000 == 0`, catch-up forces the postblock branch even when that block contains no indexed TL transactions, provided the catch-up range actually reaches the height |
| `src/main.js:829–880` `blockHandlerBegin` | Real-time separates payload strings starting with **`4`** or **`m`** for early processing; in legacy base36 `m=22`, `k=20` |
| `src/main.js:878–910` `blockHandlerMid` | Calls `processTx` on the remaining bucket; clearing runs only if some unprocessed transaction was valid **before** `Logic.typeSwitch` |
| `src/main.js:530–650` `processTx` | Sets `processedAny` before calling `Logic.typeSwitch`; a later logic failure still leaves the flag set |
| `src/clearing.js:400–430` | `clearingFunction` invokes `applyFundingRates`, then makes settlement and performs selected invariants |
| `src/clearing.js:553–575` | `applyFundingRates` returns immediately unless `blockHeight % 24 == 0` |

The `m` detail affects the **routing bucket**, not the meaning of any
type-20 or type-22 transaction. Types 20 and 22 continue to be decoded from
their own payload type character.

## Minimal regression examples

At a height divisible by 24, and assuming the catch-up loop has reached it:

| Block observation | Catch-up funding attempt | Real-time funding attempt |
| --- | --- | --- |
| No TL transactions | No, except forced 10,000-height callback | No |
| One new type 4 commit | Yes | No: handled by early bucket |
| One new type 20 channel trade | Yes | Yes if valid: `k` stays in ordinary bucket |
| One new type 22 transfer | Yes | No: `m` is put into early bucket |
| One new invalid type 18 contract trade | Yes | No |
| One newly valid type 18 | Yes | Yes |
| All transactions previously processed | No, except forced height | No |
| No TL transactions at height 30,000 | Yes (forced callback, 30,000 divisible by 24) | No |

The 10,000-height catch-up rule does not imply a full empty-chain scan.
`constructConsensusFromIndex` may return before it reaches any height if the
index is empty or if the last indexed height falls short of that point.

The model records separate `clearing_called`, `funding_height_eligible`,
`funding_attempted`, and `withdrawals_attempted` predicates. The latter
differs because catch-up invokes `Channels.processWithdrawals` in the
postblock branch, while real-time calls it each processed height after
`blockHandlerMid`.

## Synthetic reproduction

```sh
node scripts/audit_clearing_cadence.cjs --check
cargo test --locked --test clearing_cadence
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

The independent Node script generates the 32-entry corpus in
`fixtures/clearing_cadence_vectors.tsv`; Rust checks the corresponding
source-characterization predicates, the type 20/22 base36 routing mismatch,
funding-only blocks, invalid transactions, skipped heights and bounds.

These are source-derived synthetic predictions. An unmodified, pinned
JavaScript integration replay and genuine historical chain evidence would
be needed to establish which scheduler path a particular deployment ran,
whether a previous state had been loaded, whether the database processed flag
was already set, or whether funding/margin settlement succeeded. A
`funding_attempted` result is **not** a claim that balances were modified.

## Review gates for any future change

- Confirm the intended ordering of type-4/20/22 input processing and whether
  the legacy catch-up and live paths ever produced divergent ledger states.
- Characterize empty-block periodic hooks, replay after crash, reorgs and
  repeated clearing invocations. Include blocks around the 24-block funding
  period and 10,000-height catch-up forcing boundary.
- For economic authorization, require an explicit protocol decision about
  scheduling plus a recognized activation boundary. Preserve historical
  behavior during any legacy-state replay.
- Keep the Stage 24B funding sign/rounding characterization and Stage 24C
  scheduler independent. Neither gives any permission for type 3 UTXO
  settlement, type 23/31 economic settlement or channel migrations.

See [the Stage 24B diagnostic](FUNDING_AUDIT.md) for why an attempted
funding call can reverse the intended payment direction and fail
atom-level conservation.
