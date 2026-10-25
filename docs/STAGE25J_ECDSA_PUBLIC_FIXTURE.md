# Stage 25J: actual offline 2-of-2 ECDSA evidence (OpenSSL, not consensus)

**Purpose:** Independently verify the mathematics of both secp256k1
signatures over the existing Stage25I BIP143 SIGHASH_ALL digest, while
keeping the Rust economic kernel dependency-free and *fail-closed*.

Baseline: [TradeLayer-Rust main dbe9bafb](https://github.com/patrickdugan/TradeLayer-Rust/commit/dbe9bafb69493bff11c421a0be8f0982e27863c3).
Pinned historical JS: [tradelayer.js dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).
Related issues: [Stage25F #80](https://github.com/patrickdugan/TradeLayer-Rust/issues/80).

## Inputs and provenance

The frozen public fixture at
`fixtures/channel_p2wsh_ecdsa_signed_synthetic.tsv`
contains two synthetic raw Bitcoin-style transactions: a funding output
of 20,000 base-chain satoshis and a SegWit-v0 transaction spending its
first output as input #1. An unrelated input #0 is included to exercise
the BIP143 hashPrevouts and hashSequence commitments.

The P2WSH script is OP_2 PUSH33(A) PUSH33(B) OP_2 CHECKMULTISIG,
with the public keys corresponding to deterministic **throwaway test**
secp256k1 scalars 1 and 2 (generator G and 2G). The two ECDSA signatures
were generated strictly for this offline fixture using the Python
cryptography package in the author's isolated construction environment.
**No private-key bytes, signing scripts, user wallet keys, live Bitcoin
UTXOs, or transaction broadcast are included in the repository.**

The fixture and validation runner need only public data: funding and
spending transaction bytes, public keys, signatures, and the actual
computed SHA256d BIP143 message digest. The witness hashes and public
keys are derived independently from the transaction bytes by the
Python standard-library validator; they are not trusted assertions.

## Independent test verifier

`scripts/audit_channel_ecdsa_openssl.py --check` uses **Python
standard-library** hashlib, struct, base64 and subprocess plus a
preinstalled **system OpenSSL CLI**. No external Rust crates or optional
Cargo features have been added, and no `pip install` is part of CI.

It independently:

1. Parses both serialized raw transactions with canonical CompactSize,
   checking witness marker/flag, selected vin/vout and complete bytes.
2. Computes the actual funding non-witness SHA256d TXID; confirms the
   spender references the funding output in binary outpoint order.
3. Checks the native witness program matches SHA256 of the exactly
   71-byte compressed two-party script, and extracts the two DER
   signatures from the actual four-item witness.
4. Computes hashPrevouts of **all** vin outpoints, hashSequence of
   **all** vin sequences and hashOutputs of **all** output byte records.
   Builds the full BIP143 SIGHASH_ALL preimage, using actual funding
   output satoshis and the *second* selected spending input.
5. Reconstructs secp256k1 SubjectPublicKeyInfo PEM from the two public
   compressed points, then independently invokes
   `openssl pkeyutl -verify -pubin ... -pkeyopt digest:sha256`
   on the actual computed 32-byte digest and each DER signature.
6. Requires success for A-on-signature-A and B-on-signature-B. Rejects
   cross-signer swaps, duplicated signatures, mutated DER signature
   content, an altered output payment, and a changed funding coin
   value with the correspondingly updated funding transaction outpoint.

The verifier uses fixed test files and subprocess argument arrays, not
the shell. A missing OpenSSL binary or verification failure aborts CI
rather than assuming validity.

**This is mathematical ECDSA verification for synthetic test vectors,
not a production cryptographic API.** The OpenSSL CLI is a system
external program and therefore explicitly part of the offline test
environment trust base; it is not part of the dependency-free Rust
consensus kernel. The wrapper does not infer a genuine blockchain
UTXO, correct chain selection, or verified off-chain state.

Rust `tests/channel_signed_evidence.rs` loads the same committed
public fixture and independently derives the expected digest via
`src/channel_bip143_audit.rs`, checks all 3 component hashes, the
funding and spending non-witness TXIDs, and the spending wtxid.
Changing a signature witness changes wtxid but **not** BIP143 digest
or non-witness txid. Every call to
`require_verified_channel_signatures()` still returns
`SignaturesNotVerified` — no OpenSSL assertion is imported into
Rust settlement.

## Critical limitations

- Actual OpenSSL ECDSA signature validity in this synthetic example is
  necessary but **insufficient** for valid real-channel authorization.
  A non-blockchain fixture has no best-work, transaction inclusion,
  UTXO spend, input ownership, OP_CHECKMULTISIG script-execution,
  channel revision/revocation, active expiry or type4/type19
  whitelist/activation verification.
- Both signatures in this example bind the raw transaction digest, not
  a separate TradeLayer off-chain update transcript. The correct
  signed-state format, revision, economic amount and possible
  invalidated commitment remain independent tasks.
- Only **native P2WSH, BIP143 SIGHASH_ALL** is examined.
  Historical P2SH/HASH160, Litecoin chainwork/Scrypt, Dogecoin AuxPoW,
  other sighashes, Taproot, and legacy signature hashing are not covered.
- The internal SHA256 implementation is independently tested here
  against Python hashlib and OpenSSL ECDSA only for these fixtures.
  Production cryptographic code and chain consensus still require
  external audit and dedicated security review.
- No valid-signature or authorized-liquidation capability is issued
  by Rust; Stage25G permanently reports missing independent proofs,
  and type3/23/31 remain denied.

### Reproduce offline

```sh
python3 scripts/check_rust_supply_chain.py
python3 scripts/audit_channel_ecdsa_openssl.py --check
cargo test --frozen --test channel_signed_evidence
cargo test --frozen --test channel_witness_audit
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen
cargo build --all-targets --release --frozen
```

**No new Rust crates, wallet operations, live node scans, testnet,
paid APIs, external agents, or network infrastructure.** The new
OpenSSL dependency is test-only and uses the system-installed tool,
not a new runtime or production crypto dependency.
