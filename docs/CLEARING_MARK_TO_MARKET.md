# Stage 25C: atomic mark-to-market on the one spot-owned ledger

This stage extends [Stage 25B](SPOT_CLEARING_SINGLE_LEDGER.md) with
a **bounded, unverified** mark-to-market settlement experiment. It does not
authorize derivative trading or a production consensus transition.

The economic owner remains:

\`\`\`
ClearingSandbox
  SpotExchange
    Ledger                       ← the only token balances
      ordinary addresses
      ~spot:... order escrows
      ~clrm:... margin escrows
  signed positions                ← metadata, not separate token balances
  collateral policy               ← optional synthetic requirements
  mark policy                     ← initial mark, notional, inverse and rounding
  clearing replay IDs             ← not serialized by legacy TLSP/TLJP
\`\`\`

All transfers occur between already funded \`~clrm:\` margin accounts in the
sole existing \`Ledger\`. Neither the mark calculation nor the transfer
graph can mint, burn, spend spot reserves, or credit a winner without
first debiting a loser. Spot matching, ordinary sends and margin funding
retain their existing conservation checks.

## Explicit, immutable research economics

The caller must opt in to \`with_mark_policy_unverified(MarkPolicy)\`
**before any clearing event**:

| Field | Meaning |
| --- | --- |
| \`height\` | Claimed initial mark epoch (nonzero, unverified) |
| \`price_e8\` | Positive starting mark price, scaled by 1e8 |
| \`notional_e8\` | Positive per-contract notional quantity, scaled by 1e8 |
| \`inverse\` | Select linear or inverse price formula |
| \`rounding\` | Legacy BigNumber 20-decimal reciprocal sequence or exact rational |

The notional, price convention, contract identity and initial height are
**not** authenticated against the legacy contract registry, a price oracle,
or a signed transaction. The price and height advance **only after** a
successful clone/check/swap mark settlement. Old or duplicate heights,
unconfigured policies, and a second policy configuration are refused.

The configured initial mark cannot predate an already finalized spot height,
or be installed while spot input events remain queued. Once configured,
spot events/blocks must have a height **strictly greater** than the current
mark, while a new end-of-block mark must be at least the latest finalized
spot block and strictly later than the preceding mark. The mark also
rejects any **queued spot order due at or before that mark height** until
the spot book has actually processed the corresponding height. A mark can
therefore settle spot activity at the same height but cannot be backdated
behind it or bypass a ready queue. These checks are *local research
ordering constraints*, not verified L1 order or a replacement for the
authenticated Stage 23 replay boundary.

The selected policy uses the pure Stage 24A PnL formulas:

\`\`\`
linear:  (toPrice - fromPrice) * signedContracts * notional
inverse: (1/fromPrice - 1/toPrice) * signedContracts * notional
\`\`\`

The exact scaling and two rounding policies are implemented in
\`clearing_math.rs\`, with independent Node BigInt vectors. This stage
uses the **same** method for each participant and never silently
invents a production price source or a liquidation price.

The old pinned \`tradelayer.js/src/marginMap.js:1275–1297\` linear estimate
omits notional whereas \`clearing.js:2189–2208\` includes it. Thus this
is an explicit *proposal-specific mark engine*, not historical JS parity.

## Rounding residual is an economic policy decision

Per-address rounded PnL need **not** sum to zero even when signed open
interest sums to zero. Example with a 0.5-unit notional, price advance of
one atomic tick, and signed positions \`+1, +1, -2\`:

\`\`\`
individual PnL: +1 atom, +1 atom, -1 atom
net residual:   +1 atom
\`\`\`

\`clearing_pnl::project_pnl_unverified\` returns every signed delta,
payer/receiver aggregates and the nonzero residual **without mutating
state**. \`settle_mark_to_market_unverified\` rejects the same input with
\`PnlRoundingImbalance(1)\`: it must not create that extra atom or impose
an arbitrarily chosen shortfall on one participant. A future allocation
policy requires explicit separate review and activation, with account-level
semantics documented in the protocol.

Independent fixture generation:

\`\`\`sh
node scripts/audit_clearing_pnl.cjs --check
cargo test --frozen --test clearing_mark_to_market
\`\`\`

The fixture \`fixtures/clearing_pnl_vectors.tsv\` covers 11 independent
signed-position books: linear/inverse, legacy/exact, symmetric pairs,
roundoff imbalance, zero price movement, and a BigNumber intermediate
precision case.

## Atomic execution and denial behavior

The research API is:

\`\`\`rust
preview_mark_to_market_unverified(new_height, new_price_e8)
settle_mark_to_market_unverified(external_txid, new_height, new_price_e8)
settle_mark_fault_injected_unverified(external_txid, height, price, leg)
\`\`\`

The committed transfer path checks:

1. The externally provided event ID has not appeared in clearing,
   inherited Ledger operations, or spot history.
2. The mark policy exists, new height is strictly greater, price is
   positive, and PnL projection has exact zero aggregate rounded residual.
3. Every debit/credit edge refers to a registered \`~clrm:\` margin owner,
   cannot touch any \`~spot:\` or arbitrary reserved account, and all
   debits must be fully funded in the **one existing Ledger**.
4. Every transfer is applied to a private **staged clone**. After every
   leg, failure injection can abort and discard all changes.
5. A successful mark updates the price/height, then verifies supply
   conservation, spot reserves, margin escrow ownership, zero global
   position count and configured maintenance floors **before swapping** the
   cloned state into the original.
6. The outer replay ID is consumed only on a successful commit.

If mark losses would violate maintenance, the whole mark transaction
rejects. This **does not constitute liquidation**; it is an explicit
blocker until a separately ratified liquidation/insurance waterfall exists.
No partial winners are paid on a failing batch.

### Checkpoint and zero-transfer replay fence

Stage 25B correctly blocks old TLSP/TLJP checkpoints whenever actual
clearing transfers occurred. Stage 25C closes a subtler case: a no-position
mark or zero-funded clearing event may consume an external replay ID
**without moving a single token atom**.

The owner now sets \`SpotExchange::clearing_activity\` on every successful
clearing research event, not only an internal ledger transfer. Consequently
the old spot-only snapshot cannot discard *empty* clearing event history
or allow a fresh clearing sandbox to reimport it after margins are paid out.

This remains an in-memory safeguard. A durable combined spot/clearing/
registry snapshot with reversible history is still missing and must not
be represented by TLSP/TLJP.

## Tests and scope

\`\`\`sh
python3 scripts/check_rust_supply_chain.py
node scripts/audit_clearing_pnl.cjs --check
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen
cargo build --all-targets --release --frozen
\`\`\`

New adversarial cases cover: exact and 20dp PnL vectors; positive and
negative mark progression; inverse payoff; ledger-only margin payments;
unfunded loss denial despite available spot reserves; maintenance checks;
duplicate IDs; stale and zero prices; late failure injection; one-atom
roundoff refusal; zero-transfer checkpoint fence; and a deterministic
100-epoch conservation stress.

**Still unsupported:** independent price authentication, trade execution
prices and cost basis, fractional contracts, full leverage and collateral
unit policies, insurance/backstops, liquidation, margin funding scheduling,
multi-contract portfolio cross-margin, crash consistency and signed-chain
replay. Types 3/23/31 remain default-deny. Source-level numerical parity
is not evidence of historical consensus agreement.

No third-party Rust crates, new Cargo packages, build scripts or network
dependencies were added.
