# Deterministic fee engine (research, not consensus)

`src/fees.rs` is a standalone, integer-only maker/taker fee calculator.

**Scope and non-claims**

- The fee engine is **not wired into the spot matcher** (`src/spot.rs`), the
  ledger (`src/ledger.rs`), the chain replay, or any checkpoint format.
- It is **not consensus**. It defines no historical TradeLayer fee rule and
  claims no parity with the legacy JavaScript implementation.
- It moves no balances and authorizes nothing. It only computes integer fee
  amounts from a caller-supplied schedule and quote amount.
- It uses only `std`, has no third-party crates, and performs no floating-point
  arithmetic.

## Schedule

`FeeSchedule { taker_fee_bps: u32, maker_fee_bps: i32 }`, in basis points.

- A **negative** `maker_fee_bps` is a rebate paid to the maker.
- `FeeSchedule::new` and `settle_fill` both reject:
  - `taker_fee_bps > 1000`,
  - `|maker_fee_bps| > 1000`,
  - a rebate whose magnitude exceeds the taker fee
    (`-maker_fee_bps > taker_fee_bps`).
- Rebate equal to the taker fee is allowed. That yields zero protocol net on
  exact multiples and never a negative value.

The rebate guard is the core safety rule. If a venue paid out more than it
collected on a trade, the venue would be fee-negative and self-trades through
the schedule would be profitable. Public fields permit struct literals, so
`settle_fill` re-validates every schedule it receives.

## Settlement

`settle_fill(&schedule, quote_amount_atoms: u64) -> Result<FillFees, FeeError>`

| Field | Meaning |
| --- | --- |
| `taker_fee_atoms: u128` | Charged to the taker, rounded **up**. |
| `maker_fee_atoms: i128` | Positive: charged to the maker, rounded **up**. Negative: rebate paid to the maker, rounded **down**. |
| `protocol_net_atoms: u128` | `taker_fee_atoms + maker_fee_atoms`. Never negative. |

- Fees are denominated in the **quote asset** of the fill.
- `quote_amount_atoms == 0` is refused (`ZeroQuoteAmount`).
- Intermediate products are `u128` with checked multiplication.
- Rounding is directional. Charged fees round up against the payer and rebates
  round down against the recipient, so `protocol_net_atoms >= 0` always.

Accounting identity:

```
protocol_net = taker_fee - rebate_paid = taker_fee + maker_fee_atoms
```

## Accrual

`FeeAccrual` keeps a `BTreeMap<PropertyId, u128>` of protocol net per quote
property. `accrue(property, &fees)` uses checked addition. On overflow it
returns `ArithmeticOverflow` and changes nothing.

## Tiers

`FeeTier::new(min_trailing_volume_atoms, taker_bps, maker_bps)` validates the
schedule. `FeeTierTable::new(Vec<FeeTier>)` requires:

- non-empty, at most `MAX_FEE_TIERS` (16) tiers,
- the first threshold is `0`, so every volume maps to a tier,
- thresholds strictly increasing (duplicates and decreases are rejected),
- every tier's schedule valid (including rebate at most taker fee).

`schedule_for_volume(trailing_volume_atoms)` is a pure lookup. It returns the
highest tier whose threshold is at or below the caller-supplied volume. It has
no account state and no clock.

## Verification

`tests/fees.rs` covers:

- exact vectors: 1,000,000 atoms at 10/-2 bps, 1 atom with rounding in both
  directions, `u64::MAX` at several schedules, and rounding boundaries;
- a seeded (`LCG`, fixed seed) sweep of 250,000 random (schedule, amount) pairs.
  It checks ceiling/floor rounding against independent specifications, the
  accounting identity, `protocol_net >= 0`, agreement between the constructor
  and the struct-literal path, and per-property accrual totals;
- validation rejections for schedules and tier tables;
- a self-trade bound: owner cost `taker_fee + maker_fee >= 0` over a grid of
  valid schedules and amounts;
- accrual overflow refusal.

Run with the commands in `AGENTS.md`.
