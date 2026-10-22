# Stage 20: bounded spot-only block replay and durable checkpoints

This stage adds a **typed, unverified spot-only chain replay** that preserves
the complete type-5/6 orderbook, escrow ledger, fill history, replay markers,
and bounded rollback snapshots across checkpoint recovery. It extends the
isolated Stage 19 prototype without connecting it to live chain consensus.

## Why a separate chain lane

The existing `UnifiedChainState` owns a `ProtocolState` that already
contains one canonical ledger and global replay set. Stage 19's
`SpotExchange` owns a different `Ledger` because order placement reserves
tokens into private simulated escrow accounts. Simply embedding
`SpotExchange` next to `ProtocolState` would introduce **two spendable
balance authorities**. Replay markers for synthetic escrow transfers also
differ from real type-5/6 transaction IDs.

Stage 20 keeps these state machines separate. `SpotChainState` uses the
same contiguous-tip, immutable-file and bounded rollback principles as
`UnifiedChainState`, but is explicitly scoped to a **single spot ledger**
and a **pre-decoded, caller-provided block stream**.

A later versioned migration must establish one ledger, one global replay set,
a single ordered block executor and a complete co-checkpoint with the registry.
Until that exists, type-5/6 cannot be plugged directly into the ordinary
unified chain executor.

## Block execution semantics

`UnverifiedSpotBlock` carries a caller-supplied `ChainId`, `BlockHeader`
and a bounded sequence of typed `UnverifiedSpotOperation` objects:

- `PlaceType5` queues a new order for block-end processing. A full
  32-byte txid remains the order's replay and escrow identity.
- `CancelType6` cancels an existing *previously finalized* order and
  releases its outstanding reserved balance, with the source actor and
  property direction checked against the order.
- `Type3UtxoTrade` always returns
  `AuthenticatedUtxoProofRequired` because external LTC payment
  verification and fee/column semantics are absent.

A claimed transaction height must equal its containing block's height.
The coordinator checks chain namespace, strictly contiguous height,
parent hash and duplicate block ID, with a 10,000-operation upper bound
per block. All operations run on a cloned state. At block close,
queued type-5 orders are matched deterministically and reconciled against
per-property ledger conservation. An error anywhere leaves the original
tip, orders, fills, replay markers and balances unchanged.

A type-6 operation can cancel resting orders from earlier blocks.
A same-block type-5 placement is pending until the block closes and thus
cannot be canceled by a type-6 transaction in the same block in this
experimental policy.

Every successful block, including an empty one, produces a bounded prior
snapshot. `rollback_tip` restores the entire pre-block spot state,
which allows alternative forks to reuse rolled-back txids correctly.
The coordinator enforces the same 1–64 rollback depth policy as the
existing unified chain prototype. It makes no active-fork selection claim.

## TLSP v1: complete isolated spot state

`SpotExchange::snapshot_bytes` uses `TLSP\x01` and includes the complete
canonical ledger snapshot plus explicit policy IDs, queued orders, resting
books, full transaction IDs, replay markers, fills, next order sequence
and last processed height.

Loading `TLSP` requires an independently supplied token-policy set.
Malformed counts, unexpected policy IDs, duplicate/unsorted replay markers,
invalid pairs, stale queued heights, orphan or cross-property escrow
accounts, incorrect outstanding reserve amounts and noncanonical bytes
are rejected. The policy continues to exclude IDs 1–4 until native,
vesting and country-authorization semantics are specified.

The added ledger inspection method is internal and read-only. It rejects
any prefunded `~spot:` balance at initial exchange creation; after orders
are admitted, every prefixed escrow balance must correspond to exactly
one outstanding order with the exact same property and remaining amount.

## TLSC v1: spot chain and rollback history

`SpotChainState::checkpoint_bytes` uses `TLSC\x01` and stores its chain ID,
anchor, tip, rollback policy, retained pre-block tips, and one complete
`TLSP` blob for each retained state and the current state. Loading
requires the expected chain identity, anchor, rollback policy and permitted
token IDs, checks each snapshot's finalized height against its tip and
requires contiguous retained heights.

Checksummed nested encodings are re-encoded byte-for-byte after loading.
All snapshots share the existing 64 MiB maximum file size.

`write_new_checkpoint` creates a new immutable path with `create_new` and
`sync_all`. `apply_unverified_block_and_checkpoint` publishes the new
in-memory state only after the checkpoint write completes successfully.
A preexisting destination or write failure leaves in-memory state unchanged.
This is a prototype durability layer: directory fsync, multi-file atomic
publication and cryptographic state-root authentication remain future work.

## What the tests establish

`tests/spot_chain.rs` exercises a genuine pair of **synthetic token
properties**, with both assets conserved through partial fills,
unspent quote refunds, cancellation, checkpoint restoration, multi-block
rollback and an alternative-fork replay.

Adversarial tests reject fake parents, incorrect heights, wrong networks,
malicious type-3 operations, replayed transaction IDs, unauthorized
cancellation, invalid order sizes, failed middle operations, prefunded
reserved addresses, CRC corruption, forged inconsistent tip heights,
wrong policy/anchor/history and checkpoint file collisions.
A history-window test verifies old rollback frames are evicted by policy.

Commands:

```sh
cargo test --locked --test spot_chain
cargo test --locked --test spot_exchange
cargo fmt --all -- --check
rustfmt --check --edition 2021 src/spot_snapshot_impl.rs
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

## Missing consensus and protocol work

The current spot block envelope contains **pre-decoded assertions**. Its
block IDs, txids and actors are not derived from verified raw Litecoin
transactions and it cannot implement either chain's trusted adapters.
The prototype matches a proposed integer rational trading rule, which
differs materially from observed legacy `orderbook.js` pricing semantics.

For real Litecoin history, the next evidence step is to run the existing
`scripts/collect_litecoin_rpc.py` against an unpruned trusted Litecoin
Core node, acquire historically accepted type-5/6 raw bytes with actual
sender provenance and block transaction ordering, and compare their
outcomes against the legacy JS node. The existing archival collector
makes no policy decision authorizing transfers.

Activation, whitelists, native token handling, post/stop flags, price
cancellation, type-3 UTXO exchange, and proof-backed economic clearing
are still blocked. The default-deny settlement authorization gate in
`UnifiedChainState` remains unchanged.
