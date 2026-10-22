# Unified registry + ledger checkpoint and block adapter (experimental)

## Scope

The original `chain::ChainState` snapshots only the token ledger. This stage adds
`unified_chain::UnifiedChainState`, which snapshots `execution::ProtocolState`:
token balances, properties, registry activations, whitelist/attestation/oracle
records, and replay sets, with pre-block snapshots for bounded reorgs.

- `Registry::snapshot_bytes()` uses a canonical, bounded `TLRG\x01` binary representation.
- `ProtocolState::snapshot_bytes()` uses `TLPS\x01`, commits both registry and
  ledger encodings, and rejects a mismatch between global and component replay
  markers when restoring.
- `UnifiedChainState::checkpoint_bytes()` uses `TLUC\x01`, commits the independently
  supplied chain namespace, anchor, protocol admin, tip, full unified state,
  and bounded rollback history.
- All three formats use CRC32 **for accidental corruption detection only**. None
  is a cryptographic authenticated state root or Cairo/STWO proof.
- Immutable checkpoint file creation uses `create_new` and `sync_all` before
  publishing staged in-memory state. Retain earlier checkpoints as rollback
  options. The on-disk design still needs durable directory synchronization,
  compaction, disk-exhaustion handling, signed roots, and transactional storage
  for production.

## Authentication boundary

`TrustedProtocolAdapter` receives raw transaction bytes and block metadata and
returns `Option<AuthenticatedProtocolOperation>`. It **must** independently prove:

1. The header is valid for the configured network, including consensus rules,
   chain work and best-chain selection.
2. The raw transactions are members of the specified block in the original order.
3. Txid and the account/issuer controlling the spent input or verified authority
   are derived from validated signatures, scripts, and UTXO provenance.
4. A protocol operation is decoded from the same authenticated on-chain bytes,
   with version-specific activation and opcode semantics.

There is **no production adapter** here. The tests use a synthetic adapter.
`apply_block` calls the existing `apply_batch_unverified` only after the
external adapter makes these claims. That method name remains an explicit
reminder that this Rust code itself does not authenticate transactions.

Normal base-layer transactions may yield `None`: a block containing no relevant
TradeLayer transaction advances the tip without changing economic state.

## Correctness invariants

- A block's height must be exactly the current tip plus one, with the expected
  parent and network namespace.
- Entire block batches, including registry and ledger changes, commit atomically.
- Rejected blocks change neither the tip nor replay sets, activations, balances,
  oracle state, or attestations.
- Rollback restores the complete previous protocol snapshot and replay markers.
- Restore requires trusted expected chain ID, anchor, protocol admin, and
  retention policy rather than accepting those claims from the checkpoint file.
- Snapshot decoding validates field lengths, registry referential integrity,
  semantic bounds, duplicate keys, sorted canonical encoding and replay-set
  agreement across components.

## Remaining blockers

Full archival Litecoin/Bitcoin replay and a real authenticated protocol adapter,
chain-specific address and signature verification, UTXO ownership, activation
rules, full legacy opcode translation, native/vesting/channel balance support,
full deterministic state-root mapping to zkTL, cryptographic proof verification,
and economically complete Cairo transition constraints remain future work.

The `zkTL` batch binding is metadata consistency, not a proof that the economic
state transition follows from authenticated chain history. Version 34 remains
blocked pending reconciliation of cross-layer bridge versus ZK batch-move
semantics. This module does **not** accept a `zkTL` proof result as authorization
for moving balances.
