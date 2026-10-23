# TradeLayer JavaScript → Rust full-tree migration inventory

**Pin:** `tradelayer.js` `dca0888b`; Rust candidate PR #29 `c5cdd450`. **Coverage:** 315 tracked JS repository files, 50 `src/` files (49 modules plus experimental README), and the separate server entry point.

This is a file-presence and implementation-scope audit. 21 `src/` files have partial Rust analogues; 4 have only read-only evidence tooling; 24 are unported; the remaining `src` entry is an experimental README. None has verified mainnet consensus parity. Tests, utilities, fixtures, bundled binaries and docs have distinct migration roles.

## Source-module matrix

| Original file | Status | Rust analogue | Primary missing fidelity |
|---|---|---|---|
| `src/activation.js` | Partial Rust analogue | `src/registry.rs + src/joint.rs` | Activation rules differ same-block; prototype or subset, historical mainnet parity unverified |
| `src/adjuster.js` | Unported | `—` | Adjustment engine |
| `src/amm.js` | Unported | `—` | AMM liquidity/economics |
| `src/base256.js` | Partial Rust analogue | `src/wire.rs + src/wire_v1.rs` | Format-specific prototype; prototype or subset, historical mainnet parity unverified |
| `src/base94.js` | Partial Rust analogue | `src/wire.rs + src/wire_v1.rs` | Format-specific prototype; prototype or subset, historical mainnet parity unverified |
| `src/bitvmBundle.js` | Unported | `—` | BitVM integrations |
| `src/bitvmCache.js` | Unported | `—` | BitVM integrations |
| `src/bitvmRisk.js` | Unported | `—` | BitVM risk |
| `src/channels.js` | Partial Rust analogue | `src/ledger_channels_impl.rs + src/unified_chain.rs` | Escrow and withdrawals subset only; absent joint executor; prototype or subset, historical mainnet parity unverified |
| `src/clearing.js` | Partial Rust analogue | `src/clearing_math.rs` | Pure loss coverage, integer funding deadband and mark-to-mark PnL only; full clearing and settlement remain unported |
| `src/clearlist.js` | Partial Rust analogue | `src/registry.rs` | Restricted registry whitelist/attestation subset; prototype or subset, historical mainnet parity unverified |
| `src/client.js` | Evidence tooling only | `src/litecoin_evidence.rs + scripts/litecoin_node_inspect.py` | RPC inspection only, no node client port |
| `src/consensus.js` | Partial Rust analogue | `src/chain.rs + src/joint_chain.rs + src/state_commitment.rs` | Simulated transitions, history and checkpoints; authentic historical consensus outstanding; prototype or subset, hi... |
| `src/contractRegistry.js` | Unported | `—` | Derivatives contract catalog |
| `src/db.js` | Partial Rust analogue | `src/codec.rs + src/joint_snapshot_impl.rs` | Checkpoint formats, no production database replacement; prototype or subset, historical mainnet parity unverified |
| `src/dlcOracleBridge.js` | Unported | `—` | DLC oracle bridge |
| `src/experimental/binohash/README.md` | Research documentation | `—` | Experimental Binohash source; keep distinct from consensus authorization |
| `src/experimental/binohash/binohashAdapter.js` | Unported | `—` | Experimental adapter |
| `src/init.js` | Unported | `—` | Initialization |
| `src/insurance.js` | Unported | `—` | Insurance funds |
| `src/interface.js` | Unported | `—` | Client interface |
| `src/interfaceExpress.js` | Unported | `—` | HTTP interface |
| `src/iou.js` | Unported | `—` | IOU accounting |
| `src/issuanceIntent.js` | Unported | `—` | Issuance intent |
| `src/listener.js` | Unported | `—` | Block/network listener |
| `src/logic.js` | Partial Rust analogue | `src/joint.rs + src/execution.rs + src/ledger.rs + src/registry.rs` | Only typed registry, balances and spot subsets; prototype or subset, historical mainnet parity unverified |
| `src/main.js` | Partial Rust analogue | `src/joint_chain.rs + src/joint.rs + src/litecoin_evidence.rs` | Full runtime/indexer consensus sequencing absent; prototype or subset, historical mainnet parity unverified |
| `src/marginMap.js` | Unported | `—` | Derivatives margin and liquidation |
| `src/options.js` | Unported | `—` | Options economics |
| `src/oracle.js` | Partial Rust analogue | `src/registry.rs` | Oracle registration and reporting subset; prototype or subset, historical mainnet parity unverified |
| `src/orderbook.js` | Partial Rust analogue | `src/spot.rs + src/spot_chain.rs` | Spot type 5/6 subset; derivatives book unported; prototype or subset, historical mainnet parity unverified |
| `src/persistence.js` | Partial Rust analogue | `src/joint_snapshot_impl.rs + src/codec.rs` | Experimental canonical checkpoints, no historical database parity; prototype or subset, historical mainnet parity u... |
| `src/procedural.js` | Unported | `—` | Procedural token logic |
| `src/property.js` | Partial Rust analogue | `src/ledger.rs + src/registry.rs` | Fixed/managed issuance subset; prototype or subset, historical mainnet parity unverified |
| `src/reOrg.js` | Partial Rust analogue | `src/joint_chain.rs + src/unified_chain.rs` | Synthetic reversals only; prototype or subset, historical mainnet parity unverified |
| `src/scaling.js` | Unported | `—` | Scaling and settlement |
| `src/stateOracle.js` | Evidence tooling only | `src/state_commitment.rs + src/zk_batch.rs` | Read-only proof/commitment audit, not equivalent state oracle |
| `src/tally.js` | Partial Rust analogue | `src/ledger.rs + src/joint.rs` | Balances and supply prototype, exclusions for restrictions/vesting/margin; prototype or subset, historical mainnet ... |
| `src/tradeHistoryManager.js` | Partial Rust analogue | `src/spot.rs + src/spot_snapshot_impl.rs` | Spot fills recorded; complete history/index APIs absent; prototype or subset, historical mainnet parity unverified |
| `src/txDecoder.js` | Partial Rust analogue | `src/wire.rs + src/read_only.rs + src/wire_v1.rs` | Limited historical typed decoder and diagnostics; prototype or subset, historical mainnet parity unverified |
| `src/txEncoder.js` | Partial Rust analogue | `src/wire_v1.rs + src/wire.rs` | New proposal and limited legacy vectors, not historical encoder parity; prototype or subset, historical mainnet par... |
| `src/txIndex.js` | Evidence tooling only | `scripts/legacy_js_replay.cjs + scripts/collect_litecoin_rpc.py` | Read-only index comparisons, no full production indexer |
| `src/txUtils.js` | Evidence tooling only | `scripts/litecoin_source_trace.py + src/litecoin_evidence.rs` | First-input address provenance only, not full RPC/wallet utility |
| `src/types.js` | Partial Rust analogue | `src/protocol.rs + src/joint.rs + src/wire.rs` | Recognized 36 types but small executed subset; prototype or subset, historical mainnet parity unverified |
| `src/validity.js` | Partial Rust analogue | `src/registry.rs + src/joint.rs + src/ledger.rs + src/spot.rs` | Most validity handlers still unported; JS replay oracle is diagnostic; prototype or subset, historical mainnet pari... |
| `src/vaults.js` | Unported | `—` | Vault primitives |
| `src/vesting.js` | Unported | `—` | Vesting lifecycle |
| `src/volumeIndex.js` | Unported | `—` | Volume/reward index |
| `src/walletInterface.js` | Unported | `—` | Wallet API |
| `src/walletListener.js` | Unported | `—` | Wallet monitoring |

