# Architecture and market decisions (debated, then decided)

**Status:** a decision record, not an activation. Nothing here changes consensus. Figures about other venues are from public sources dated 2026-10-28 and are cited in the PR that introduced this file; where I am unsure I say so.
Each question lists the strongest case *for* and *against* every option, the decision, and the **kill criteria**: the evidence that would reverse it.

## Q1. Where does matching and margin run?

| Option | Strongest case for | Strongest case against |
|---|---|---|
| A. Litecoin L1 only (every order is an L1 tx; this repo's current indexer model) | No bridge, no new validators, collateral stays native, smallest trusted base | 2.5-minute blocks and a script-limited UTXO chain cannot host a CLOB with liquidations; market makers cannot quote through 2.5-minute latency |
| B. Own L1 (Hyperliquid's route) | Full control of latency, fees, liquidation engine, token economics | Needs validators, a token, a bridge for collateral and a cold start for liquidity all at once. Bridges are the largest hack category. Hardest option for a team with no traffic yet |
| C. Sequenced app-chain: Rust state machine matches off L1, anchors state roots on L1, custody and forced exit on L1 | Fast matching; collateral stays on Bitcoin-family chains; reuses the channel escrow, state-commitment and DLC/BitVM work already in the org; the deterministic Rust ledger is exactly the execution layer | Single sequencer is a trust and liveness risk until forced exit and proofs exist; custody design is hard |
| D. Ethereum L2 / zk rollup (Lighter's route, as I understand it) | Deep stablecoin liquidity and tooling | Abandons the one thing TradeLayer has that others do not (Bitcoin-family-native assets and DLC oracles) and competes head-on with funded incumbents |

**Decision: C, staged.** Litecoin L1 remains settlement and custody only. The Rust state machine is the execution layer. Stages: (1) single operator sequencer, deposits capped, state roots anchored; (2) user-triggerable forced exit through the existing channel-withdrawal path; (3) fraud or validity proofs (`zk_batch`) and a multi-party sequencer set.
Why not A: the product physically cannot work. Why not B: bridge plus token plus cold start is three hard problems at once. Why not D: no differentiation.
**Kill criteria:** if forced exit cannot be made to work on Litecoin script within the stage-2 milestone, custody must move to a federated or BitVM-style design, and the venue must be described honestly as custodial until then.

## Q2. How does a new venue bootstrap liquidity?

The objection to answer: takers pay worse prices than the deep "mainline" venues, and makers who quote tight cannot arbitrage away risk, so the book never fills.

| Option | Strongest case for | Strongest case against |
|---|---|---|
| Taker and maker rebates for everyone | Fastest volume | Pays people to trade: self-trade farming is profitable whenever rebates exceed fees; volume vanishes when emissions stop; investors discount it |
| Fee-funded maker rebates only, capped emissions on top | Cannot go fee-negative; rewards liquidity not churn | Slower; needs a reason to show up at all |
| Oracle-priced pool (takers trade at index; LPs take the other side) | Removes "worse than mainline" for takers | LPs are adversely selected by latency traders; needs a hard OI cap and funding |
| List what the deep venues do not have | There is no mainline price to lose against, so the arbitrage objection disappears for those markets | Smaller addressable market; needs real demand for those assets |

**Decision:** lead with markets that have **no deep mainline venue** (TradeLayer-native tokens, LTC-denominated pairs, attested/clearlisted assets, pre-launch and oracle-settled contracts). In those, makers are not hedging against Binance, so the objection does not apply. For liquid majors, run a designated market-maker program with **maker rebates funded from fees** plus a **capped, time-limited, per-account-capped emission**, scored on resting liquidity that actually trades, never on raw volume. **Never rebate takers.**
Enforced in code: `src/fees.rs` rejects schedules where the rebate exceeds the taker fee; `src/rewards.rs` zeroes liquidity that never trades, caps per-account share, and caps fee-funded payout at the account's own fees paid.
**Kill criteria:** if, after one rewards season, more than half of rewarded volume comes from accounts that vanish when rewards end, stop the emission and keep only the fee-funded rebate.
**Caveat:** paying tokens for derivatives trading needs legal review before any of it is public.

## Q3. Compete with Stacks?

Stacks today (2026-10-28): TVL about $60M and falling, 30-day DEX volume about $107M with one venue at 98.5%, no live perps venue found, chain fees about $36K per 30 days. It is a general smart-contract chain; it is weak at trading.
**Decision:** do not compete as a general platform. Compete as a *trading venue* for Bitcoin-family assets. Stacks users and Bitcoin holders are a market, not a rival.
**Kill criteria:** if testnet cannot attract makers who quote at least 3 listed markets within 3 months, the wedge is wrong.

## What a $100M valuation requires

Not a decision, arithmetic: at a 4 bps take rate and an assumed 10x revenue multiple, about $2.1B of monthly volume; at 30x, about $0.7B (the multiple is an assumption, no perps-DEX multiple was sourced). Valuation follows measured volume, fees and users; none is measured today. See `docs/PRODUCT_MIGRATION.md` for what must be built first.

## Build order that follows from these decisions

1. Fee engine and reward accounting (done as standalone modules, not wired in).
2. Wire fees into the spot matcher with post-only and self-trade prevention (needs a protocol decision on whether fees are in consensus).
3. Sequencer state-root anchoring and forced exit (stage 1-2 of Q1).
4. Read-only RPC (in review), Scrypt block hashing for a Litecoin follower, payload builders (in review).
5. Perps: margin and liquidation exist only as research modules; they gate on step 3.
