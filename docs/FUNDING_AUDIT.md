# Stage 24B: Funding direction and conservation audit

**Research status:** independent, offline accounting projection. This module is
a characterization of attempted legacy balance updates and a proposed
conservation rule. It does **not** execute settlement, mutate TradeLayer
balances, or authenticate signed transaction history.

Reference: pinned
[`tradelayer.js@dca0888b`](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).

## The signed-payment defect

In `src/clearing.js:670–724`, `applyFundingToPositions` accumulates:
```js
fundingAmount = abs(contracts) * notional * (fundingRate / 10000)
shortFunding += fundingAmount   // in the fundingRate < 0 branch
processFundingPayments(shorts, longs, shortFunding, ...)
```

Since `fundingRate < 0`, `shortFunding` is negative. In
`src/clearing.js:727–768`, `processFundingPayments` uses:
```js
amountOwed = totalFunding * payerWeight / totalPayerContracts
Tally.updateBalance(payer, collateral, -amountOwed, ...)
amountReceived = totalFunding * receiverWeight / totalReceiverContracts
Tally.updateBalance(receiver, collateral, amountReceived, ...)
```

The algebra therefore **credits supposed short-side payers** and **debits
long-side receivers** for negative funding. The upstream caller comments say
the opposite. `tally.js:117–225` checks resulting nonnegative available
balances, which means a real invocation may fail partway through its
asynchronous account updates. This audit reports the *attempted* deltas,
rather than claiming those updates committed.

The single-position synthetic check (10 bps, one unit notional) produces:

| Actor | Legacy attempted delta | Proposed direction |
| --- | ---: | ---: |
| Long | −100,000 atoms | +100,000 atoms |
| Short | +100,000 atoms | −100,000 atoms |

No live-chain evidence establishes the reachability or deployment impact
of this branch. The pinned `tests/clearingFundingPrecision.test.js` covers
funding-rate clamp and maximum rate only; live trade/oracle spam exercises
different surfaces and carries no assertions for funding transfer direction.

## Separate per-account rounding can leak supply

`processFundingPayments` rounds each payer share and each receiver share
separately to eight decimals using BigNumber's default HALF_UP rounding.
Even when the aggregate long and short contract weights are equal,
their independently rounded totals can disagree.

At +10 bps with notional 510 atoms per contract and payer weights
1, 1 and 2 against a receiver weight 4, the source first rounds
per-position charges to **1 + 1 + 1 = 3 atoms**. It then redistributes
the three-atom amount by the payer weights, rounding separately to
**1 + 1 + 2 = 4 atoms**, while the receiver obtains 3 atoms. The projected
balance sum changes by **−1 atom**. Reversing position direction and rate
produces a **+1-atom** legacy residual. With payer weight 2 and receiver
weights 1 and 1, an otherwise balanced OI set can leave a +1-atom residual.

Absent receivers are another important test: the legacy code iterates
payer-side updates, then returns when receiver contract weight is zero.
The projection records the resulting incomplete transfer.

## Two explicitly separate calculations

`funding_audit::audit_funding(..., FundingMode::LegacySigned)`

- Uses signed basis points scaled by 10,000, integer contract counts,
  exact 1e-8 notional atoms and first-stage per-position HALF_UP rounding
- Models independent weighted splits of the original *signed* total
- Returns projected signed account deltas, net atom residual and nominal size
- Does not infer which historical `Tally.updateBalance` calls succeeded

`funding_audit::audit_funding(..., FundingMode::ConservingProposal)`

- Makes the payer debit negative for either funding direction
- Preserves the per-payer, separately rounded amount; sums exact charges
- Allocates the received total by largest remainder (Hamilton); ties use
  lexicographically ascending ASCII account names
- Requires a nonempty receiving side and enough available atoms in each
  paying account
- Computes an immutable balance preview only after all checks; net deltas
  and previewed total balances must be exactly conserved

Both modes reject duplicate account IDs, empty identities, excessive
positions and unsafe arithmetic. They do not model fractional contracts,
arbitrary non-atomic notional, historical BigNumber configuration changes,
unsafe JS Number conversion, collateral margin/insurance sourcing or
bankruptcy waterfalls. The synthetic fixtures are constructed so the
intermediate 20-decimal BigNumber ratio does not affect the final atom.

## Empirical reproduction

Run the dependency-free, synthetic reference:

```sh
node scripts/audit_funding_vectors.cjs --check
cargo test --locked --test funding_audit
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

For **optional direct execution of pinned original `clearing.js`** when
the legacy repository is present with dependencies installed, run the same
14 cases with mocked Tally and no live node access. On Linux, run in a
network-isolated namespace from a trusted clone:

```sh
unshare -rn node scripts/check_legacy_funding_runtime.cjs \
    --js ../tradelayer.js
```

The runner checks that Git HEAD is precisely
`dca0888b827c5bc28fa9189bc9e1d507f5c5670c`, that tracked files
are clean, and that BigNumber retains default 20dp/HALF_UP. It intercepts
`clearing.js`'s source-level CommonJS imports, replacing the network,
database and tally modules with inert mocks. Captured calls are expressed in
atomic units and compared against the synthetic vectors.

This runner executes trusted local source and therefore **must** be run
under an isolated execution environment with no wallet keys, secrets, or
live RPC. On Windows, use a network-isolated VM/sandbox with a separate copy
of the source. No `npm install`, broadcasts, wallets or live node calls
are performed by the runner.

## A further block-cadence question

`src/clearing.js:553–575` only applies funding on heights divisible by
24. The catch-up loop in `src/main.js:350–405` calls
`Clearing.clearingFunction` only when its `skip` flag is false; otherwise
some heights divisible by 24 can be skipped in catch-up replay. This needs
a separate deterministic scheduler characterization, especially for chains
with blocks containing no TradeLayer transactions.

## Authorization

Any proposed historical correction is a consensus change. This code
has no connection to the Stage 22 single ledger or Stage 23 authenticated
Litecoin reader. Types 3/23/31, account transfers, economic settlement and
Cairo/STWO proof authorization remain default-deny. Real historical
acceptance, deployment revision and migration/activation policy have
not been established.
