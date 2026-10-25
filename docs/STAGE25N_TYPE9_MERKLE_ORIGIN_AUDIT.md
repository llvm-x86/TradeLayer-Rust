# Stage 25N: Relative Bitcoin Type-9 attestation origin audit

**Research-only.** This stage binds a *syntactic* source-like type-9 direct-attestation payload to actual raw transaction bytes and an independently SHA256d-checked TXID-Merkle branch inside one caller-anchored, locally checked Bitcoin header segment. The output remains explicitly UNVERIFIED for TradeLayer admission, channel revision and liquidation.

Baseline: [Stage25M main 12d458e0](https://github.com/patrickdugan/TradeLayer-Rust/commit/12d458e04df58bff5a0f295642b3a04774be988c). Historical source: [tradelayer.js dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).

## Pinned source facts and unresolved versions

- **Transaction type 9**: src/types.js:25–61,248–269 selects the issue/revoke-attestation encoder/decoder/validator. src/txDecoder.js:250–260 uses four comma-separated fields: revoke flag, base-36 clearlist ID, target address, metadata. src/txEncoder.js:220–233 also builds that four-field payload.
- **Possible DOUBLE PREFIX**: Types.encodePayload already begins with a TradeLayer marker and type ID, while Encode.encodeIssueOrRevokeAttestation also prepends a marker and type. This can create a nested envelope. Existing src/wire.rs explicitly treats nested envelopes as ambiguous. Stage25N accepts only a single syntactic source frame and **rejects** nested prefixes. It cannot claim to cover all historically emitted type-9 encodings until the actual block-processor/carrier call path is independently fixed and versioned.
- **Type-9 authority**: src/validity.js:1003–1098 requires type-9 activation; for nonzero list ID, the claimed sender must match the historical list admin; revocation requires an existing active attestation. Zero-list records have special country-self-cert and BANLIST handling, including an administrator exception. Target references may be resolved through transaction outputs. None of those authorization/context checks is discharged by parsing OP_RETURN bytes or proving Merkle inclusion.
- **Tip-database insufficiency**: src/clearlist.js:147–189 stores rows under address-based keys, risking overwrites across list IDs. This stage uses a bounded *event list*, and never interprets tip rows as a complete verified historical membership series. It also excludes derived-xpub, merkle-root and country events from this direct subset.

## Implemented raw-byte and relative-header checks

New src/channel_type9_inclusion_audit.rs composes unchanged read_only::inspect_transaction, bitcoin_inclusion::CheckedHeaderSegment::check_transaction_inclusion, Stage25M ClaimedHistoricalDirectView, and one narrow four-field type-9 parser.

For each event, the bridge checks a serialized raw Bitcoin-style transaction, its TXID (not an asserted digest), proof index/count/siblings, claimed block hash/height, exact tx index, claimed raw TXID, one unambiguous ASCII TL carrier, type-9 selector, revoke flag, positive list ID and target address. A mismatch refuses.

The *target* type-4 or type-19 transaction also requires its own raw bytes and relative Merkle proof in that **same** checked header segment, with the expected TL type selector. Its economic body, sender and TradeLayer authorization remain opaque. Stage25M then rechecks strict prior-event order, same-block chronological precedence, claimed fork consistency and the direct sender-vs-either-party list subset. Source-selected metadata are recorded only as opaque bytes and give no proof of an administrator.

The returned UnverifiedBitcoinType9History stores read-only checked TXID/position observations and borrowed claimed event metadata. It carries **no authority token**:
- require_authenticated_admission() always yields OriginIsNotAuthorization.
- require_canonical_signed_revision() always yields CanonicalRevisionUnspecified.
- Stage25M's own authenticated admission and signed-revision methods remain hard-deny.
- No call reaches Ledger, clearing, trusted protocol replay, reserve debits, type 3/23/31 or live liquidation.

## Deterministic falsifiers

The frozen fixtures/channel_type9_merkle_synthetic.tsv has five fabricated Bitcoin-style transactions in **one** fake regtest-like block at height 101, anchored to user-selected display hash 6a repeated 32 times.

| TX index | Raw ASCII OP_RETURN | Synthetic meaning |
|---|---|---|
| 0 | tl90,7,alice,OK | Direct list-7 attest Alice |
| 1 | tl90,b,bob,OK | Direct list-11 attest Bob |
| 2 | tl91,7,alice,OK | Direct list-7 revoke Alice |
| 3 | tl40,7 | Opaque type-4 target |
| 4 | tlj1,2 | Opaque type-19 target |

An independent Python standard-library tool, scripts/audit_type9_merkle_vectors.py, constructs every synthetic raw transaction from its stated payload and compares five SHA256d TXIDs, Merkle branches, header root/ID and local regtest-like PoW. It rejects mutated payloads, wrong branches and positions. The Rust suite reproduces those paths and exercises source-claim list/target/action substitution, wrong TXID, cross-network, mismatched or forged block positions, duplicate event, impossible chronology, wrong target type, unsupported Bitcoin alternative chains and code ambiguity. It also makes the successful Type4/Type19 direct match return permanent authorization denial.

**The synthetic block is not valid Bitcoin consensus history.** Fake base-chain inputs have no UTXO ancestry; the header is a fabricated profile-conforming hash with a caller-selected anchor, and lacks a complete block/coinbase/witness/script proof. Likewise a perfectly matching type-9 byte string may be sent by a stranger, or be excluded by the source's historical activation/admin/revocation rules. Relative TXID inclusion alone cannot decide that.

## Remaining acceptance gates

1. Authenticated Bitcoin active-chain (validated difficulty/chainwork, full blocks including coinbase witness commitment, transaction order, UTXO state, scripts and reorg decisions). Litecoin requires separately verified Scrypt; Dogecoin requires its AuxPoW route. A synthetic or user-specified high-height anchor is not an active-chain proof.
2. Unambiguous historical type-9 wire-source contract, including nested-prefix encoder paths and ref-to-vout target resolution. Reconstruct *first-input sender* from actual prior output scripts and validate the historical clearlist admin, type-9 activation, existing active rows on revocation and source authorization.
3. Replay full authenticated type-7 list creation, type-8 administration, type-9 events, property/commit metadata and independent xpub-derived, country, ban and registry data. An incomplete event bundle can appear sufficient but cannot establish global historical validity.
4. Ratify canonical co-signed channel-state bytes, funding-outpoint binding, state revision, supersession/revocation, commitment invalidation, expiry, dispute and reorg semantics. Prior ECDSA signatures bind chain transaction bytes, not this missing off-chain transcript.
5. Independently verify current oracle/native reference, AMM-first actual fills, complete margin/bankruptcy accounting, insurer waterfall and ADL before economic activation.

## Reproduce offline

    python3 scripts/check_rust_supply_chain.py
    python3 scripts/audit_type9_merkle_vectors.py
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test channel_type9_inclusion_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen

No new third-party Rust crates, signing keys, wallets, live funds, nodes, network scans, testnet or protocol state changes.
