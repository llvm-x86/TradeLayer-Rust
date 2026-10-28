# Maker reward accounting (`src/rewards.rs`)

## Scope: accounting only

This module is an in-memory, integer-only calculator. It takes declared epoch
statistics and returns how a reward budget would be split among makers.

- It is **not** wired to the matcher, to the ledger, or to the registry.
- It mints, moves, and escrows **no token**. It produces **no payout transaction**.
- It is **not chain consensus**. Its inputs (`resting_notional_seconds`,
  `filled_notional_atoms`, `fees_paid_atoms`, `attested`, the budgets) are
  supplied by a trusted caller and are not verified here.
- **Legal caveat:** paying tokens as rewards for derivatives trading raises
  regulatory questions that depend on jurisdiction and product. This design
  must not go live without review by counsel.

## Inputs and outputs

`plan_epoch_rewards(params, fee_pool_atoms, emission_budget_atoms, makers)`

- `RewardParams`: `fee_share_bps` (<= 10_000), `max_account_share_bps` (1..=10_000),
  `fee_rebate_cap_bps` (1..=10_000), `require_attestation`. Degenerate values are
  rejected before any allocation.
- `MakerEpochStats` per maker: `resting_notional_seconds`, `filled_notional_atoms`,
  `fees_paid_atoms`, `attested`. Takers have no entry and are never paid.
- Output `RewardPlan { payouts, retained_atoms }`. `payouts` lists nonzero totals
  sorted by `AccountId`. `sum(payouts) + retained_atoms` equals the budget exactly.
  Duplicate `AccountId`s are rejected.

## Rules

1. Budget: `fee_budget = floor(fee_pool_atoms * fee_share_bps / 10_000)`, and
   `budget = fee_budget + emission_budget_atoms` (checked).
2. Score: `score_i = resting_notional_seconds_i` if `filled_notional_atoms_i > 0`
   and (`attested_i` or not `require_attestation`), otherwise `0`.
3. Two independent pools, each split pro-rata to score:
   `raw_i = floor(pool * score_i / total_score)` (zero if `total_score == 0`),
   using exact 256-bit intermediate arithmetic.
4. Per-account cap: `payout_i <= floor(pool * max_account_share_bps / 10_000)`,
   applied to each pool. The excess is **retained**, not redistributed in the
   same pass.
5. Fee-funded pool only: `fee_payout_i <= floor(fees_paid_i * fee_rebate_cap_bps / 10_000)`.
   Because `fee_rebate_cap_bps <= 10_000`, a maker's fee-funded payout never
   exceeds the fees it paid.
6. Floor remainders are **retained**. The function returns `retained_atoms` and
   never over-distributes.
7. Order independence: inputs are sorted by `AccountId` before any computation,
   and no `HashMap` iteration is used.
8. Overflow: all arithmetic is checked. Overflow returns an error and never panics.

## Anti-sybil analysis (what is and is not guaranteed)

The per-account cap is keyed on `AccountId`. It is therefore a limit on one
identity, not on one entity. These claims are argued below and tested in
`tests/rewards_accounting.rs`.

**(a) Splitting an entity across identities cannot raise its uncapped payout.**
With `max_account_share_bps = 10_000` and the fee rebate cap in force, splitting
score, fills, and fees across identities gives a total payout `<=` the combined
account's payout. This holds because
`floor(x) + floor(y) <= floor(x + y)`, `min(x_i, y_i) summed <= min(sum x_i, sum y_i)`,
and removing a zero-fill identity lowers the entity's share, since
`s / (O + s)` is increasing in `s`. Equality is **not** guaranteed; floors can
lose atoms. Test: `splitting_without_binding_cap_never_increases_payout_exact_example`
(5 becomes 4), plus 20,000 seeded random splits.

**(b) The per-account cap does not resist splitting.** Once the cap binds on the
combined account, splitting into identities can earn up to `k * cap`. Example:
pool 1000, cap 500, entity score 300 vs another account at 100. Combined, the
entity earns 500. Split 150/150, it earns 750. Test:
`per_account_cap_is_defeated_by_identity_splitting`. **Anti-sybil resistance has
to come from elsewhere**: attestation, which is an external allowlist, and the
fee rebate cap for the fee-funded part.

**(c) The fee-funded aggregate is bounded per entity, even across identities.**
For any set of identities controlled by one entity, the sum of fee-funded
payouts is `<= fee_rebate_cap_bps / 10_000 * (sum of fees those identities paid)`.
This follows from `sum floor(f_j * r) <= floor(f * r)`. Test:
`fee_funded_aggregate_is_bounded_by_fees_across_identities` and the seeded
random test. This is the sybil-resistant part of the design.

**(d) Wash trading of the fee-funded part is unprofitable.** A wash trader's
fee-funded payout is capped at `fees_paid * fee_rebate_cap_bps / 10_000`, so its
net fee-funded result is `<= 0` whenever the cap is at most 10_000. Tests:
`fee_rebate_cap_limits_wash_trader_to_its_own_fees` and
`wash_trader_with_zero_fees_gets_no_fee_funded_payout`.

## Residual risks (not mitigated by this module)

- **Emission-funded payouts are not fee-capped.** A zero-fee account that
  satisfies the fill and attestation gates still earns its emission share
  (`residual_emission_pool_is_not_fee_capped`). Wash trading can therefore farm the
  emission pool. Mitigation requires a separate design, such as capping emission
  payouts by fees or by independently verified maker flow. This is **not** solved
  here.
- **The filled-notional gate is self-reported.** A same-operator wash trade
  satisfies `filled > 0`. This module trusts the caller's statistics.
- **Attestation is external.** `attested` must come from the registry's active
  attestation for the chosen list, and the module does not check that.
- **Identity limits are not enforced here.** See (b).
- **No token, no custody, no settlement.** Everything above is accounting.

## Verification status

Commands run on the worktree `feat/reward-accounting` with
`RUSTUP_TOOLCHAIN=nightly-2026-04-29` (the pinned 1.90.0 toolchain was not used):

- `cargo fmt --all`: applied.
- `cargo clippy --all-targets --locked --offline -- -D warnings`: clean.
- `cargo test --offline --locked --lib rewards`: 7 passed.
- `cargo test --offline --locked --test rewards_accounting`: 19 passed, including
  100,000 seeded epochs and 20,000 seeded identity-split trials.

Not run here: the pinned 1.90.0 toolchain, `scripts/ci-preflight.sh`, the
`check_rust_supply_chain.py` policy check (which requires 1.90.0), and the full
repository-wide test suite beyond the commands above. The result of each is
stated in the pull request.
