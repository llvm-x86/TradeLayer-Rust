# Stage 8: One replay domain for registry and token balances

Source basis: [legacy TradeLayer.js](https://github.com/patrickdugan/tradelayer.js) and the [zkTL kernel inventory](https://github.com/patrickdugan/zkTL/blob/main/docs/PROTOCOL_SWEEP.md). The new `execution` module is a **simulation-grade, unverified coordinator** for transactions 0, 1, 2, 7–9, 11–15. Its purpose is to demonstrate deterministic state transitions, atomicity, and shared replay semantics while the chain-specific verification surface remains unfinished.

## Operation mapping

| Transaction type | ProtocolOperation / RegistryAction | State effect |
| --- | --- | --- |
| 0 | Registry::Activate | Admin-authorized opcode activation records |
| 1 | Issue | Create fixed/managed token and issuer balance |
| 2 | Send | Transfer an exact amount of atomic units |
| 7–9 | Registry::CreateWhitelist / UpdateAdmin / Attest / Revoke | Whitelist and attestation state |
| 11 | Grant | Managed-token mint to a specified account |
| 12 | Redeem | Burn managed supply held by the issuer |
| 13–15 | Registry::CreateOracle / PublishOracle / CloseOracle | Oracle lifecycle and exact prices |

The coordinator runs `ProtocolState::apply_batch_unverified` with caller-ordered `UnverifiedTx` inputs and clones its combined state before application. Every operation checks the global transaction-ID replay set, chain namespace, and monotonic block height. For token actions, it requires the corresponding opcode to have been activated in a **strictly earlier** block; registry actions apply their own activation and authority rules. Each underlying ledger or registry retains its own local replay markers, too. A batch failure restores all three replay sets along with balances and registry records.

## What remains blocked

- All incoming transaction metadata is **unverified**. The claimed actor, transaction ID, height, chain ID, operation, and property ID require an independent chain parser and signature/UTXO-origin verification. A protocol-level account ID alone does not authenticate a signer.
- The fixed/managed token ID for issuance is supplied by the caller, while historical TradeLayer.js creates IDs from persisted property state. The actual chain mapping must be specified and checked against historical transactions.
- The global state has only clone-based rollback at this stage; the previous `ChainState` checkpoint format persists ledger-only state. A new versioned checkpoint must commit to both ledger and registry, ordered activations, and one replay domain. `ProtocolState` is intentionally isolated from the production chain adapter.
- The historical version-1 protocol's binary `TLR` wire format is a **proposal**. The coordinator consumes typed operations, and does not infer them directly from OP_RETURN payloads.
- Native token genesis, vesting/reserved/margin/channel accounting, sell-side fee handling, AML/country-list rules, token-admin rotation, derivatives, and ZK settlement remain outside the executable slice.
- zkTL's state roots span over 30 different namespaces and do not match the Rust ledger/registry snapshot format. The existing Cairo/STWO prover binds public batch metadata rather than establishing full economic state-transition validity. Proof claims must not be used to authorize ledger mutation.
- tx34 remains disallowed as an activation because legacy TradeLayer assigns it to Cross-Layer Bridge while zkTL uses it for ZK Batch Move. Type 32 requires a separate, versioned rollup/settlement specification.

## Verification

`cargo test --locked --test protocol_execution` checks activation, issuance, transfers, managed mint/burn, global replay protection, shared rollback, forked snapshots, cross-chain rejection and type34 collision. These synthetic tests establish internal determinism, not live chain correctness.

This stage adds no external dependencies, networking code, wallet access, or arbitrary execution.
