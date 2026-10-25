# Stage 25F: bankruptcy threshold and channel/mark authority handoff

**Review scope:** This is a source-characterization companion to the
concurrently updated [Stage 25F draft PR #81](https://github.com/patrickdugan/TradeLayer-Rust/pull/81),
not a competing liquidation implementation. Base is
`TradeLayer-Rust@be15554453581dd44c2f314d512cb00b4a90a54f`;
legacy source is
[`tradelayer.js@dca0888b827c5bc28fa9189bc9e1d507f5c5670c`](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).
No access, live chain, paid calls, testnet, or external proving work is involved.

## Corrections established by the pinned JavaScript

The economic rules are substantially present in the legacy sources.
Their existence is different from proving they were deployed without
regressions, and different from authorizing unverified Rust transactions.

| Behavior | Pinned source | What the code establishes |
| --- | --- | --- |
| Forced risk event | `clearing.js:1850–1984` | Maintenance/loss coverage is evaluated in clearing. Positive coverage can select partial; exhausted coverage selects total |
| Price route | `clearing.js:1081–1212` | Oracle contract consumes oracle-data entry at the current block; native contract consumes volume-index price at the current block. The no-new-entry return is not a new mark |
| Funding reference | `clearing.js:580–633` | Native funding uses `VolumeIndex.getVWAP` (192-block lookback); oracle funding uses `OracleList.getTWAP`. These are **funding references**, not automatically a fresh current-block mark |
| AMM liquidation priority | `clearing.js:2343–2450,2575–2665` | Preferred AMM counterparty first, subject to linear/inverse safe-price boundary; subsequent orderbook matching remains separately invoked |
| Maker time/price | `orderbook.js:720–774,1373–1440` | Bid/ask sorting by price and `blockTime`; earlier timestamp sets maker pricing in the shown paths |
| Partial quantity | `clearing.js:2488–2529` | Residual deficit divided by contract initial margin, capped by outstanding position, rounded to eight decimals; zero result can force full liquidation |
| Bankruptcy threshold | `clearing.js:2232–2288` | Linear and inverse reciprocal bankruptcy formulas using equity, size, notional and previous mark. `BigNumber.decimalPlaces(30)` for returned prices |
| Average position price | `marginMap.js` `updatedAvgPrice` and `realizePnl` | Weighted-average accounting exists; no per-lot bookkeeping is needed in the source design. `settlePNL` references a mark and is not identical to avg-entry accounting |
| Loss waterfall | `clearing.js:2718–2810`, `marginMap.js:simpleDeleverage`, `insurance.js:190–240` | Available then margin pool seizure, remainder through ADL, pool credits capped; separate insurance-fund payment code exists. Historical side effects require differential characterization, not an invented new rule |

### Authorization chain: do not conflate three distinct proofs

1. **Commit admission / whitelist (transaction type 4).**
   `validity.js:473–653` checks available balance, activation and token
   eligibility; `propertyData.whitelistId` constrains the committer.
   Optional `commitClearlistId` checks direct/derived attestation.
   `channels.js:754–792` extracts public keys from the commitment
   transaction and recomputes the configured multisig channel address
   when both keys are known. That establishes a source-described
   *identity/address relationship*, not by itself the validity of
   an arbitrary later off-chain state signature.
2. **Channel-contract trade admission (transaction type 19).**
   `validity.js:1804–1960` checks activation, expiry, participant
   addresses, reference price and leverage-based price deviation,
   amount/margin, and any declared channel clearlist. The shown
   channel-clearlist rule accepts **at least one** listed participant,
   whereas the token commitment rule applies to the individual sender.
   These conditions are NOT interchangeable.
3. **Two-of-two signature / valid state authorization.**
   A channel's two public keys and matching script address do **not**
   themselves prove that a particular update was co-signed by both
   parties. A verified base-chain 2-of-2 spend or validated off-chain
   signed-state transcript with precise sighash/channel binding,
   amendment sequence, signer identities and non-replay rules is
   necessary to establish that. Neither `validateTradeContractChannel`
   nor this Rust audit implements an independent signature verifier.
   The actual validator/adapter and exact signed-state bytes remain a
   separately auditable prerequisite. Do not treat a CRC, a pubkey
   extracted from a commitment input, a whitelist-membership boolean,
   or a caller-declared AMM quote as a substitute.

Oracle publication has its own rule: `validity.js:1353–1400`
checks `OracleList.isAdmin(sender, oracleId)`, numeric values,
activation, and existence of the oracle record. Future Rust replay
must resolve that authority against *ordered authenticated chain
history* before feeding a current-block mark to any forced liquidation.

**Timing requirement:** validate whitelist and channel membership at
the block/commit transition when the source rule applies; a later
clearlist snapshot does not automatically prove historical eligibility.
Preserve the exact state-changing admission and identity rules, and
distinguish them from subsequent 2-of-2 spend/state authorization.

## Exact rational bankruptcy audit — no 30dp parity claim

The new `src/bankruptcy_source_audit.rs` exposes one pure
`BankruptcyFraction` in 1e-8 **price units**, never a token,
executable price tick, winner credit, or proof.

For equity `E` (collateral atoms), previous mark `L`, quantity
`|q|` and notional `N` (both prices and notional scaled by 1e8):

- Linear long: `L - E / (|q|N)`
- Linear short: `L + E / (|q|N)`
- Inverse long: `1 / (1/L + E/(|q|N))`
- Inverse short: `1 / (1/L - E/(|q|N))`

These reproduce the **symbolic equations** of
`clearing.js:2232–2288`. All numerator/denominator arithmetic is
checked `u128` and reduced by gcd. Zero equity or zero contracts
return the last mark, as in the source. Nonpositive boundaries and
nonpositive inverse reciprocals fail closed. A synthetic quote may
be compared against the fraction, but the function **does not**
apply an AMM-safe inequality or authorize a match.

Important numerical distinction: the pinned JS evaluates inverse
reciprocals with BigNumber's intermediate decimal precision and
returns a **30-decimal-place** threshold. Rust currently returns the
algebra's **exact rational value**; an actual eight-decimal quote can
straddle a fractional-satoshi threshold. Rounding of the historical
30dp representation into a canonical 1e-8 executable price tick has
**not** been established by this audit. Claim only algebraic
characterization, not byte-exact JS parity or consensus rules.

## Next executable gates (not implemented here)

The eventual trusted adapter needs separately authenticated: selected
contract kind, current-block oracle or native mark and its state root,
whitelist/commit validity at the correct height, channel script and
co-signed state/valid base-chain spend, AMM/orderbook source and maker
priority, actual collateral and position average, approved bankruptcy
and fee arithmetic, and exactly-once loss-waterfall insurance/ADL
accounting. Source-based rules can be ported with dedicated failure and
replay tests; an unverified boolean from a caller never suffices.

Stage25E2B full-book closeout and Stage25F PR81's bilateral match remain
unchanged. No state, checkpoint or API is connected to trusted replay.
This audit is purely read-only and zero third-party crates.

## Focused checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen --test bankruptcy_source_audit
python3 scripts/check_rust_supply_chain.py
```

The tests cover long/short linear and inverse thresholds, exact versus
fractional one-satoshi quote boundaries, zero-equity fallback,
nonpositive inverse price, arithmetic overflow and no-spend API scope.
