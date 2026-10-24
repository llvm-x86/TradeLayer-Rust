# Stage 25E1: fail-closed liquidation and insurance shortfall preflight

## Rationale and scope

The goal is a Rust-native risk engine, **not** a blind port of old JavaScript
liquidation side effects. This stage adds **read-only** liquidation diagnostics
to the Stage 25B–D single-ledger clearing model without relaxing execution
authorizations or reusing the legacy insurance database as a second ledger.

Pinned historical source: [tradelayer.js@dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c)

| Source interval | Observed behavior | Design implication |
| --- | --- | --- |
| \`src/clearing.js:1865–1970\` | Loss coverage is checked before classifying insolvency and queuing liquidation | A **projection** must distinguish maintenance breach from actual bankruptcy |
| \`src/clearing.js:2148–2173\` \`applyLossPoolDrain\` | Available funds are used first, then margin, with remainder assigned to ADL | This ordering is historical observation, not approved future consensus |
| \`src/clearing.js:2345–2449\` | Liquidation looks at AMM first and only takes bounded prices, followed by orderbook paths | A status/priority list is **not** an authenticated market fill |
| \`src/clearing.js:2685–2828\` | Seizure, ADL, position clearing and counterparty credits occur in different async paths | Require one atomic debit/credit system in Rust, no partial commits |
| \`src/insurance.js:7–107,190–239\` | Insurance balances are maintained in a *separate database* and payouts debit that separate array | **Never** import the disconnected balances as a spendable Rust insurer |

The user’s November–January JavaScript clearing rewrites and spam testing
are an important reason to retain a strict historical-observation versus
intended-new-rule distinction.

## Rust implementation

\`src/liquidation_audit.rs\` provides:

- \`assess_liquidation_risk_unverified(positions, posted_margins, mark, maintenance)\`:
  compares signed **projected** per-owner mark PnL with real posted collateral.
  Produces post-mark signed equity, maintenance requirement, maintenance
  deficit, and the amount of predicted mark loss *not payable from the
  posted margin escrow*.
- \`ClearingSandbox::preview_liquidation_unverified(new_height, new_price_e8)\`:
  reads its *one* SpotExchange-owned Ledger’s real \`~clrm:\` margin
  balances, the frozen synthetic collateral policy, current signed lots,
  and Stage 25C's explicit PnL mode. **No** available account/spot reserved
  atoms are automatically recognized as pledged collateral.
- A strictly diagnostic severity ordering (insolvency, then descending
  maintenance gap, then lexicographic account ID) for easy fixed-seed
  counterexample comparison. This is **not** AMM price/time ordering,
  deleveraging counterparty selection or an executable liquidation queue.
- \`preview_loss_budget_unverified(loss, available, margin, insurance, rule)\`:
  independently compares \`LegacyAvailableThenMargin\` with
  \`PostedMarginOnly\` for a single hypothetical loss budget. It records
  each bounded source, insurance contribution and *unresolved* amount.

### Meaning of the calculated amounts

Given a posted margin \`M\`, a signed one-period mark PnL \`P\`, and a
maintenance obligation \`R\`:

\`\`\`
equity_after_mark      = M + P
maintenance_deficit    = max(0, R - equity_after_mark)
uncovered_mark_loss    = max(0, -P - M)  when P is negative
\`\`\`

- **Healthy**: \`equity_after_mark >= R\`.
- **BelowMaintenance**: \`0 <= equity_after_mark < R\`.
- **Insolvent**: \`equity_after_mark < 0\`.

A below-maintenance but solvent account can be flagged without claiming
insurance is owed. A bankrupt account's uncovered mark loss is *not a
spendable debit or claim*. The mark projection's independent half-up
rounding residual is reported verbatim; a nonzero residual continues
to prevent actual Stage 25C settlement.

The fixed initial/maintenance threshold remains a **caller-configured
synthetic rule**, not a verified contract/leverage/notional economic
policy. The mark is not authenticated to an oracle. The same exact
signed account projection feeds both the read-only risk audit and
Stage 25C's fail-closed candidate mark.

### Hypothetical loss budget

For a requested loss of 100 atoms, posted margin 40, ordinary available
funds 200 and an **unverified observed** insurer with 30:

| Rule | Available | Margin | Hypothetical insurance | Unresolved |
| --- | ---: | ---: | ---: | ---: |
| Legacy available then margin | 100 | 0 | 0 | 0 |
| Proposed posted margin only | 0 | 40 | 30 | 30 |

The output is always purely mathematical:

\`\`\`
requested = available_used + margin_used + insurance_used + unresolved
\`\`\`

**Do not use the input \`hypothetical_insurance_atoms\` to authorize a debit.**
It is not bound to an actual Ledger escrow or globally reserved across
multiple claimants. Two independent previews can each describe using
the same hypothetical insurance amount: they **cannot** both execute
against a single reserve. The Rust engine currently contains no
spendable insurance escrow and no ADL transfer route.

## Security and denial

This stage creates **no** new state-changing clearing operation, no
liquidation execution, no insurance debit, no callbacks, no mark
advance, no combined-checkpoint format revision and no extra replay
markers. TLCL/TLCC v1 and all previous snapshots remain byte
compatible. Type 3/23/31 economic settlement stays default-deny.

The source-independent risk audit rejects:

- Missing/zero maintenance policy and incomplete owner-margin maps
- Projected PnL accounts that differ from current signed-position owners
- Unbalanced global open interest, zero-size positions or >128 positions
- Overflow in checked integer maintenance, equity or aggregate risk
- Price/height/queued-spot input prohibited by Stage 25C's mark preflight

No payout or reservation authority can be inferred from a green test,
CRC, property conservation check, or verified-looking filename.

## Offline tests

\`\`\`sh
python3 scripts/check_rust_supply_chain.py
node scripts/audit_liquidation_loss_budget.cjs --check
cargo test --frozen --test liquidation_preflight
cargo test --frozen
cargo clippy --all-targets --frozen -- -D warnings
cargo fmt --all -- --check
cargo build --all-targets --release --frozen
\`\`\`

The tests cover 16 independent BigInt source-inspired loss budgets
(including \`u128::MAX\`), below-maintenance versus insolvent distinctions,
correct shortfall accounting, mark-rounding residuals, signed OI and
overflow rejection, no order/escrow mutations, fixed-seed repeated
risk observations and deterministic diagnostic priority.

## Stage 25E2 prerequisite

Before **any** insurance-funded transfer or forced liquidation:

1. Ratify a versioned contract/price/margin/liquidation rule, including
   close-out priority, counterparty matching and permissible seizure sources.
2. Introduce a real domain-separated \`~ins:\` reserve inside the **same
   existing SpotExchange-owned Ledger**. Funding it must debit an actual
   ordinary owner account. Never trust the old insurance DB balance.
3. Extend the combined TLCL/TLCC checkpoint with a new version and
   explicit, independently validated insurer ownership, contributions
   and replay history, while retaining safe interpretation of old v1
   checkpoints and keeping TLSP/TLJP history fences intact.
4. Atomically stage liquidation fills, position reductions, margin/insurer
   transfers and unresolved bad-debt state; fail before any unbacked
   creditor payout. Fault-inject each transfer boundary and restore
   every byte after failure or reorg.
5. Run the old pinned JavaScript clearing method under a fully isolated,
   dependency-pinned harness and record observed outcomes separately from
   proposed economics. Supply-chain gate stays zero-crate.

The next stage can build this without scanning testnet or touching funds.
