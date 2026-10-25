# Stage 25L: relative Bitcoin P2WSH dual-inclusion audit

**Research-only evidence.** This stage binds one raw P2WSH funding transaction
and its spending transaction to txid-Merkle paths in the same locally checked
Bitcoin SHA256d header segment. The returned
`UnverifiedBitcoinChannelInclusion` has private fields and an authorization
method that **always returns** `ActiveChainWitnessAndSignaturesUnverified`.

## Checks made

The existing `bitcoin_inclusion::CheckedHeaderSegment` checks SHA256d header
proof-of-work against a claimed profile ceiling, each header's parent link,
and a transaction's **non-witness txid** Merkle path. Stage 25L requires both
transactions in the same checked segment and rejects spending before funding.
An intra-block spend must appear at a higher transaction index than funding.

It composes those inclusion results with the existing Stage 25H structural
native P2WSH audit: selected raw funding output amount, funding TXID/vout,
spending vin/prevout, exact 71-byte two-pubkey witness script, its SHA256
commitment, native empty scriptSig and DER-shaped SIGHASH_ALL witness elements.
These checks use unchanged production routines and add no Cargo crates.

The audit is **Bitcoin-only**. Litecoin Scrypt and Dogecoin AuxPoW require
independent network-specific validation. The test-only signed fixture was
originally marked Litecoin because it uses the same Bitcoin-style serialized
transaction format. Reusing its raw bytes with fabricated Bitcoin-like
headers establishes an offline adversarial vector, not real cross-chain
funding, confirmation or network ownership.

## Deterministic independent fixtures

`tests/channel_inclusion_audit.rs` uses two fake regtest-like headers at
claimed heights 101 and 102, anchored to a caller-chosen block ID of
`34` repeated 32 times. A separate fabricated one-block Merkle tree
contains funding at index zero and spending at index one. All headers use
`nBits=0x207fffff`, and no source claims they are authentic Bitcoin blocks.

`scripts/audit_bitcoin_channel_merkle_fixture.py` independently parses the
Stage 25J public funding/spend transaction bytes, derives each non-witness
TXID using Python `hashlib`, rebuilds the headers/Merkle roots, checks their
simple PoW ceiling and verifies the raw spending prevout. It runs in CI
separately from Rust's internal SHA-256d implementation.

A decisive counterexample changes a DER signature's R byte while retaining
its syntactic DER shape. The spending **wtxid changes**, yet the **txid does
not**, so the same block header and txid-Merkle proof remain valid. Stage 25L
explicitly demonstrates that relative transaction inclusion does not prove
which SegWit witness appeared in the block. A valid coinbase witness commitment
plus full-block and script validation remain separate proof obligations.

Tests also reject wrong sibling hashes, altered funding transaction amount,
invalid header height, reversed same-block ordering, malformed witness DER,
empty two-leaf proof and unsupported Litecoin/Dogecoin header claims.

## Precise outstanding authorization work

1. Authenticate the chosen Bitcoin best chain and all historical difficulty,
   full-block/UTXO, coinbase witness commitment and confirmation rules. The
   present header segment is deliberately anchored to caller-supplied data.
2. Verify both ECDSA signatures against the precise BIP143 digest, actual
   script execution and the spent UTXO, under an independently audited
   cryptographic implementation or authenticated full-node evidence.
3. Bind the on-chain transaction to the correct channel state, both parties'
   latest agreed revision, revocation/expiry and historical Type-4/Type-19
   whitelist/admission semantics.
4. Verify the market inputs, liquidation activation, AMM-first execution,
   insurance/ADL and sat rounding before exposing any economic permit.

The existing `TrustedChainAdapter`, clearing transitions and production
`require_verified_channel_signatures()` interfaces remain untouched.
**No funds, transactions, wallets, testnet/mainnet nodes, secrets or deployment**
are involved. The source remains a necessary evidence checker only.

### Offline verification

```sh
python3 scripts/audit_bitcoin_channel_merkle_fixture.py
python3 scripts/check_rust_supply_chain.py
bash scripts/ci-preflight.sh --full
cargo test --frozen --test channel_inclusion_audit
cargo test --frozen --test bitcoin_inclusion --test channel_witness_audit --test channel_signed_evidence
cargo build --all-targets --release --frozen
```

The exact new source was staged as text and format-checked by
[CI-only PR #96](https://github.com/patrickdugan/TradeLayer-Rust/pull/96)
before publication. Source files and CI gates are published by their verified
Git blobs. The wider formal and liquidation trust boundaries remain open in
[issue #47](https://github.com/patrickdugan/TradeLayer-Rust/issues/47)
and [issue #80](https://github.com/patrickdugan/TradeLayer-Rust/issues/80).
