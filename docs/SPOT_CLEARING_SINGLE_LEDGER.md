# Stage 25B: one owned ledger for spot and clearing (offline research)

This stage removes the double-ledger architectural hazard from the Stage 25A
clearing sandbox. The clearing object now exclusively **owns a SpotExchange**,
which exclusively owns the one token **Ledger**. Both spot-order reservation
and margin escrow touch the same supply-accounted token balances.

It is not linked into `JointProtocolState`, `JointChainState`, any Litecoin
node, signed-chain adapter, economic-proof verifier or production settlement.

## Ownership, API and trust boundaries

```
ClearingSandbox (one unverified research contract, one collateral)
  ├── SpotExchange          ← sole Ledger owner; existing spot orderbook
  │     ├── Ledger          ← all issuer, wallet, spot reserve, margin assets
  │     └── queued/books/fills
  ├── positions             ← signed counts, never tokens
  ├── margin_escrows        ← owner → "~clrm:" Ledger account
  └── seen                  ← research-event replay IDs
```

`from_spot_unverified(network, contract, collateral, spot)` consumes an
existing `SpotExchange` by value. It checks the collateral property,
existing live spot reserve integrity, no channel/settlement liabilities and
no preexisting unowned clearing escrow. `from_ledger_unverified` remains a
compatibility constructor, bootstrapping an empty-permission `SpotExchange`
that becomes the one and only owner.

The following APIs are **unverified caller-input experiments**:

- `apply_ordinary_operation_unverified`: issue/send/mint/burn through the
  spot-owned ledger; `~` accounts cannot be named as ordinary actors
- `enqueue_spot_order_unverified` and `process_spot_block_unverified`:
  original Stage 19 spot queue and matching; actor cannot be any reserved
  `~` account
- Stage 25A margin deposits, margin withdrawal while flat, matched positions,
  funding transfers and fail-at-every-leg fault injection

Every method clones the joined object, applies the action, checks both
`SpotExchange::verify_reserves` and margin-owned `~clrm:` escrows, checks
`Ledger::verify_conservation`, and commits the entire object or nothing.
An inherited spot input ID cannot be reused by a clearing operation, and
vice versa. Internal debit IDs are distinct from external replay events.

## Two reserved account namespaces

Ordinary token and spot operations reject all synthetic `~` accounts.
The internal clearing transfer seam is `pub(crate)`, restricted to a
transfer that actually touches `~clrm:` and no `~spot:` or unknown
reserved namespace. Only clearing metadata chooses the `~clrm:` account
using its pinned genesis/contract/collateral/owner derivation.

The old TLSP spot snapshot has no position, margin-owner or clearing
event replay metadata. Every internal clearing transfer permanently sets a
`clearing_activity` fence on its owning SpotExchange, even if all margin is
later withdrawn and the escrow balances become zero. **TLSP serialization,
JointProtocolState migration and creation of another clearing sandbox from
that spot state all reject this history-bearing instance.** The old
TLJP/TLJC formats remain unchanged and are barred from importing the
unrepresented event history. No TLSP/TLJP writer may orphan collateral or
lose past clearing replay IDs. The joined clearing object has **no
persistence API** pending a new versioned combined checkpoint and reorg
proof. Opening an existing TLSP with unrecognized reserves is also rejected.

## Opt-in initial and maintenance collateral floors

The joined sandbox now also supports a **caller-chosen immutable research
policy** through `with_collateral_policy_unverified(CollateralPolicy)`. It
accepts nonzero `initial_atoms_per_contract` and
`maintenance_atoms_per_contract`, requiring initial >= maintenance.
The configuration cannot be changed after clearing has begun.

After each synthetic matched trade, a participant increasing absolute
exposure or flipping direction must have locked margin >=
`initial_atoms_per_contract * abs(new_contracts)`. Other active positions
must stay above the maintenance floor. Checked `u128` multiplication
rejects overflow. Each funding transfer rechecks the maintenance floor
before committing any balance/position/replay-marker changes.

**Important:** these are fixed synthetic per-contract thresholds, NOT
reviewed TradeLayer initial-maintenance economics. They do not incorporate
historical contract notional, oracle mark, inverse payoff, leverage,
unrealized PnL, liquidation slippage or insurance. A production risk engine
must specify these formulas and enforce a versioned protocol rule, instead
of treating arbitrary caller-selected thresholds as trusted parameters.
Without an explicit policy, legacy Stage 25A sandbox remains unchanged.

Tests verify opening exposure limits, late-failing funding, immutable
policy configuration, spot reserve exclusion and retry after replenishing
collateral. A failed maintenance check conservatively declines the
proposed payment; it is *not* a liquidation algorithm.

## Tests and known limitations

`tests/spot_clearing_one_ledger.rs` performs mixed token issuance,
ordinary sends, matched derivative positions, spot matching, margin funding,
open spot orders and replay collisions. It checks both properties' supplies,
concurrent spot and margin reserves, no reserved-account impersonation and
late-failure rollback. Stage 25A's fault-injection tests remain intact.

**No clearing policy parity is claimed.** This stage does not provide production maintenance
margin, mark-to-market debits, multi-contract clearing, liquidations, insurance,
fees, bankruptcy, channel derivatives or any signed-chain authority.
The simulated matched trade can be submitted by a caller without a real
trade, price, maintenance proof or issuer permission. These deliberately
unsupported operations must remain outside the joint consensus dispatcher.
The existing Stage 22 `JointProtocolState` retains its sole spot-owned
Ledger, and no second independently spendable clearing ledger is introduced
into its state.

```sh
python3 scripts/check_rust_supply_chain.py
cargo test --frozen --test spot_clearing_one_ledger
cargo test --frozen --test clearing_sandbox
cargo test --frozen --test clearing_collateral_policy
cargo test --frozen --test clearing_checkpoint_fence
cargo test --frozen
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
```

Next stages require defined initial/maintenance margin and exact PnL,
a separately reviewed price/notional scale, one authentic contract registry,
a combined versioned checkpoint, and a signed-chain authorization boundary.
No historical JavaScript bugs are silently promoted into consensus.
