# Stage 19: isolated TradeLayer spot-book experiment (types 5 and 6)

This module implements a **bounded offline matching and escrow prototype**
for the subset of legacy type 5 (token-for-token order) and type 6
(token order cancellation) that can be specified without a signed
Litecoin/Bitcoin transaction or native-asset/country-policy ambiguity.

The reviewed JavaScript reference is
[tradelayer.js at dca0888](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c):
`src/logic.js`, `src/validity.js`, `src/orderbook.js`,
`src/txDecoder.js` and `src/txEncoder.js`.

**No historical consensus parity is claimed.** The legacy order matching
contains arithmetic and cancellation ambiguities that should be treated as
protocol design decisions before any activation or signed-chain replay.

## Legacy execution trace

| Source | Observed legacy behavior | Rust research subset |
| --- | --- | --- |
| `logic.js` type 5 | Queue by offered/desired property IDs and original txid | Canonical unordered pair; full 32-byte txid |
| `orderbook.js` block queue | Sort ready entries by height, sender's last three characters and txid | Same ASCII suffix comparator, canonical pair iteration |
| `orderbook.js` addTokenOrder | Move offered amount from available to reserved | One private, full-txid-derived simulated escrow per order |
| `orderbook.js` matchTokenOrders | Select best bid/ask, match maker price, mutate remaining amounts | Exact `u64` atomic lots and checked `u128` cross products |
| `logic.js` type 6 | Cancel all/by-txid/by-price and contract branches | Token cancel-all and exact original-txid only; contract and price modes rejected |
| `logic.js` type 3 | Consume an LTC UTXO payment to deliver committed channel tokens and apply fee/reward | Explicit `AuthenticatedUtxoProofRequired` rejection |

The Rust implementation uses immutable read access to an existing
`Ledger`, whose ownership moves into `SpotExchange`. Internal transfers
receive domain-separated synthetic IDs; offered amounts are moved from
available balances to order-specific escrow. Matching transfers both assets
atomically. Completed limit orders return any unused quote escrow to the
buyer. Cancellation releases precisely the currently unfilled offered
balance. The ledger's per-property supply conservation is checked again
after each batch, together with exact outstanding-order reserve amounts.

A complete block queue is staged on a clone and applied only after all
ready orders succeed. A later invalid order rejects the entire attempted
processing call without losing earlier queued entries or consuming any
additional escrow balance. Entry bounds are 4,096 queued plus open orders
and 16,384 recorded fills. Input replay history is also capped at 65,536 entries.

## Price unit discrepancy

In the legacy `orderbook.js`, `calculatePrice` is

```js
new BigNumber(amountOffered).dividedBy(amountExpected)
```

for **both** directions. The orderbook determines a sell order by
`offeredPropertyId < desiredPropertyId` and a buy order by the
reverse relation. For a pair `7-8`, a sell offer of **100 units of 7
for 200 units of 8** has a legacy calculated price of **0.5**, while
a buy offer of **220 units of 8 for 100 units of 7** has price **2.2**.
These ratios are measured in different quote units. Legacy matching
then tests `buy.price >= sell.price` and multiplies base units by
the selected trade price.

The Stage 19 safety policy expresses both directions consistently as
**quote units of property 8 per base unit of property 7**:

- Sell limit: `wanted_quote / offered_base`
- Buy limit: `offered_quote / wanted_base`
- A match requires `buy_offer * sell_offer >= sell_want * buy_want`
- The earlier block supplies maker price; for same-block crossing orders,
  the buy order supplies price while unsupported post/stop flags are rejected
- The executed quote amount is the *integer ceiling* of
  `filled_base * maker_quote / maker_base`
- Checked inequalities guarantee that this integer fill satisfies the
  original buy and sell limits; sub-atomic/dust violations reject the
  atomic processing call

The deterministic price policy is a **proposed semantic repair**, not
a silent claim that old JavaScript traders received those amounts.

`scripts/audit_spot_vectors.cjs` independently calculates 8 rational
matching scenarios with `BigInt`. The fixture includes the literal
legacy sell-side ratio in units of 1e-8 for comparison. Rust tests replay
both maker directions and verify atomic balances against those fixtures.

## More legacy issues exposed by the source audit

- The legacy order code truncates order IDs to `txid.slice(0,3) + txid.slice(-4)`.
  The prototype keys cancellation and escrow by all 32 bytes.
- `validateCancelOrder` rejects txid-based cancellation as deprecated,
  while `logic.js` dispatches it. The logic references
  `cancelTokenOrdersByTxid` but the implementation declares
  `cancelTokenOrderByTxid`. The Rust exact-ID cancellation is a
  safe **research subset** and must be tested against actual activation
  history before accepting this semantic change.
- Legacy self-match logic can drop an order while leaving unclear reserve
  release. This Rust model keeps both reservations until explicit cancellation.
- Legacy same-block queue sorting uses address-tail comparison and then
  `localeCompare`; this prototype permits ASCII account names only and
  uses a bytewise full-address fallback. More elaborate locale behavior is
  outside the research subset.
- Legacy active pair iteration follows its persisted `activePairs` order.
  Rust uses canonical sorted pair keys for deterministic processing.
- Legacy buy/sell price matching uses floating-point and BigNumber values,
  while this Rust prototype accepts exact `u64` atomic quantities only.

## Trust and integration boundary

The simulation allows a caller to designate a **permitted token ID set**.
IDs 1 to 4 are always refused, pending parity for native tokens, vesting,
restricted procedural supply and country attestations. The allowlist itself
is an *unverified caller choice*; this is not authentication or
whitelist-consensus enforcement.

No `TrustedProtocolAdapter` exists for these operations, and the new
module is **not wired to `ProtocolState`**. Legacy `tl` wire type-5/6
bytes are not interpreted by this module. It does not enforce actual type
activation heights, sender signatures, chain transaction order, witness/
UTXO ownership, orderbook persistence, reorg rollback or independent
reference-ledger consensus. Existing TLPS/TLUC snapshot formats omit the
spot orderbook, so a ledger-only checkpoint is insufficient to recover
this state safely.

No transaction or payout is authorized by the spot engine. The Stage 13
economic settlement gate remains default-deny. Type 3 requires independently
authenticated LTC payment outputs, channel column ownership and a properly
specified fee/reward rule before an executable mutation can be added.

## Test commands

```sh
cargo test --locked --test spot_exchange
node scripts/audit_spot_vectors.cjs --check
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
```

The next compatibility milestone is to collect actual confirmed Litecoin
TradeLayer type-5 and type-6 transactions, verify raw transaction parsing and
block provenance, and classify each discrepancy against this explicit
migration policy before enabling any historical replay.
