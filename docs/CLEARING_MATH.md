# Stage 24A: Clearing arithmetic kernel (offline research)

This is the first isolated Rust slice mapped directly to the otherwise
unported legacy `src/clearing.js` module. It never changes ledger balances,
funding accounts, maintenance margin, insurance, positions, or settlements.

**Reference:** [tradelayer.js@dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c)
and these actual source intervals:

| Formula | Legacy JS source | Rust |
| --- | --- | --- |
| Option-adjusted loss coverage | `src/clearing.js:50-64` `computeLossCoverage` | `compute_loss_coverage` |
| Funding-rate deadband | `src/clearing.js:661-668` `clampFundingRate` | `clamp_funding_bps` |
| Linear/inverse mark-to-mark PnL | `src/clearing.js:2189-2208` `calculateMarkToMarkPNL` | `mark_to_mark_pnl_atoms` |

The source is independently reviewed by its pinned SHA, and the 15 synthetic
test rows are calculated in `scripts/audit_clearing_math.cjs` using
dependency-free Node BigInt. The vectors are *mathematical diagnostics*, not
live JavaScript integration results or proof of historical consensus.

## Fixed-point representation

Public amounts are integer atomic units, where 100,000,000 atoms = one
quantity unit. Contract count is a signed integer. Price and notional inputs
are nonnegative decimal quantities scaled by 1e8. Fractional contract counts,
negative or non-atomic prices, nonfinite values, and dynamically modified
BigNumber precision remain unported.

### Loss coverage

The pinned JS returns:
```
coverage = available + (margin / 2) + premiumMTM - maintNaked
```
Its `margin / 2` can be a *half atom*. Rust returns
`coverage_twice_atoms`, an integer in half-atomic units:

```
2 * available_atoms + margin_atoms
+ 2 * premium_mtm_atoms - 2 * naked_maintenance_atoms
```

A hypothetical atom-denominated loss is coverable only if
`2 * loss_atoms <= coverage_twice_atoms`. This is diagnostic math and
does not authorize debits, liquidations, or an actual collateral waterfall.

### Funding deadband

For integer basis-point input `r`, matching the pinned source:

```
if r > 5: r - 5
if r < -5: r + 5
otherwise: 0
```

The surrounding JS funding algorithm has unported asynchronous state reads,
cash transfers, contracts weights, and nonzero-sum rounding. This Rust function
only describes the rate deadband, with no periodic scheduling or transfers.

### Mark-to-mark PnL

A linear contract uses:
```
pnl = (to - from) * signedContracts * notional
```

An inverse contract uses:
```
pnl = (1/from - 1/to) * signedContracts * notional
```

All final amounts are rounded to 1e-8 collateral units, halfway away from
zero. A zero source or destination price produces zero PnL because the legacy
function does so. A zero notional defaults to **one** as the legacy
`notional || 1` expression does.

Two rounding modes are exposed:

- `Legacy20DecimalPlaces`: models the default BigNumber.js
  `DECIMAL_PLACES=20` reciprocal rounding, separately for `1/from`
  and `1/to`, then rounds the final result to eight decimal places.
- `ExactRational`: retains rational precision through the reciprocal
  difference and rounds only the final result. It is a proposed arithmetic
  policy for new designs and **can disagree by whole atoms** with the
  legacy sequence.

The vector `20dp_inverse_divergence` demonstrates a **one-atom** difference
(legacy `36633685829`, exact `36633685828`), even though both are based
on the same algebraic inverse formula. An eventual consensus design must
select and explicitly activate the intended policy; it must never silently
replace the historical rule while replaying legacy state.

The old `src/marginMap.js:1275-1297` `estimatePNL` calculates the
**linear** case *without multiplying by notional*, unlike this
`clearing.js` method. Reconciling the call graph and settlement source of
truth is required before connecting either formula to positions.

The model is intentionally bounded by checked `u128` intermediates and
returns `ArithmeticOverflow` for values outside its integer-supported range.
It cannot produce a signed ledger operation or represent an authenticated
economic proof.

## Validation

```sh
node scripts/audit_clearing_math.cjs --check
cargo test --locked --test clearing_math
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

## Subsequent ports

Next economic work must map which code path computes binding liquidation
funding, whether cumulative funding debits/credits conserve supply after
per-address rounding, and how loss coverage interacts with insurance and
position maintenance. Implement these as independently tested numerical
invariants first. Preserve Stage 23's authenticated Litecoin replay boundary,
and retain types 3, 23 and 31 economic settlement default-deny.

See `docs/JS_RUST_PORT_AUDIT.md` in the parallel documentation PR for the
full module inventory.
