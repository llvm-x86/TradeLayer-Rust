# zkTL Cairo/STWO integration boundary

Stage 14 adds [read-only Rust state and TLPT transition commitments](PROVING_COMMITMENTS.md). They use a separate namespace and have not been reconciled with zkTL's roots; they cannot authorize economic state changes.

Stage 15 adds a [read-only root alignment inspector](TLPT_ZKTL_ALIGNMENT.md) for TLPT block statements, zkTL receipts and caller-supplied transaction observations. A matching report remains unverified as chain or economic evidence.

Source prototype: [patrickdugan/zkTL](https://github.com/patrickdugan/zkTL), specifically `src/state_checkpoint.js`, `src/hash_utils.js`, `src/zk_consensus_envelope.js`, `scripts/tlzk_stwo_batch_prover.sh` and `artifacts/cairo_inputs/batch_checkpoint_latest.json`.

## What is implemented

- `scripts/export_tlzk_batch.py` takes a committed zkTL batch JSON and checks the exact source `hashCanonical()` SHA-256 identifiers for each `receiptCore`, the ordered transition-root array, and the `batchCore` ID, along with receipt ordering, block bounds and state-root continuity. It requires valid, state-kernel-labeled receipts and refuses the contested transaction type 34.
- `tradelayer_core::zk_batch` independently recomputes the same canonical JSON digests in Rust, checks the field sequence, and rejects corrupted or audit-only candidate receipts. There are no Rust crate dependencies added.
- `tradelayer-zk-inspect` prints `stwo_proof=UNVERIFIED economic_validity=UNVERIFIED` on success. Its result is **metadata consistency**, not a trusted proof verdict.
- A source-derived 26-receipt fixture from `zkTL/artifacts/cairo_inputs/batch_checkpoint_latest.json` cross-tests Python and Rust without running zkTL's JavaScript. A 36-type catalog tracks the original protocol and known Cairo port coverage.

## Critical trust and compatibility gaps

1. **STWO proof scope.** The current zkTL Cairo program in `scripts/tlzk_stwo_batch_prover.sh` proves a small algebraic binding over `(batchId, transitionRoot, finalStateRoot, receiptCount, validReceiptCount)`. It does not execute every JavaScript state transition in Cairo. A `verifierResult.ok` boolean or SHA-256 of a proof file is not a proof verification. `wasm/tlzk_verifier` checks JSON/attestation consistency; this does not establish economic correctness.
2. **Transaction ID collision.** `zkTL/src/index.js` calls `tx34` a ZK batch move. `tradelayer.js/src/types.js` assigns type 34 to **Cross-Layer Bridge**, while type 32 is Batch Move ZK Rollup. The bridge rejects type 34; any eventual ZK execution requires a formally versioned opcode registry and activation rules.
3. **Cross-chain identity.** The zkTL examples use Bitcoin testnet4-style Raito inclusion, while the archival Rust collector scans Litecoin testnet3. `batchCore` does not commit to a trusted L1 chain ID, fork, inclusion block hash or confirmed tip. A future binding must make all of these explicit and independently verify them.
4. **State-root compatibility.** zkTL hashes Merkleized TradeLayer namespaces, whereas the Rust ledger currently serializes a smaller balance/property/replay-set model. Roots are **not interchangeable**. A reviewed state namespace mapping and a canonical root adapter are required before linking them.
5. **Authorization.** zkTL `supportLevel: state_kernel` is metadata from JavaScript execution. The Rust path cannot use that string as proof of signer authority, accepted chain history, or a valid Cairo/STWO proof.
6. **Full port.** The Rust ledger offers only isolated issuance/send/mint/burn primitives; historical consensus includes activation, reserved balances, channels, derivatives, settlement and network-specific validation. `src/protocol.rs` inventories every legacy ID and records the ZK JavaScript coverage without suggesting those transitions have been ported to Rust.

## Reproduce the fixture inspection

```sh
python3 scripts/export_tlzk_batch.py /path/to/zkTL/artifacts/cairo_inputs/batch_checkpoint_latest.json --out /tmp/tradelayer-zk-batch.tsv
cargo run --locked --bin tradelayer-zk-inspect -- /tmp/tradelayer-zk-batch.tsv
python3 -m unittest discover -s tests -p 'test_tlzk_export.py' -v
cargo test --locked
```

## Next code requirements

Port the actual Cairo transition constraints and typed witness commitments, pin the STWO verifier program hash and verify actual proof bytes against the public inputs, reconcile type 32/34, and independently bind network/genesis/height/UTXO inclusion to the snapshot's starting state. When that verifier and state-root adapter are independently tested, they may be wired behind the existing `TrustedChainAdapter` control boundary. This branch intentionally contains no API that can mutate ledger balances using zkTL metadata.