## Key blockers

- **Evidence:** mainnet Litecoin archival replay still needs genuine chain-backed candidates and a validated prior state. PR #29 has synthetic tests and read-only provenance tooling.
- **Forks in semantics:** JS catch-up transaction ordering and same-block activation can differ from Stage 22 Rust. This must be adjudicated before spending authority.
- **Economic functionality:** `clearing.js`, `marginMap.js`, `contractRegistry.js`, options, vesting, and AMM are mostly unported. `orderbook.js` has a narrower Rust spot prototype.
- **Type coverage:** Rust catalogues all 36 legacy type IDs. The Stage 22 joint executor accepts 13 types in synthetic simulation; type 3 and settlement types 23/31 remain denied.
- **Security review:** `server/litecoind.exe` is checked-in binary; `tests/sensitiveConsoleOutput.txt` is an uninspected potentially sensitive fixture. Neither should be copied into generated test data without review.

## Tree sections

| Group | Files | Recommended disposition |
|---|---:|---|
| Source | 50 | Port or verify core state/economics |
| Tests | 111 | Translate runnable assertions into independent Rust fixtures |
| Archived tests | 60 | Review historical relevance before translation |
| Utilities | 57 | Separate wallet/admin operations from consensus |
| BitVM3 | 21 | Independent proving research |
| Service bundle | 2 | Inspect API and pinned binary provenance |
| Documentation | 4 | Retain specifications |
| Build / configuration | 5 | Build dependency audit |
| Assets / fixtures | 5 | Preserve evidence provenance |

## Source references and audit boundaries

- JS pinned tree: https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c
- Rust candidate: https://github.com/patrickdugan/TradeLayer-Rust/tree/c5cdd450844730dc2804ef1fa076355acfec9b1e
- Stage 23 draft: https://github.com/patrickdugan/TradeLayer-Rust/pull/29
- Tracking issue: https://github.com/patrickdugan/TradeLayer-Rust/issues/28

All 315 paths and Git tree byte sizes were individually recorded. Core mappings follow the curated prior audit and Rust documentation. Utility and test statuses are mainly based on filename and directory roles, labelled in the companion CSV. Those rows need source review before assigning exact 1:1 parity.

## Stage 24A addendum (23 October 2026)

[PR #32](https://github.com/patrickdugan/TradeLayer-Rust/pull/32)
introduced the first isolated numeric slice from legacy `src/clearing.js`,
merged at `2811dcca`. Its new `src/clearing_math.rs` tests option-adjusted
loss coverage, funding basis-point deadband, and bounded linear/inverse PnL
with deliberate legacy 20dp-vs-exact reciprocal rounding modes. All 15
vectors are synthetic, independently calculated with Node BigInt. Existing
settlement and liquidation remain inaccessible from this module.

The core-source status count is now **21 partial / 4 evidence-only /
24 unported / 1 non-code README**. Other rows retain the original Stage
23B/C assessment commit for reproducibility; the clearing row identifies
the newer Stage 24A merge commit.
