# Stage 22: One token ledger per independent TradeLayer replay

The Rust project now has an **opt-in, experimental joint executor** that
applies registry updates, token accounting and spot orders to exactly one
token ledger. Each Bitcoin, Litecoin and Dogecoin TradeLayer runs an
independent instance with its own genesis, block order, registry,
property supply, orderbook, and transaction replay history.

This is the Stage 22 single-ledger slice, **not yet the original
`UnifiedChainState` implementation**. The existing TLUC/TLPS and
isolated TLSC/TLSP APIs remain unchanged so historical experimental
checkpoints do not silently acquire new semantics.

## Ownership and executable surface

```text
JointChainState (one chosen TradeLayerMainnet)
  ├── pinned network genesis / local block tip / rollback history
  └── JointProtocolState
       ├── Registry: activation, whitelist, attestations, oracle records
       ├── SpotExchange: type-5 book and type-6 cancellation
       │    └── Ledger: ONE authority for all available and escrow balances
       ├── global_input_ids: actual simulated protocol txids
       └── ledger_input_ids: subset dispatched as ordinary token operations
```

Each block processes a bounded sequence of `UnverifiedJointTx` items
under one staged state clone. Type-0/7-9/13-15 registry operations use
the existing `Registry::apply` checks. Ordinary token types 1/2/11/12
call `SpotExchange::apply_joint_ledger_operation_unverified`, using the
*same* `Ledger` that type-5 reservations and market fills mutate.
Type-5 orders queue until block end and type-6 cancellations consume
existing resting orders. Activation must have occurred in an earlier block.

The new ordinary-operation entry rejects any actor, recipient or issuer
using the private `~spot:` escrow prefix, plus all channel, settlement
or other unsupported `Operation` variants. It checks the original
user transaction ID against the ledger replay set before committing.
Every ledger mutation is cloned and followed by a reserve reconciliation.

The block coordinator updates its global input replay set only after
each successful operation; a late failure aborts the full block and
leaves registry, balances, books, fills and replay markers unchanged.
That replay set separates natural input transaction IDs from the
domain-separated synthetic ledger transfer IDs used for spot reserves
and fills. The joint checkpoint validates disjoint input buckets
against registry, spot, and ordinary-ledger replay evidence.

## Deliberate exclusions

| Legacy type | Stage 22 handling |
|---|---|
| 0, 7-9, 13-15 | Registry prototype |
| 1, 2, 11, 12 | Shared-ledger issue/send/mint/redeem |
| 5, 6 | Shared-ledger spot orders and cancellation |
| 3 | Denied pending authenticated LTC/UTXO payment rules |
| 4, 21, 22 | Existing channel research prototype, outside joint executor |
| 23, 31 | Denied pending independent economic settlement proof |
| All other unported types | Explicit unsupported-operation denial |

The old isolated chain and spot prototypes remain available for
differential tests. Their ledgers must never be simultaneously used as
separate spending authorities for the same real tokens.

## New TLJP and TLJC formats

`TLJP\x01` is a canonical, bounded snapshot containing a network tag,
pinned genesis hash, complete `Registry` snapshot, complete `SpotExchange`
snapshot (including its single `Ledger` and escrow balances), globally
processed input txids, direct ledger input txids, and finalization height.
Loading requires an independently supplied expected network, registry
administrator, and permitted asset set. Decoding verifies consistent
replay buckets, checks orderbook reserve conservation, rejects
unsupported or malformed fields, and re-encodes canonically.

`TLJC\x01` is the chain wrapper. It additionally contains the network's
genesis-derived `ChainId`, configured anchor, tip, bounded rollback
policy, and a complete TLJP snapshot for every retained prior block.
Each checkpoint is limited to 64 MiB through the existing codec bounds.
Loading checks chain identity, tip continuity, expected configuration and
nested state heights before accepting a restart.

Both formats use CRC32 for accidental corruption detection. They provide
no cryptographic authentication against deliberate forgery. The Stage 14
TLPT root domain remains distinct, so a TLJC hash is never accepted as
a Cairo/STWO proof or independent best-chain certification.

As with earlier checkpoints, immutable `create_new` writes and file
`sync_all` precede publishing newly staged in-memory state. More
complete production durability will require directory sync, an
authenticated state commitment and crash/restart fault injection.

## Explicit Stage 20 spot migration

`JointChainState::migrate_from_spot_unverified` accepts an existing
`NetworkBoundSpot` instance and a separately reconstructed `Registry`.
The supplied chain, protocol administrator and registry history are
unverified until a real chain adapter confirms their provenance.

The migration keeps the spot ledger, outstanding orders, processed
spot input IDs and registry records. A new TLJC chain anchor is created
**at the imported tip**. All Stage 20 rollback frames are deliberately
left behind. Conflicting input IDs across registry and spot replay
histories cause rejection.

An explicit `JointChainState::migrate_from_unified_unverified` path
also imports an existing **profile-bound TLUC registry+ledger state**.
Its source must have a finalized tip, and the ledger must have no
remaining channel escrow, pending withdrawals, settlement claims or
receipts. Such obligations would otherwise stop progressing under the
joint executor's narrower economic instruction set.

This migration copies the old registry, ledger balances, supply and both
component replay marker sets into TLJP. It checks that their union matches
the original unified global input IDs. The new spot orderbook begins empty,
with a single copy of the imported token ledger. The new TLJC anchor is the
old tip; prior TLUC rollback frames are deliberately excluded.

Both import methods are **unverified research migrations**. A fully
authenticated production migration with historical chain provenance,
protocol rule parity, native/vesting restrictions and complete legacy
channel continuity remains outstanding. The original TLUC/TLSC loaders
continue using their former formats and cannot silently load TLJC files.

## Three independent chains and proving constraints

Bitcoin, Litecoin and Dogecoin remain separately named
`TradeLayerMainnet` values. Their protocol state histories cannot be
cross-loaded because TLJC commits to the selected mainnet tag, genesis
and derived ChainId.

The existing Bitcoin SHA256d/Raito-Cairo research path is Bitcoin-only.
Litecoin Scrypt and Dogecoin Scrypt/AuxPoW require their own node-backed
consensus evidence. The memory-hard ROMix shuffle keeps a corresponding
Cairo L1 verification path unavailable in this architecture.

No chain-specific trusted adapter is implemented by `JointChainState`.
Inputs are typed, caller-supplied operations. There is still no
authenticated raw Litecoin/Bitcoin/Dogecoin transaction and signer
binding, no canonical best-chain selection for Litecoin/Dogecoin, and
no complete economic Cairo/STWO proof verifier. The settlement policy
remains **default deny** for all three TradeLayers.

## Tests and next engineering milestone

```sh
cargo test --locked --test joint_chain
rustfmt --check --edition 2021 src/joint_snapshot_impl.rs
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Adversarial tests exercise shared available-vs-reserved balances,
partial spot fills and cancellation, registry activation boundaries,
ordinary token issuance, cross-component txid replay, self-rollback
and fork replacement, proof-denial for type 3 and settlements,
per-chain checkpoint identity, malformed snapshots, explicit Stage 20
migration and immutable-file collision safety.

The next compatibility milestone is authenticated Litecoin TradeLayer
historical replay with raw block order and source-derived actors,
followed by the same work in each separately deployed TradeLayer.
