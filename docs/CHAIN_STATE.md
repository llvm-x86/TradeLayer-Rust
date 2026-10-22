# Authenticated block boundary, rollback and checkpoint format

**Status:** local projection prototype. **Not a Bitcoin, Litecoin, or Dogecoin consensus node.** Never attach this crate to wallets or allow it to update mainnet balances without a separately audited chain adapter, activation rules, historical replay, and a reorg policy.

## Trusted input boundary

`CandidateBlock` contains a chain namespace, header identifiers, and **raw** transaction bytes. It contains no claimed account owner or predecoded ledger instruction. `TrustedChainAdapter` must implement:

- `verify_block`: check the canonical block identifier, linkage, proof-of-work and chain-specific header rules, and any UTXO/transaction inclusion requirements.
- `authenticate_and_decode`: calculate the transaction ID from verified bytes, determine the authorized account (including proper Bitcoin/Litecoin/Dogecoin input ownership and protocol sender-selection rules), and decode a permitted TradeLayer operation **from those same bytes**.

The adapter is part of the trusted computing base and currently has *no production implementation*. Tests use a deliberately synthetic adapter. A malformed or permissive implementation defeats the security boundary even though the Rust ledger's own arithmetic is correct.

The engine checks that the authenticated origin equals the issuer/from actor in the returned operation. It enforces chain namespace, consecutive height, parent hash, replay protection, and a bounded number/size of input transactions. A rejected block preserves balances, supply, tip, history, and replay markers together.

Additional required consensus rules include transaction activation by chain and height, property-ID allocation, native and vesting property handling, whitelist/attestation policy, transaction-order constraints, signing requirements, valid canonical destination addresses, and deterministic rounding.

## Rollback

Each accepted block records a complete pre-block ledger and tip in memory. `rollback_tip(expected_tip)` requires a match against the current tip and restores replay markers along with token balances. An engine-created checkpoint preserves the bounded rollback history across restarts. A rollback past the retained history fails, requiring a full trusted replay from an older checkpoint.

The current implementation clones the full ledger per block, which is intentionally simple and expensive. Replace this with journaled state changes and a crash-consistent database before indexing historical blockchains at scale.

## On-disk checkpoint

`checkpoint_bytes()` emits a deterministic, bounded binary structure with magic/version `TLCS\x01`, chain ID, immutable anchor, current tip, rollback policy/history, and length-prefixed canonical ledger snapshots. Ledger blobs use `TLL\x01`, sorted property/balance/replay-marker records, and conservation verification. Both formats end in CRC32 checksums.

CRC32 detects accidental data corruption. **It does not authenticate the checkpoint.** A malicious actor can rewrite the file and recompute the checksum. Production requires a trusted state commitment, verified chain provenance, safe storage access, and rollback-resistant checkpoint discovery.

`write_new_checkpoint(path)` creates an **immutable** new file and syncs file content. It never overwrites an existing checkpoint. `apply_block_and_checkpoint` stages the next state, saves it, then updates the in-memory state. A failed save leaves the engine's in-memory state untouched. The caller owns selection, directory sync policies, checkpoint retention, and crash recovery across complete files. Platform-specific crash persistence is not fully established.

`from_checkpoint_bytes` and `load_checkpoint` require expected chain, anchor, and history limit **independently** of the checkpoint itself, verify record order and supply totals, reject bad checksums/truncation/trailing bytes, and reconstruct reorg history.

## Tests and next work

The test suite checks actor binding for issue/transfer/mint/burn, rejected/unverified blocks, wrong parent/height/chain, multi-transaction atomicity, duplicate IDs, fork replacement, bounded rollback, zero-supply managed properties, malformed snapshots, filesystem immutability, and restart recovery.

Follow-up: implement a read-only chain adapter with vetted Bitcoin crates and test vectors. Test transaction-origin extraction, canonical OP_RETURN decoding, chain-specific activation heights, state hashes and historical ledger replay before supporting signed inputs from real networks.
