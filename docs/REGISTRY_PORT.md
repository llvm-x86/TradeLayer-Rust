# Stage 7: Registry transitions (Rust prototype)

Source comparison: [`tradelayer.js`](https://github.com/patrickdugan/tradelayer.js) `src/activation.js`, `src/logic.js`, `src/validity.js`, `src/clearlist.js`, `src/oracle.js`; and [`zkTL`](https://github.com/patrickdugan/zkTL) `docs/PROTOCOL_SWEEP.md`, `src/index.js`.

## Included transactions

| Legacy tx type | Implemented Rust registry primitive | Unported behavior |
| --- | --- | --- |
| 0 | Admin-gated activation with explicit activation height and code hash | Genesis native tokens, persistent activation consensus hashes, network-specific admin discovery |
| 7 | Create whitelist with deterministic numeric ID, admin and backup | On-chain address validation, exact legacy URL storage and `ClearList` schema |
| 8 | Rotate whitelist or oracle primary/backup with explicit ownership | **Token issuer/admin rotation** requires atomic mutation of the ledger and remains rejected |
| 9 | Admin-gated attestation and revocation keyed by (list ID, subject) | List 0 self-certification, banlist updates, xpub/merkle batch attestation, reference-output resolution |
| 13 | Create oracle with monotonically allocated ID, admin, backup, ticker, and lag | Registry-specific oracle subtype support and cross-chain source verification |
| 14 | Publish positive, checked integer price in **1e-4** units; optional OHLC | Legacy market-index side effects, quote attestation, proof of data origin |
| 15 | Admin-only irreversible oracle closure | Backward compatibility with historic oracle event log |

`RegistryAction::tx_type()` maps actions to their historical opcode. Unimplemented opcodes remain unsupported at this layer. Type **34** is deliberately refused in the activation registry because `zkTL` calls it a ZK batch movement while `tradelayer.js` calls it a cross-layer bridge. Protocol type 32 remains unresolved for cryptographic ZK execution.

## Semantic changes requiring governance / migration review

- A transaction type activates **after** its recorded activation block, matching the intent of JavaScript `isActivated()`'s strict `block > activationBlock` check. Duplicate activation is rejected. The legacy code may overwrite activation metadata on reactivation, which should not be silently inherited.
- Whitelist and oracle IDs start at 1, increase monotonically, and reject overflow. Legacy IDs come from datastore contents and must be mapped during historical replay.
- Whitelist backup addresses can rotate the administrator, but the primary administrator alone can grant/revoke attestations. The legacy validator applies a similar division, with inconsistent `admin`/`adminAddress` record shapes.
- List 0 self-certification and banlist updates are **explicitly rejected** pending policy design. `Registry` does not interpret arbitrary metadata as country attestations.
- Oracle prices are **integers scaled by 10,000**, avoiding JavaScript `Number` divergence. This is an internal model, not an on-chain wire-specification commitment.
- Registry transactions use a global replay set for this module. Ledger transactions have a distinct replay set. These sets must be unified in a complete block application engine.

## Security boundaries

`RegistryTx.actor`, `RegistryTx.txid`, and `RegistryTx.height` are assertions from the caller. The Rust registry **cannot authenticate a signature or verify Litecoin/Bitcoin block inclusion**. It must eventually receive them from a chain-specific trusted adapter which verifies inputs, origin, block ordering, and the wire payload. Current tests use synthetic identities.

`Registry::apply_batch` clones the state, applies caller-ordered transactions, and commits only if all succeed. A failed transaction does not consume replay IDs or allocate resource IDs. A cloned prior state can be used for controlled rollback tests, but checkpoint serialization and production reorg journals for registry state remain to be implemented.

This code **does not verify Cairo/STWO proofs**. `zkTL`'s existing Cairo batch-binding test proves an algebraic binding over public metadata rather than full per-transaction economic validity. A trusted proof verifier, program hash allowlist, authenticated chain inclusion and a namespace-root adapter remain necessary before a ZK result can mutate ledger or registry state.

## Run tests

```sh
cargo test --locked --test registry_transitions
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
```

The new module adds no third-party crate dependencies.
