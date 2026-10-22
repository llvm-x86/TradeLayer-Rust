# TradeLayer JS -> Rust: staged migration

Historical reference: [patrickdugan/tradelayer.js](https://github.com/patrickdugan/tradelayer.js)

This repository is a prototype, not a deployable consensus node. Preserve the JavaScript implementation and its complete transaction history throughout the port. Work in small, reviewable steps.

## Step 0: Bootstrap (complete)

- Dependency-free Rust library with toolchain pinning, committed lockfile, and GitHub CI.
- Type-range wrapper for the 36 transaction IDs dispatched by the JavaScript implementation.
- Unit tests for type range and explicit unsupported values.

## Step 1: Characterize wire format (in progress)

- Completed: source-derived fixture generator for type 0 (activation), type 1 (issuance), type 2 (send), type 11 (managed grant), and type 12 (managed redemption).
- Completed: checked Rust ASCII envelope parser and preliminary typed field decoders.
- Completed: explicit regression cases for nested prefix, fractional-unit marker, and multi-whitelist discrepancies.
- Remaining: actual JavaScript `txEncoder` / `txDecoder` execution in isolation, accepted on-chain corpus, and cross-language differential outputs.

See [docs/WIRE_FORMAT.md](docs/WIRE_FORMAT.md).

## Step 1b: Propose corrected v1 wire encoding (this branch)

- A distinct binary `TLR` + version + numeric type-byte envelope, avoiding the historical base-36 selector and double-prefix bug.
- Exact atomic-unit type-2 send encoding and decoding, with address syntax and checked sizes.
- Deterministic source-independent v1 vectors in `fixtures/v1_vectors.tsv`.
- Legacy `wire` parser retained for offline diagnostics only, with no implicit fallback from v1.
- **Not activated on any network.** Consensus activation and chain-history migration must be settled before use.

See [docs/WIRE_V1.md](docs/WIRE_V1.md).

## Step 2: Exact quantities and ledger (prototype complete)

- Added dependency-free `Ledger` accounting primitives and typed account/property/tx identifiers.
- All amounts use checked `u128` atomic units. This is a **provisional internal representation**, pending reference-ledger and historical replay vectors.
- Implemented fixed/managed issue, authorized mint, issuer-only burn, and transfers over available balances.
- Added per-property supply conservation, duplicate transaction rejection, and all-or-nothing batches (clone-and-swap for now).
- Added deterministic property tests, overflow/rollback adversarial cases, and a draft binary send integration fixture.
- Remaining: reserved/margin/vesting/channel accounting, persistent transactional storage, cryptographic state commitments, reorg undo journals, state snapshots, and independent accepted-chain corpus.

See [docs/LEDGER.md](docs/LEDGER.md).

## Step 3: Limited state transitions

Port activation, fixed/managed issuance, and ordinary sends after the wire format is settled. Compare Rust results with confirmed JavaScript and chain-history fixtures. Isolate native tokens, vesting, policy restrictions, and procedural issuance until specified.

## Step 4: Integration

Add persistence, block ordering, reorg handling, and RPC as separate layers. Continue through channels, derivatives, clearing, and settlement using adversarial testing and review.

## Before any production deployment

A written protocol specification and independent consensus reference vectors must determine behavior. Require fuzzing, chain replay, multiple-implementation comparison, build reproducibility, review of dependencies and build scripts, and key-isolation testing.

## Step 2.5: Trusted block adapter and reversible checkpoints (prototype)

- `chain` module requires raw bytes through the `TrustedChainAdapter` boundary and rejects mismatches between authenticated origin and the ledger operation's actor.
- Atomic block application checks chain identity, contiguous height, parent hash, replay, and conservation. It retains bounded pre-block snapshots for rollback.
- Deterministic, CRC-protected ledger + chain checkpoint encoding can persist and restore the tip, replay set, and rollback history. A save to a new immutable path must succeed before staged state is published.
- Real UTXO/script/signature verification, block header validation, chain activation, state authentication, and full consensus replay remain future gates. See [docs/CHAIN_STATE.md](docs/CHAIN_STATE.md).

## Step 5: Read-only Bitcoin-family transaction inspection (in progress)

- Completed: bounded Bitcoin-style raw transaction parser, with canonical CompactSize checks and SegWit witness stripping for txid calculation.
- Completed: offline OP_RETURN payload inspection, separating read-only legacy ASCII diagnostics from the proposed v1 binary decoder.
- Completed: independently published raw Bitcoin transaction fixtures and explicitly labeled synthetic TL payloads; txids and SegWit wtxid checked byte-for-byte.
- Completed: CLI for local TSV replay and deterministic fail-closed malformed-input tests.
- Remaining: historically confirmed TradeLayer payloads from chain-indexed archival RPC, inclusion/UTXO ownership verification, consensus rules, and replay against the original JS node. The scanner does not satisfy the trusted-adapter contract.

See [docs/READ_ONLY_REPLAY.md](docs/READ_ONLY_REPLAY.md).

## Step 6: Node-backed Litecoin archive and differential decoder (prototype)

- Standard-library-only local Litecoin Core collector checks full-block
  transaction Merkle roots, confirmations, block-parent continuity, raw txids,
  exact output scripts, and final best-chain membership before emitting TSV and
  provenance manifest. The node supplies PoW/consensus verification.
- Bounded by default to 500 consecutive blocks, local RPC only, and explicit
  network selection. No wallet or transaction-broadcast methods.
- Added Rust legacy typed-field export and optional comparison against an
  actual trusted JavaScript `txDecoder.js` checkout. CI tests the integration
  with a deliberately synthetic decoder and avoids executing the legacy npm
  dependency graph.
- Remaining: acquire an unpruned archival node's actual observed TradeLayer
  bytes, verify sender/UTXO histories and signatures, and establish replay
  verdicts for live consensus activation. No real TradeLayer archival fixtures
  have been collected in this branch.

See [docs/ARCHIVE_REPLAY.md](docs/ARCHIVE_REPLAY.md).

## Step 7: zkTL Cairo commitment bridge (read-only)

- Pinned the 26-receipt [zkTL](https://github.com/patrickdugan/zkTL) sample at commit `edff311d7bfc71cd8b33bf6e2cf2bb2f282bec2d`.
- Python exports verified canonical JSON checkpoint metadata; Rust independently recomputes receipt IDs, ordered transition root and batch ID with SHA-256, tests state-root continuity and fail-closed receipt metadata.
- Added all-36-type migration catalog and explicitly rejected the ZK batch tx34 / legacy cross-layer-bridge tx34 semantic collision.
- **Not yet:** Cairo per-transition computation, cryptographic STWO proof verification, authenticated L1 inclusion, a shared Rust/zkTL state-root scheme or permission to mutate the ledger from the checkpoint.

See [docs/ZK_CAIRO_INTEGRATION.md](docs/ZK_CAIRO_INTEGRATION.md).

## Step 7: Authenticated registry execution (isolated prototype)

- Added `registry.rs` with typed, bounded state transitions for tx0,
  tx7–tx9 and tx13–tx15. State includes ordered activation records,
  clearlist ownership and attestations, oracle owners and last quotes,
  replay markers, and deterministic ID allocation.
- Each batch applies atomically with rollback-on-error, strictly
  monotonic heights, administrator checks, and activation only in
  blocks later than its activation block.
- Type 34 activation, token-admin migration, special list 0
  self-certification/banlist updates, and arbitrary xpub batches
  explicitly fail closed.
- The caller remains responsible for independently verifying
  signature authority, chain origin and transaction ordering.
  Registry and ledger replay sets are separate until a unified
  validated state machine is built.
- See `docs/REGISTRY_PORT.md` and
  `tests/registry_transitions.rs`. No new external Rust crates.

## Step 8: Unified transaction replay (simulation-grade)

- Added `execution::ProtocolState` coordinating registry actions and
  ledger issuance, send, managed grant and redemption as one staged state.
- Maintains a single replay set, chain namespace and monotonic height,
  while requiring activation in a strictly earlier block for balance actions.
- Atomic mixed batches roll back whitelist/oracle records, token balances,
  and replay markers together. Regression tests exercise fork cloning and
  cross-module duplicate IDs.
- Inputs remain explicitly `UnverifiedTx`; the signed-chain adapter,
  unified durable checkpoint, ZK proof verification and all other transaction
  implementations remain separate future steps.

See `docs/UNIFIED_EXECUTION.md`.

## Stage 9: Combined state checkpoint and reversible protocol blocks

- Canonically encode/decode the registry, full unified execution state, and
  chain snapshots with strict bounds, replay-set integrity checks, and
  immutable checkpoint saves.
- Restore registry activations, whitelists, attestations, and oracle quotes
  together with ledger balances, so rollbacks restore the same global state.
- Add a chain-neutral external `TrustedProtocolAdapter` seam. Tests only use
  fake adapter assertions; actual block, transaction, signature, UTXO, and
  authority verification remain unimplemented.
- Add adversarial tests for mid-block failures, fork replacement, checkpoint
  corruption, wrong chain/admin/anchor, and immutable checkpoint conflicts.

**Future:** versioned chain-authorization adapter, persistence compaction,
full historical replay, and independent cryptographic state root/proof
verification. [Design details](docs/UNIFIED_CHAIN_CHECKPOINTS.md).


## Stage 10: Channel escrow, transfers and reserved withdrawal settlement (prototype)

- Execute typed versions of legacy 4 (commit), 21 (withdraw request), and
  22 (channel-to-channel transfer) on the unified replay coordinator.
- Conserve supply across available balances, individual channel-column
  escrow, and reserved pending withdrawals using checked arithmetic.
- Enforce at most two distinct channel participants; reject unauthorized
  column moves and preserve owner identity on inter-channel transfers.
- Queue one-property requests to mature at H+7 and process those releases
  after each unified chain block, even a block with no TL transactions.
  Complete rollback restores the queue and amounts. Legacy JavaScript does
  not reserve at request time; this deliberate difference requires further
  differential testing before historical consensus parity.
- Preserve TLL v1 for channel-free states; read/write canonical TLL v2
  including channels and pending reserves through existing TLPS/TLUC
  checkpoints. Rust remains dependency-free.

**Blocked:** on-chain authorization, legacy column tie-breaking, whitelist
and payment-reference options, withdraw-all, clearing/PNL, native balances,
accepted-chain fixtures, and Cairo/STWO economic proof verification.

See [docs/CHANNEL_ESCROW.md](docs/CHANNEL_ESCROW.md).
