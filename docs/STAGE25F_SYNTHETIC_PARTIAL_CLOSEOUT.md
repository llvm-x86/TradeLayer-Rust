# Stage 25F: synthetic bilateral partial closeout and pinned JS source audit

**Status: research only, draft, unmerged.** No authenticated liquidation, settlement permission, live liquidity, or testnet/node operations. The preceding Stage 25E2B **whole-book** closeout remains separate and unchanged.

## Provenance

- Rust baseline: [TradeLayer-Rust `be155544`](https://github.com/patrickdugan/TradeLayer-Rust/commit/be15554453581dd44c2f314d512cb00b4a90a54f), merged PR #79 source `db28db75e22d77700552f49261c94fc114bb7be6`.
- Historical reference: [tradelayer.js `dca0888b`](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).
- Scope: [Issue #80](https://github.com/patrickdugan/TradeLayer-Rust/issues/80). No second Stage 25F implementation branch/PR was active when this branch was opened; formal/Kani and Stage 23 work are untouched.

The November–January JS clearing rewrites matter: we must characterize the **actual reachable code paths**, including mistakes and competing implementations, rather than canonize the source or invent new economics.

## Existing historical mechanics: recovered, not missing

| Pinned source | Source-level behavior | Work still required before trusted Rust parity |
| --- | --- | --- |
| `src/clearing.js:1081–1214` | Oracle contract mark: oracleData event at the current block. Native contract mark: volumeIndex price for the pair at the current block. A cached prior mark distinguishes updates. | Independently authenticated, height-bound source data and reproducible history |
| `src/clearing.js:580–633`, `src/volumeIndex.js:522–552`, `src/oracle.js:494–524` | **Separate funding references:** native uses 192-block volume-weighted VWAP; oracle uses 192-block equal-weight TWAP | Do not incorrectly substitute either rolling funding reference for the block mark |
| `src/clearing.js:643–658,770–829` | Two class definitions of `getIndexPrice` exist; the **later** method takes precedence in JS. | Characterize the actually invoked method, not the earlier overwritten definition |
| `src/clearing.js:1937–1984,2488–2529` | Maintenance/loss-coverage classifications distinguish partial and full liquidation. A partial quantity is estimated from deficit ÷ initial margin and rounded to eight decimals. | State which historical rounding and risk branch applies to the actual contract at the actual height |
| `src/clearing.js:2232–2288`, `src/marginMap.js:559–623` | Bankruptcy and liquidation prices are explicitly calculated in **two** source paths; the first uses 30 decimal places, the second four decimal places and a 0.5% adjustment. | Identify the invoked path, not a newly invented bankruptcy boundary |
| `src/clearing.js:2343–2449` | **AMM is preferred first**, but only at the appropriate safe liquidation boundary. Bid levels descend, ask levels ascend, earlier block time breaks price ties. Linear and inverse inequalities differ. | Authenticate AMM inventory, matched quantity and boundary evidence |
| `src/orderbook.js:720–774,1373–1449` | Regular orderbook matching sorts bids high/asks low then by earliest block time; maker price generally follows the earlier order, with market-flag handling. | Obtain verified order/nonce and fee context rather than fabricate maker priority |
| `src/clearing.js:2575–2678` | Liquidation actually attempts AMM sweep, then regular matching and caps the safe filled size. | Do not equate a caller-declared price with executed fills |
| `src/clearing.js:2688–2828`, `src/marginMap.js:1550–1694` | Remaining size goes through concrete loss-pool, `simpleDeleverage`, counterparty sorting and ADL accounting. | Differential tests of this exact asynchronous path, including insolvency and recoverability |
| `src/insurance.js:190–265` | Insurance payout caps available funds, records a payout, and rounds to eight decimals; old balances are in a separate database. | Read those rules as source evidence; insurer withdrawals must still come from one actual Rust Ledger |
| `src/clearing.js:20–27`, `src/orderbook.js:700` | `toSatsInt` scales by 1e8 and uses `ROUND_HALF_UP`. Other source formulas use `decimalPlaces(8)`. | Independently verify every intermediate and signed tie case, not merely final sat conversion |

### Weighted average entry exists; no per-lot cost-basis requirement

The historical engine stores `marginMap.position.avgPrice`. `marginMap.js:420–466,628–648` updates a weighted average when **increasing** same-direction exposure and preserves the average when **reducing** it. `orderbook.js:2516–2520,2716–2739` explicitly captures that average **before** an update might clear the position.

There are two separate PnL quantities:

- `marginMap.realizePnl` (`1028–1072`): **accounting realized PnL**, comparing **average entry** to execution.
- `marginMap.settlePNL` (`1207–1262`): **incremental settlement PnL**, comparing **previous mark** to execution (falling back to avg entry). Its **linear** branch omits the notional multiplier that the `realizePnl` branch uses. The two calculations cannot simply be substituted for each other.

The synthetic Rust Stage25F operation below currently uses **incremental reference-mark arithmetic only**, and the receipt field is named `mark_reference_transfer_atoms` accordingly. It makes **no** historical accounting-realized parity claim. Porting weighted average to persistent Rust position state and TLCL/TLCC requires a separately reviewed versioned state change, not a new per-lot bookkeeping scheme.

### Commit-whitelist and 2-of-2 channel provenance

The source contains actual authorization-related checks, not a blank authorization policy:

- `validity.js:490–648`: commitment validation checks sender balance, activation, token clearlist membership and channel multisig-address reconstruction when pubkeys are known.
- `channels.js:754–790`: extracts pubkeys from commit-transaction inputs and tests whether the derived 2-of-2 multisig address matches the channel.
- `validity.js:1805–1967`: validates a contract-channel trade's price/reference, activation, expiry, commit addresses and channel clearlists (the latter path requires at least one of the two counterparties attested when declared).
- `validity.js:2395–2423`: another channel-token-trade path checks both commit addresses against applicable property whitelist sets.

The on-chain 2-of-2 **co-signature/witness**, actual funding output and UTXO spent/invalidated at the relevant height still must be **verified by the underlying chain adapter**, not represented by a caller-supplied `bool`. Those source-level whitelist conditions are not all interchangeable. Stage25F does not add a fake co-signature proof or enable forced liquidations based on claimed membership.

## Admitted synthetic operation: exactly one existing pair

```text
SyntheticBilateralReduction {
    long: existing AccountId with a positive position,
    short: a distinct existing AccountId with a negative position,
    contracts: strictly positive integer, strictly less than BOTH positions,
    execution_price_e8: caller-declared positive price
}
```

`reduce_bilateral_positions_unverified(external_id, height, &fill)` is **one explicitly synthetic matched pair**, **not a forced liquidator call**. It never fabricates counterparties or creates/changes a maker order. Both parties reduce exactly equal-and-opposite amounts. Neither position can become flat or flip; opening, full closure and partial-fill remainders are outside this narrow experiment.

The frozen `MarkPolicy` supplies notional, linear/inverse and selected `Legacy20DecimalPlaces` or `ExactRational` rounding. The synthetic transfer is calculated as:

```text
long mark-reference delta  = Stage24A_PnL(+q, last_mark, declared_fill, frozen terms)
short mark-reference delta = Stage24A_PnL(-q, last_mark, declared_fill, frozen terms)
```

The pair's exact rounded sum must be **zero** or the whole batch fails. The loser pays **only from their actually funded** `~clrm:` margin escrow into the winner's `~clrm:` escrow in the existing single SpotExchange-owned Ledger. No ordinary available balance, insurer `~ins:` reserve, unfilled orderbook, fake AMM depth or IOU backs a winner. Any insufficient balance or post-transfer maintenance breach rejects with all positions, balances, queues and replay markers unchanged.

The entire operation uses a staged clone and one atomic commit after invariant verification. Before/after transfer failure injection and repeat-claim tests prove rollback at every admitted boundary. The event is a separate `UnverifiedClearingEvent::SyntheticBilateralReduction` inside **synthetic TLCC only**; neither the E2B full-book transition nor trusted JointChainState replay is modified. Types 3, 23 and 31 remain denied in trusted replay. TLCL/TLCC v2 persistence/rollback captures sole Ledger balances and reduced signed lots without changing snapshot format.

An execution height must be later than the current reference mark and previously finalized spot height. A due queued spot order rejects rather than allowing a guessed same-block priority. This is a conservative synthetic sequencing restriction, not a statement of the historical maker ordering. **The global mark itself does not advance** in this partial reduction.

## Read-only pinned-JS characterization module

`liquidation_source_characterization.rs` is explicitly independent of all settlement and state APIs. It includes:

- the exact **four-way linear/inverse safe-price inequality matrix**, inclusive at the bankruptcy boundary;
- an **immutable**, bounded **AMM-first** synthetic depth estimate (highest bid/lowest ask; earliest block time; stable equal-time input order). Ordinary levels remain untouched and no fill is authorized;
- price/time **sort-only** characterization of ordinary orders, without maker rights or fee settlement;
- the separate **oracle event/native block volume-price mark** versus oracle TWAP/native VWAP **funding** routes (192-block reference);
- eight-decimal `ROUND_HALF_UP` weighted average entry for *increasing* exposure; reduction leaves the prior average unchanged;
- signed average-entry accounting PnL versus previous-mark incremental PnL, including the pinned JS **linear settlePNL notional omission** and sat-tie tests.

These are **source-specific offline diagnostics**, not a newly chosen clearing policy. They are not called from the synthetic bilateral transition.

## Still blocked for economic authorization

1. The actual signed and whitelist-valid **channel trade/commit**, underlying 2-of-2 UTXO witness and time-bounded chain provenance.
2. Authenticated oracle event or native volume-index **mark**, distinct from the correct funding TWAP/VWAP history.
3. Real AMM reserve/offer source, a safe bankruptcy boundary from the applicable source method, and actual maker/orderbook matched trades and fees.
4. Decimal partial-liquidation quantity and rounding/source-branch parity; the synthetic slice remains whole-integer and strictly partial.
5. Persistent weighted average/realized-versus-incremental accounting and an explicit protocol choice where the pinned JS branches disagree.
6. Authorized seizure, insolvency/insurance/ADL and any systemic loss rights. JS contains these methods, but no verified chain-bound liquidation observation is connected to Rust yet.

These are **evidence/integration and source-divergence gates**, not claims that the JS lacks these mechanisms. A checksummed checkpoint or synthetically conserved Ledger does not establish permission to force-close a market position.

## Focused offline validation

```sh
python3 scripts/check_rust_supply_chain.py
rustfmt --check --edition 2021 src/clearing_partial_impl.rs
cargo test --frozen --test clearing_partial_closeout
cargo test --frozen --test liquidation_source_characterization
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
```

The tests cover existing-pair lot reductions and payer conservation; unchanged unrelated margin owners and resting spot orders; insufficient pledged margin with insurer untouched; full-close/flip/missing owner/competing claims rejection; fault injection before and after the sole transfer; TLCL/TLCC recovery and reorg. Source-only tests independently cover AMM-first and maker price/time, four safe-price directions, avg-entry and incremental-mark distinctions, plus sat-level half-up rounding.

The supply-chain policy remains **one Rust crate with zero third-party dependencies**, offline and frozen. No changes to formal/Kani work, historical JS repo, node/testnet, wallets or funds.

**PR remains draft and unmerged.**
