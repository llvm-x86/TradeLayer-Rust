# Stage 25D: TLCL/TLCC complete clearing recovery (offline research)

The old TLSP/TLSC/TLJP/TLJC checkpoint encodings cannot serialize clearing
margin-escrow owners, derivative positions, immutable collateral and mark
policy, or clearing replay markers. Even a **fully closed and refunded**
position may carry nonzero historical replay obligations. Reusing a spot-only
checkpoint would resurrect spent operations or strand token collateral.

Stage 25D introduces **new format namespaces** rather than weakening old
snapshots or pretending a valid CRC authenticates a blockchain header.

| Format | Purpose | Public trust status |
| --- | --- | --- |
| \`TLCL\\x01\` | One complete \`ClearingSandbox\`, including its single SpotExchange-owned Ledger and all spot/margin/replay/policy state | Unverified, offline, CRC only |
| \`TLCC\\x01\` | One \`ClearingChainState\`, network/contract/anchor/tip, full TLCL present state and bounded complete TLCL prior-state witnesses | Unverified, offline, CRC only |
| \`TLSP\\x01\`, \`TLSC\\x01\`, \`TLJP\\x01\`, \`TLJC\\x01\` | Existing legacy spot-only and registry+spot lanes | **Still reject all active or historical clearing activity** |

## Ownership

The checkpoint serializes the **sole** economic ledger nested in
\`SpotExchange\`, not a copied clearing balance table. Every owner's
\`~clrm:\` address is derived from network genesis, contract, collateral
and owner, and is separately serialized and validated against its actual
ledger balance. \`~spot:\` escrows must match the stored resting orders.
The clearing position map holds signed lots but cannot itself mint atoms.

\`TLCL\` records, in canonical order:

1. Version magic, one mainnet tag, pinned genesis and genesis-derived
   chain identity, contract ID, collateral property ID.
2. Optional immutable fixed synthetic initial/maintenance collateral
   requirements. Nonzero and maintenance <= initial are mandatory.
3. Optional mark terms: latest height and price, nonzero notional, linear
   or inverse setting, and the *explicit* Legacy20DecimalPlaces versus
   ExactRational policy.
4. Permanent \`clearing_activity\` flag, sorted owner → escrow records,
   sorted nonzero signed position records, and strictly sorted clearing
   transaction IDs.
5. The entire nested TLSP-shaped payload: one ledger, policy-allowed
   properties, queued/resting spot orders, fills, replay IDs and last
   finalized spot height; each nested payload retains its own CRC.
6. Outer CRC, maximum 64 MiB total.

The legacy TLSP writer/reader are **unchanged in authority**: they
continue to refuse any state with clearing history or a foreign escrow.
Only crate-private combined snapshot methods permit the inner payload
to carry \`~clrm:\` accounts. The *outer* TLCL loader checks all escrow
derivations, replay-ID uniqueness across all lanes, position sum zero,
actual collateral floors, spot book reserves and token-supply conservation.
It rejects duplicate/unsorted owner or transaction records and re-encodes
every recovered byte to enforce canonical representation.

The restored \`mark_policy\` must agree with independently caller-supplied
**notional, inverse and rounding terms**, and the stored collateral policy
must exactly match independent expectations. A valid checksum never grants
authority to select these policy parameters.

## Block journal and crash recovery

\`ClearingChainState\` is an *unverified research executor*:
\`UnverifiedClearingBlock\` contains caller-chosen headers and typed events,
not verified Litecoin/Bitcoin/Dogecoin transaction bytes. A header ID or
parent field is just a value supplied by the test harness.

The bounded block application routine requires an exact single-height
increment, matching parent ID, distinct observed block IDs, matching
mainnet and <= 4,096 events. Events may include ordinary token operations,
synthetic spot placements, margin deposits/withdrawals, matched derivative
lots, and explicitly requested funding. Unsupported economic settlement
always rejects. An optional mark must occur **last**, after the spot
book's end-of-block finalization, and may not bypass due queued orders.

Each block first executes entirely on a cloned \`ClearingSandbox\`.
The previous *whole* state, including replay IDs, is kept in a bounded
deque (maximum 64 recent complete states). Any later failed transition
or invalid mark leaves the original chain object byte-for-byte unchanged.
Successful rollback restores the complete previous object. An alternate
caller-asserted fork can then be replayed, but this does not establish
which branch is real, nor retain history beyond the configured limit.

\`TLCC\` persists a network/contract/collateral-bound anchor and tip,
history limit, consecutive rollback witnesses and the current state.
On restart it checks every witness's spot finalization height against the
recorded tip, rejects gaps, wrong anchors, wrong network identity,
unexpected policy and noncanonical bytes, and refuses deep rollbacks that
have no retained prior witness.

\`apply_unverified_block_and_checkpoint\` applies to a **clone**, then
writes an immutable file via \`create_new\`, \`write_all\` and \`sync_all\`,
and only then publishes the in-memory state. A failed or colliding path
never mutates balances, mark policy, queued orders, margin escrow ownership
or replay IDs. Files are never overwritten. The caller still must
establish directory durability, storage-media integrity, backup rotation
and independent source authenticity before considering production.

### Security and known limitations

- A CRC is an error-detection checksum, **not** a signature, Merkle proof,
  authenticated consensus checkpoint or authority to release collateral.
- Both \`TLCL\` and \`TLCC\` are new, versioned, offline formats. They are
  intentionally **not** accepted by the older restore paths.
- No signed-chain adapter, proof verifier, Litecoin Scrypt, Dogecoin AuxPoW,
  economic activation rule, multi-series margin engine, liquidation, fee
  source, insurance or bankruptcy waterfall has been implemented.
- Contract names, matched trades, actors, price and funding are caller
  assertions, and even an internally consistent state may represent an
  economically invalid historical ledger. Replaying historical JavaScript
  still requires exact deployment rules and authenticated source evidence.
- Full-state clone-and-swap and snapshot copying prioritize correctness
  over production throughput. Binary and witness sizes are bounded but
  untrusted input should still be parsed only within a constrained process.
- The existing zero-external-Cargo-crate, frozen toolchain policy remains.

## Validation

\`\`\`sh
python3 scripts/check_rust_supply_chain.py
cargo fmt --all -- --check
rustfmt --check --edition 2021 src/clearing_snapshot_impl.rs
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen --test clearing_snapshot --test clearing_chain
cargo test --frozen
cargo build --all-targets --release --frozen
\`\`\`

The tests include fully mixed spot+clearing snapshots, cross-lane replay
rejection, corrupted/recomputed CRC, wrong genesis/contract/mark policy,
zero-balance historical clearing events, repeated mark epochs, restart
and fork-replacement, deep-history refusal, late-invalid-operation
atomicity, empty block finalization, file collisions and unavailable
checkpoint directories.

This work stays entirely offline, independent of the Stage 23 Litecoin
historical-evidence branch. It adds **zero external crates**.
