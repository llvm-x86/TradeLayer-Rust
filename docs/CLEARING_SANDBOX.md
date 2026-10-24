# Stage 25A: atomic margin and funding sandbox (offline, zero crates)

This is the first **state-mutating Rust derivatives-clearing slice**, built
after the read-only Stage 24A/B/C arithmetic, funding, and cadence audits.
It deliberately replaces economic mutations with a small bounded Rust
state machine rather than wrapping the side-effectful legacy JS engine.

**Do not connect it to live consensus.** All trade matches, identities,
rates, transaction IDs and contract identifiers are *unverified caller
assertions*. This model does not accept Litecoin/Bitcoin/Dogecoin transactions,
validate signatures, reconstruct historical contract state, or authorize
type 3, 23 or 31 settlements.

## Architecture and custody of token atoms

\`ClearingSandbox\` exclusively owns **one** existing \`Ledger\`:

\`\`\`
ClearingSandbox
  network: Bitcoin | Litecoin | Dogecoin (separate instances)
  collateral_property: PropertyId
  contract_id: u32
  ledger: Ledger                     ← the ONLY stored token balances
    user available accounts
    domain-separated margin escrows
  positions: BTreeMap<AccountId,i64> ← signed sizes, never another balance
  margin_escrows: owner -> escrow ID  ← references into that same Ledger
  seen: BTreeSet<TxId>               ← outer research-event replay markers
\`\`\`

Posting margin is a real atomic \`Ledger::Transfer\` from the owner's
available balance to a reserved escrow. Funding transfers move existing
collateral **directly between escrow accounts**. Position-size changes
neither mint nor burn assets. The system never stores a second sum of
margin balances that could drift from the ledger.

Escrow names are \`~clrm:\` plus 32-byte double-SHA256 in hex over a
domain-separated namespace, pinned network genesis, contract ID, property
ID and owner bytes. Internal operation IDs also include an external event
ID and a leg nonce. This deterministic hashing is a research namespace,
not a signature or independently audited cryptographic authorization.

External actors beginning with \`~\` are rejected. A ledger containing
existing spot escrows, channels, withdrawal/settlement liabilities or
unrecognized \`~\` balances cannot be imported into this separate sandbox.
This prevents silently combining it with the Stage 22 SpotExchange and
its already-owned ledger. **This module is not yet a new JointChainState
opcode or approved migration path.**

## Supported offline transitions

| Transition | Validations | Economic mutation |
| --- | --- | --- |
| \`deposit_margin_unverified\` | Nonzero amount, ordinary account, known collateral, replay check, bounded owners | One checked ledger transfer into owner's reserved margin escrow |
| \`apply_matched_trades_unverified\` | Nonempty, bounded, distinct parties, posted margin, checked signed amount | Updates equal/opposite long and short sizes, no balance change |
| \`settle_funding_unverified\` | Rate and notional bounded; FundingMode::ConservingProposal preview with payer sufficiency and zero net | Deterministic funding legs transfer existing locked collateral, atomically |
| \`withdraw_margin_unverified\` | Must be flat; posted amount sufficient | Transfers locked collateral from escrow back to owner |
| \`settle_funding_fault_injected_unverified\` | Same checks, explicit failure at selected transfer boundary | **Never commits** on injected failure; available for offline robustness testing |

The funding projection reuses Stage 24B's **proposed** debit directions
and deterministic Hamilton apportionment. It does not claim parity with
the old JavaScript's negative-rate sign bug or independent rounding
leakage. A historical migration must explicitly choose and activate any
changed economic rule rather than silently applying this model.

## Atomicity guarantee and limitations

Every public mutation stages the *entire* state on a private clone and
checks replay markers, token supply conservation, escrow ownership and
zero signed aggregate open interest before swapping the candidate in.
The funding method additionally checks individual margin collateral
before producing any transfer. Injected faults at each transfer leg,
including the boundary after the final leg but before commit, leave
the original balances, positions and markers unchanged.

This clone-and-swap design is intentionally conservative and not a
production throughput scheme. It has no durable checkpoint, undo journal,
crash recovery, external block authorization or funding scheduler.
No proof is made that \`TradeLayerMainnet\`'s label matches the actual
chain input. Existing \`JointChainState\` adapters and Type 3/23/31
execution remain unchanged and default-deny.

**Not yet implemented:** fee routing, initial and maintenance margin
requirements, orderbook matching proofs, contract registration, inverse
funding units and per-contract notional validation, PnL/mark-to-market
posting, liquidation waterfalls, insurance, bankruptcy/ADL, channel
derivatives, balance snapshots and periodic funding heights. It has
one research contract and one collateral token, not a multi-series
engine. Insufficient funds reject; they do not trigger liquidation.

## Source provenance

The old JavaScript reference at
\`tradelayer.js@dca0888b827c5bc28fa9189bc9e1d507f5c5670c\`
was repeatedly rewritten, and surviving end-of-block state changes
must be tested as observations, not blindly adopted as consensus.
Stage 24B demonstrated attempted negative funding reversal and
rounding imbalance; Stage 24C demonstrated catch-up/realtime cadence
and funding-bucket disagreement. This new engine intentionally does
not inherit either implicit scheduling path.

## Test commands

\`\`\`sh
python3 scripts/check_rust_supply_chain.py
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen --test clearing_sandbox
cargo test --frozen
cargo build --all-targets --release --frozen
\`\`\`

Tests include: three-party funding in both directions; exact supply
conservation; one-owned-ledger escrows; replay including inherited
ledger marker IDs; late trade validation failure; insufficient
collateral; funding fault injection before each internal leg; complete
position close then withdrawal; cross-mainnet namespace isolation;
disallowed reserved actors; bounds and a fixed-seed multi-round
funding stress test. The Rust crate adds **zero dependencies**.

## Next stage

Stage 25B should implement typed collateralized positions, explicit
initial/maintenance limits and realized PnL on this same ledger;
Stage 25C should add ordered derivative orderbook fills, liquidation
and insurance as isolated invariant-preserving state transitions.
Before replay activation, provide a signed-chain source adapter and
a checkpoint/reorg journal with independent state-root and historical
JavaScript differential vectors.

Do not permit client-supplied \`MatchedContractTrade\` to become an
authenticated transaction type. A future verified adapter must provide
actual chain-based authority, economic trade validity and versioned
activation rules.
