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

The old TLSP spot snapshot has no position or margin-owner metadata.
**It is now forbidden to encode any ledger with non-spot `~` reserve
balances as TLSP.** This preserves existing TLSP/TLJP/TLJC format semantics
instead of producing an apparently valid but orphaned checkpoint. The
joined clearing object has **no persistence API** pending a new versioned
combined checkpoint and reorg proof. Opening an existing TLSP with
unrecognized reserves is also rejected.

## Tests and known limitations

`tests/spot_clearing_one_ledger.rs` performs mixed token issuance,
ordinary sends, matched derivative positions, spot matching, margin funding,
open spot orders and replay collisions. It checks both properties' supplies,
concurrent spot and margin reserves, no reserved-account impersonation and
late-failure rollback. Stage 25A's fault-injection tests remain intact.

**No clearing policy parity is claimed.** This stage does not add maintenance
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
cargo test --frozen
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
```

Next stages require defined initial/maintenance margin and exact PnL,
a separately reviewed price/notional scale, one authentic contract registry,
a combined versioned checkpoint, and a signed-chain authorization boundary.
No historical JavaScript bugs are silently promoted into consensus.
