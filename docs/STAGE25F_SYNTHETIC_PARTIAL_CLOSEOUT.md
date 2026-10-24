# Stage 25F (bounded slice): synthetic bilateral partial reduction

**Status:** offline, fake counterparty match ONLY. There is no real liquidation
authority, no claimed historical TradeLayer consensus parity and no trusted
chain adapter. The existing whole-book Stage25E2B closeout remains untouched
and entirely separate.

## Provenance and concurrent work

- Baseline: \`TradeLayer-Rust@be15554453581dd44c2f314d512cb00b4a90a54f\`,
  containing the merged PR #79 head
  \`db28db75e22d77700552f49261c94fc114bb7be6\`.
- Scope contract: [Issue #80](https://github.com/patrickdugan/TradeLayer-Rust/issues/80),
  unassigned with no separate active Stage25F PR at branch creation.
- This work does not touch concurrent \`formal/\` and \`ci/stage22-*\`
  branches, the active Stage23 draft, upstream \`tradelayer.js\`, live
  blockchain data or any crate/dependency policy.

Pinned historical JavaScript:
[\`tradelayer.js@dca0888b827c5bc28fa9189bc9e1d507f5c5670c\`](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).

| Source | Observation | Policy NOT inferred |
| --- | --- | --- |
| \`src/clearing.js:1937–1984\` | Claims positive loss coverage lead to a *partial* rather than *total* liquidation label | Who is legally permitted to seize, close, or choose a market counterparty |
| \`src/clearing.js:2488–2529\` | Deficit/initial-margin heuristic divides residual by initial margin and uses an eight-decimal, potentially fractional liquidation amount | No rounding/ceil/floor policy for a Rust **integer-lot** partial close |
| \`src/clearing.js:2232–2288\` | Linear and inverse bankruptcy prices are derived from last mark, equity, contracts, and notional | No ratified bankruptcy/ADL boundary or trusted price feed |
| \`src/clearing.js:2343–2449\` | AMM liquidity is prioritized at safe boundary prices; bids descend, asks ascend, \`blockTime\` breaks ties; inverse flips safe-side inequality | No maker/fill rights, AMM reserve model or synthetic fill selection |
| \`src/orderbook.js:720–774,1373–1440\` | Sorts by price then block time and uses earlier order price as maker price | No authenticated orderbook, fees, maker rebates or tie-breaking adapter |
| \`src/clearing.js:2575–2678\` | Uses \`estimateLiquidation\`, AMM sweep, actual matched-order results, and size capped by safe filled amounts | No executable fill proof from a user-supplied \`execution_price_e8\` |
| \`src/clearing.js:2688–2828\` | Unfilled size enters ADL and collateral-pool seizure/credit pathways | No ratified waterfall, bad-debt handling, insurer or ADL permissions |

The November–January JavaScript clearing rewrites make these evidence of
*code behavior*, not authority for the economic rule now in Rust.

## Admitted, explicitly synthetic experiment

The single new research type \`SyntheticBilateralReduction\` contains:

\`\`\`text
long: existing ordinary owner with signed position > 0
short: distinct existing ordinary owner with signed position < 0
contracts: integer quantity > 0, strictly less than EACH owner's open lot
execution_price_e8: strictly positive caller-declared price
\`\`\`

\`reduce_bilateral_positions_unverified(external_txid, height, &fill)\`
has **one pair** and at most **one collateral transfer**. There is no
unilateral partial seizure. There is no change to \`MarkPolicy\` or
\`CollateralPolicy\`. It neither opens nor flips any position, and does
not infer any maker from a client-provided address. To be *strictly
partial*, BOTH original positions must remain nonzero with the same
sign after reducing equal-and-opposite lots.

The existing immutable \`MarkPolicy\` specifies a last reference mark
price, notional, linear/inverse mechanics and its explicit
\`Legacy20DecimalPlaces\` or \`ExactRational\` mode. **No new rounding or
pricing algorithm is chosen.** For the removed lot quantity \`q\`,
the existing Stage24A function computes the two signed hypothetical
realized changes:

\`\`\`text
long PnL  = Stage24A_PnL(+q, prior_mark, declared_fill, frozen terms)
short PnL = Stage24A_PnL(-q, prior_mark, declared_fill, frozen terms)
\`\`\`

The required sum is **exactly zero**, otherwise the batch rejects with
\`PnlRoundingImbalance\`. This is a *declared reference-mark convention*,
not proof of actual per-lot cost basis or fill execution. Only **the
losing owner's real, pledged \`~clrm:\` escrow** may pay the winning
owner's \`~clrm:\` escrow, through the existing restricted
\`SpotExchange::apply_clearing_transfer_unverified\` seam. If the payer
does not have enough margin, the entire operation rejects: ordinary
available accounts, queued or resting \`~spot:\` reserves and real
\`~ins:\` insurer funds are never used.

The transaction operates on an entire staged clone of the existing
**single** SpotExchange-owned Ledger, position table and replay IDs.
Only if every transfer and the reduced lots pass spot-reserve, signed
open-interest, maintenance, conservation and escrow-owner invariants is
the state swapped into place. The operation records one fresh external
replay ID and fences old spot-only snapshots even when its calculated
PnL transfer is zero. A fault at the single transfer boundary or
immediately after transfer but before lot changes restores the exact
pre-call TLCL v2 state.

**Temporal bound:** an execution height must be strictly after the last
configured mark and strictly after a previously finalized spot block.
A due-but-queued spot order prevents admission, rather than letting
a guessed order/mark priority decide the simulated fill. This does not
supply market order authentication. The operation does not advance the
global mark: unaffected positions are still referenced to its prior
value, and those other owners are not settled or revalued.

### TLCC research-only integration

\`UnverifiedClearingEvent::SyntheticBilateralReduction\` is a distinct
fake event, executed within the existing synthetic TLCC block journal.
It is not an op code in \`JointProtocolState\`, and types 3/23/31 remain
default-deny. TLCL/TLCC v2 need **no format change**, since already
persisting the updated sole Ledger balances, signed lots, position
owners, replay IDs and mark terms. A whole-block failure restores all
prior spot, margin, insurer, mark and participant state, and a retained
rollback witness permits a replacement synthetic fork.

## Deliberately unanswered economic decisions (fail closed)

1. **Actual price and match authorization:** no authenticated maker, AMM
   depth, safe-bankruptcy boundary, execution timestamp, order nonce,
   signed chain provenance or counterparty consent/proof exists. The
   declared fill can only be an unverified test input.
2. **Which owners may be forced:** there is no liquidation-eligibility
   authority, liquidation priority, risk-level ordering, margin seizure
   mandate or protocol rule that can select an unwilling counterparty.
3. **Partial quantity and execution:** historical JS uses decimal
   \`deficit / initialMargin\`, AMM-first sweep and price/time ordering.
   Their precision, bankruptcy clamp, fees, partial-fill rounding and
   intent/lot identities are not ratified in Rust. Stage25F admits only
   one strictly partial *integer* pair; full closure and an unfilled
   remainder reject.
4. **Cost basis and realized PnL:** the existing position map stores only
   signed size, not trade-entry prices. The reference-mark convention
   implemented for synthetic tests is NOT a historical realized
   cost-basis rule and cannot settle a live derivatives contract.
5. **Loss waterfall:** insurance reserve contribution, fee cache,
   confiscating ordinary available balances, bankruptcy price, ADL,
   systemic bad debt or debt/IOU issuance are not permitted. Insufficient
   pledged margin rejects with zero winner payout.
6. **Contract/source binding:** IDs, fill prices, contract notional,
   participant names and heights remain independently unverified
   assertions. Correct checksums, conserved supply and recovery from a
   checkpoint do not prove economic or blockchain validity.

A trusted liquidation executor must separately ratify and authenticate
all six dimensions. **This PR does not do so.**

## Focused offline tests

\`\`\`sh
python3 scripts/check_rust_supply_chain.py
rustfmt --check --edition 2021 src/clearing_partial_impl.rs
cargo test --frozen --test clearing_partial_closeout
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
\`\`\`

The tests cover directional payer/receiver escrows, exact collateral
conservation, unchanged unrelated owner lots and resting spot orders,
zero-PnL replay fencing, wrong-side/missing/oversized lots, depleted
payer margin with untouched insurer, post-transfer maintenance failure
rollback, repeated and competing claim IDs, failure injection before
and after the sole possible transfer, stale heights and queued spot
ordering, TLCL v2 round-trip and TLCC v2 rollback/alternate synthetic
fork. No testnet/node scan or resource-heavy job is authorized.

Keep the PR **draft, unmerged**, for code and economic-policy review.
