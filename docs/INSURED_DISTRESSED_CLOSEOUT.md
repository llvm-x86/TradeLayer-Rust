# Stage 25E2B: atomic *synthetic* full-book insured closeout

**Scope: offline research only.** All transactions, parties, marks, contract
parameters, reserve sponsorship and finalization heights are
**caller-asserted, not authenticated**. This is an independent proposed
economic transition, not evidence of the deployed JavaScript clearing
consensus, an AMM liquidation, an insurance claim, or authority to pay
actual funds.

The aim is a discriminating Rust accounting step: can one state owner
settle a distressed synthetic book using **only existing losing margin
and the actual insurance escrow in the same Ledger**, then close every
long/short lot atomically, with no IOUs or separate NeDB balances?

## Explicit proposed rule: whole-book termination

This stage adds **only** the full-book closeout rule.
It does not make a trading priority or partial-liquidation policy claim.

- The same Stage 25C mark PnL method calculates one signed mark delta for
  every existing position, under the frozen linear/inverse and rounding
  model. The total rounded PnL must be **exactly zero**; an inconsistent
  residual rejects rather than being debited from the insurer by fiat.
- Stage 25E1 calculates maintenance post-mark using *actual*
  \`~clrm:\` posted margin and the synthetic immutable maintenance rule.
  The proposed full-book termination requires **at least one** account
  under maintenance. Otherwise it rejects \`NoDistress\`.
- Every losing account owes exactly its signed projected loss. Debit
  \`min(loss, margin escrow)\`; if the margin escrow is insufficient,
  add precisely the unpaid portion to the *one* \`~ins:\` reserve's required
  contribution. Unpledged ordinary account balances and \`~spot:\`
  order escrows **are never counted**.
- Before any debit, require the single real insurer balance to cover
  *all* remaining losing obligations. \`InsufficientInsurance\` aborts
  the entire batch without a credit, reduced position or consumed replay
  ID. This protocol proposes **no** bad debt, unpaid winner, insurer loan,
  ADL, emergency minting or haircut.
- Credit each winning account's \`~clrm:\` margin escrow the exact
  positive projected mark PnL. Construct deterministic, strictly
  positive payer-to-creditor transfer legs in lexicographic owner order.
  Loss-bearing margin escrows pay first, then one real insurer reserve.
  The entire transfer graph must reconcile exactly: debits = credits.
- **After all transfers succeed**, remove all matched long and short
  positions together and advance the mark height/price. Any margin
  remaining in a now-flat owner's escrow stays real ledger collateral,
  withdrawable through the preexisting checked margin-withdraw API.
- Every transition is staged on a private clone. A fault before any
  transfer leg, or after the final leg before commit, discards the
  **whole** state (spot book, margin, insurer, mark, policies, replay
  markers, positions). One external ID can commit only once.

There is **no** insurer withdrawal or standalone payout API. The
crate-private \`SpotExchange::apply_insured_closeout_leg_unverified\`
permits a \`~ins:\` → \`~clrm:\` transfer only after the outer
\`ClearingSandbox\` checks the exact derived escrow, contract,
collateral, signed PnL, all creditor IDs, sufficient funds and
distressed risk condition. The pre-existing \`~clrm:\` transfer seam is
**not broadened**. Unverified synthetically closeout events remain
outside \`JointProtocolState\`, \`JointChainState\`, and all authenticated
Litecoin/Bitcoin/Dogecoin adapters.

### Example

A short loses 100,000,000 atoms but has 25,000,000 atoms of posted
margin. A long gains 100,000,000 atoms. If the insurer holds at least
75,000,000 real atoms, the closeout transfers 25,000,000 from the
short's margin escrow and 75,000,000 from the insurer to the long's
margin escrow. Token supply does not change. Both positions become
flat. With only 50,000,000 insurer atoms, **nothing executes**.

Two successive independent previews do **not** reserve the same
insurance balance. If one committed closeout exhausts 75,000,000 of a
100,000,000 reserve, the next proposed 75,000,000 shortfall rejects
because only 25,000,000 remain.

## APIs and strong denial boundaries

- \`preview_distressed_closeout_unverified(height, price)\`: read-only,
  returns proposed losing-margin contribution, total real insurer
  requirement, current reserve, creditor amount, and unfunded remainder.
  This **does not reserve** any insurer atoms.
- \`settle_distressed_closeout_unverified(txid, height, price)\`:
  atomically consumes the whole plan only when fully funded and under
  maintenance; not connected to signed L1 transactions.
- \`settle_distressed_closeout_fault_injected_unverified(..., leg)\`:
  deterministic test-only rollback probe.
- \`UnverifiedClearingEvent::DistressedCloseout\`:
  synthetic, terminal, end-of-block operation. Cannot coexist with
  another final Mark or additional events after it. The journal
  finalizes spot before closeout, preserving mark/spot order.
- \`UnverifiedClearingEvent::UnsupportedSettlement\` remains denied.
  Economic types 3, 23 and 31 remain default-deny in actual replay.

The mark and maturity still lack independent chain and oracle
authentication. The initial/maintenance policy is **not** a historical
TradeLayer consensus decision. This rule should never be reused for
partial liquidation, AMM priority, orderbook fills, real insurance
rights or portfolio cross-margin without separate protocol approval.

The original \`tradelayer.js@dca0888b\` contains separate asynchronous
\`Tally.updateBalance\`, \`MarginMap.simpleDeleverage\`, AMM price bounds,
\`PnlIou\` and \`insurance.js\` NeDB records. Its implementation is
useful for fault injection and historical differential testing but
does not justify assigning economic legitimacy to any observed bug.
No attempt is made to port those mutations verbatim here.

## Persistence and replay

Stage25E2A's **existing** TLCL/TLCC v2 format is sufficient to
persist this research transition's *resulting* spot/clearing/insurer
balances, flat positions, mark height and replay IDs. The TLCC pre-block
witness preserves the state **before** this closeout. A rollback restores
full insurer and margin funds and can replay a different caller-supplied
fork without crediting the old winner twice.

This does **not** encode a durable, independently authenticated
liquidation reason or proof-of-fill. A live protocol would need an
explicit rule-version and proof record in a separately reviewed new
checkpoint format. A valid CRC is neither authentication nor a solvency
certificate. Old TLSP/TLJP/TLJC remain permanently fenced from
insurance/clearing history.

## Required offline CI

\`\`\`sh
python3 scripts/check_rust_supply_chain.py
cargo fmt --all -- --check
rustfmt --check --edition 2021 src/clearing_distress_impl.rs
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen --test insured_distressed_closeout
cargo test --frozen
cargo build --all-targets --release --frozen
\`\`\`

The regression suite asserts loss/margin/insurer/creditor equality,
maintenance versus insolvency, insurer shortage and duplicate claims,
the exact order of multi-party funding legs, replay and stale epochs,
late fault rollback after every leg, TLCL v2 restore, TLCC v2 reorg
replacement, and whole-block failure if a late claim cannot be fully
funded. No external crates or real node/testnet transactions.

### Remaining economic work

There is **still no partial forced liquidation**, exchange fill,
seized-collateral distribution policy, insurance fee allocation,
bankruptcy/ADL or residual loss policy, contract settlement authorization,
authenticated mark or portable proof. Before moving this beyond
research-only:
1. derive contract policy and market prices from independently verified
   protocol input;
2. ratify versions and activation with historical replay divergence tests;
3. specify partial liquidation, fee/insurance priority and ADL handling;
4. formally check whole-state conservation, replay and liquidation
   rights and test fail-after-each-step under crash/reorg conditions.
