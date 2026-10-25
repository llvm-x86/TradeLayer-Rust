# Stage 25G — Source-bound mark and channel-admission boundary

**Research-only; no liquidation authorization.** This is a read-only bridge between pinned JavaScript source characterization and a future independently verified blockchain/channel input adapter. No transition in this stage can debit a ledger, change mark policy, clear a position, issue tokens, or authorize E2B or Stage 25F settlement.

Baseline: [TradeLayer-Rust main 47fb6e26](https://github.com/patrickdugan/TradeLayer-Rust/commit/47fb6e263931e7e203f1c73447576fabcffadc55). Historical source: [tradelayer.js dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).

The earlier source characterization modules already describe AMM-first safe prices, maker order/price-time, weighted-average positions, half-up PnL and exact-rational bankruptcy thresholds. They are *not* an authorized liquidation engine.

## Source mark semantics

Pinned clearing.js:1081–1212 distinguishes:

- **Oracle:** an oracle-data entry matching the exact oracle ID and current block. It returns updated=false if the new price equals the cached last mark.
- **Native:** a volume-index price from the configured notional/collateral property pair (with legacy fallback to the contract ID document). A price for an earlier block is not a new current-block mark.
- If only cached history exists, no fresh mark exists. An ambiguous oracle-data query whose historical implementation chooses rows[0] does not establish a deterministic consensus ordering. The new audit rejects multiple same-height rows instead of inventing priority.
- **Funding references are distinct**: clearing.js:580–633 reads native 192-block VWAP and oracle 192-block TWAP. They may not impersonate fresh liquidation marks.

The new pure audit_current_block_mark_unverified method checks the asserted network, series, configured native pair or oracle ID, target height, positive price, nonzero oracle publication identifier, uniqueness, and change from previous mark. **The input expectation itself is a caller assertion**, not a verified contract registry or blockchain state. Its output is explicitly named UnverifiedFreshMark.

## Separate independent authorization obligations

| Layer | Pinned JS evidence | Missing independent verifier |
| --- | --- | --- |
| Chain | Indexed transactions and network identification | Actual BTC/LTC/DOGE consensus/header/fork and transaction inclusion, first-input/UTXO context |
| Oracle/native feed | validity.js:1353–1400 oracle administrator; clearing.js:1081–1212 block-specific price | Authenticated oracle administration and publication or deterministic volume-index reconstruction from authenticated trades |
| Type-4 commitment | validity.js:473–653 sender balance, token whitelist, optional commit clearlist and matching multisig address | Historical whitelist/activation state, sender and confirmed commitment/output at that exact block |
| Type-19 channel trade | validity.js:1804–1960 expiry, reference price/leverage, participants and at-least-one-member declared channel clearlist | Channel trade inclusion and state evolution, preserving type-19's *distinct* list rule |
| 2-of-2 channel authorization | channels.js:754–792 extracts pubkeys and reconstructs channel address | Both genuine signatures over the particular updated channel state or chain spend, exact script/sighash/revision/revocation and replay status |
| Economic fill | clearing.js:1937–1984,2232–2288,2343–2828 and orderbook.js:720–774 | Verified AMM-first safe executed fills, maker priority, margin/average-price account state, exact sat/30dp bankruptcy rounding, fees, waterfall and activation |

**Do not conflate commit and channel list checks.** The type-4 token/commit whitelist applies to the committing sender; the type-19 declared channel clearlist accepts at least one qualified party in the shown source. Historical membership must be bound to the event time. The presence of two pubkeys or a reconstructed channel address does NOT verify signatures on the updated off-chain state; a 2-of-2 witness or authenticated co-signed state transcript is still required.

The code does not accept assertions such as signed=true, adminVerified=true, whitelisted=true, or ammFilled=true as verification.

## Pure API and fail-closed behavior

- **audit_current_block_mark_unverified(expectation, records)** returns a private-field UnverifiedFreshMark, never a trusted mark.
- **audit_liquidation_claim_unverified(mark, channel_claim)** structurally checks matching network/contract/height, expiry, distinct non-reserved participants, nonempty claimed state reference and bounded declared clearlist IDs. It returns only UnverifiedLiquidationClaim.
- Every candidate permanently reports six MissingIndependentProof gates, and require_authenticated_liquidation() always returns AuthenticationUnavailable. There is no authenticated token issuer or settlement permit API.

Even a plausible synthetic mark/channel claim stays unverified. The tests reject wrong network, series, oracle ID, native index pair, funding TWAP/VWAP misclassification, missing/stale/future mark, duplicate current price, unchanged price, zero publication ID, invalid channel ownership/height/expiry/state reference, duplicated/unbounded clearlist claims and forbidden reserved actors.

## Scope and review

This module is not connected to ClearingSandbox, ClearingChainState, JointChainState or a trusted transaction dispatcher. Existing type 3/23/31 and forced liquidation remain default-deny. It adds no third-party crates, Cargo changes, wallets, scans, node requests, external agents or paid tools.

The next reviewable slice is a bounded independent source adapter using **actual pinned and independently verified offline chain/channel fixtures**, including network-specific header/transaction validation and authentic two-signature binding. Such fixtures and signature verifiers have not been supplied by this PR. CRC-validated TLCL/TLCC recovery cannot replace them.

Focused offline commands:

- python3 scripts/check_rust_supply_chain.py
- cargo fmt --all -- --check
- cargo clippy --all-targets --frozen -- -D warnings
- cargo test --frozen --test liquidation_input_gate
- cargo test --frozen
- cargo build --all-targets --release --frozen
