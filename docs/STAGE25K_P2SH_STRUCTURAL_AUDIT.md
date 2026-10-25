# Stage 25K: Legacy P2SH 2-of-2 commitment and spending-structure audit

**Offline structural provenance only; not a Bitcoin/Litecoin Script interpreter,
ECDSA signature verifier, validated UTXO inclusion proof, co-signed channel
state or liquidation authorization.**

Rust baseline: TradeLayer-Rust main `0cdc632134bc11b356ab9a61af21d388e6d0b6c7`.
Pinned source:
[tradelayer.js@dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).
Standard: [BIP16](https://bitcoin.org/bip/16/).

## Why this particular path

Historical `txUtils.js:1419–1490` has **two actual 2-of-2 scripts**.
For a native bech32 address it constructs P2WSH through bitcoinjs-lib;
for BTC P2SH or LTC P2SH it uses bitcore-lib / bitcore-lib-ltc.
`channels.js:754–792` reconstructs a 2-of-2 address from the committed
participants' extracted pubkeys. These checks are not claims of valid
signatures, legitimate value ownership or channel-state authorization.

The preceding stages established native P2WSH script/output structure
(Stage25H), BIP143 SHA256d message digest (Stage25I) and independent
OpenSSL verification of **synthetic** native SegWit ECDSA signatures
(Stage25J). None covers the P2SH source branch.

## Current strictly bounded Rust implementation

`src/hash160.rs` implements RIPEMD160 and HASH160 (RIPEMD160(SHA256))
for historical P2SH research. The implementation adds **zero external
Rust crates** and is backed by seven independent hashlib RIPEMD160
vectors (including the 55/56-byte padding boundary and 1M-byte test)
plus hashes of the same two-key redeemScript in both pubkey orders.
It is NOT independently audited production consensus cryptography.

`src/channel_p2sh_audit.rs` consumes RAW funding/spending serialized
transactions and two claimed compressed 2-of-2 public keys:

1. Use existing strict read-only transaction parser for canonical
   transaction serialization and recomputed actual non-witness TXIDs.
2. Select a funding vout holding positive Bitcoin/Litecoin base coin
   and an exact `OP_HASH160 PUSH20 <hash> OP_EQUAL` 23-byte script.
3. Select the spending vin, require **no SegWit witness marker**,
   and bind its 36-byte outpoint to the *actual* funding TXID and vout.
4. Interpret only a **strict subset** of push-only scriptSig:
   `OP_0 PUSH(sigA) PUSH(sigB) PUSH71(redeemScript)`,
   with direct/minimal pushes. Accept no extra script operations, tags,
   trailing pushes or nonminimal `OP_PUSHDATA1` encodings.
5. Reuse Stage25H BIP66 DER **shape** and `SIGHASH_ALL`-only checks,
   requiring two distinct signature byte strings.
6. Reuse Stage25H's exact canonical
   `OP_2 PUSH33(pubA) PUSH33(pubB) OP_2 CHECKMULTISIG` script template,
   matching two distinct claimed compressed pubkeys in either order.
7. Compute HASH160(redeemScript) and compare to the 20-byte script
   hash in the raw funding output; reject any byte mismatch.

**This is deliberately narrower than full BIP16 consensus**, which
permits other push-only encodings and script forms subject to its
activation rules, context and consensus flags. Rejecting a script is
not a historical consensus-invalidity verdict.

The output `UnverifiedP2shTwoOfTwoSpend` has no constructor that
claims cryptographic authorization. Calling
`require_verified_channel_signatures()` **always** returns
`SignaturesNotVerified`.

### Source test distinctions

Our test harness uses deliberately **fake**, correctly DER-shaped
signatures; these may pass the P2SH structure and HASH160 checks.
That does not establish consent or a mathematically valid ECDSA
signature, even though two signer keys and a funding outpoint match.
Unlike BIP143, a P2SH legacy SIGHASH_ALL preimage is constructed
from the *legacy transaction serialization*, with the scriptCode
rules, chosen input script replacement and legacy sighash behavior.
No such digest/actual ECDSA verifier is present in this stage.

The test corpus verifies altered funding outpoints, outputs/hashes,
wrong redeem scripts, reverse-key order, wrong value, duplicated
claimed signers, malformed/nonminimal DER and scriptSig, zero dummy,
wrong SIGHASH type, invalid vin/vout, unsupported SegWit and Dogecoin,
and precisely the same permanent no-authorization result.

For the independent oracle, `scripts/audit_p2sh_hash160.py` calls
Python standard-library `hashlib.new("ripemd160")` backed by
the system OpenSSL provider. It tests the same seven known-answer
messages and two different 71-byte redeem-script HASH160 outputs.
If system RIPEMD160 is unavailable CI fails rather than assuming a
reference comparison occurred.

## Still missing — no invented channel revision policy

The pinned JavaScript stores channel participants and commitments and
validates token/channel clearlist eligibility, but the inspected
material does **not** independently define a canonical revocable
off-chain state revision/sighash transcript for Rust to port. A
type-4 whitelist claim, matching channel address, synthetic txid,
or P2SH script hash is not evidence of such a revision.

Real authorization requires: independently audited historical
P2SH legacy signature hashing + both secp256k1 ECDSA signatures;
actual confirmed chain output, correct activation/consensus rules
and spend status; token commitment and historical type-4/type-19
whitelist admission; channel revision and revocation/invalidation
binding; real oracle/native mark and AMM-first safe matched orders;
fully ratified bankruptcy, insurance and ADL policy. Stage25G remains
`AuthenticationUnavailable` and trusted type3/23/31 replay is denied.

The new module does NOT modify `Ledger`, `ClearingSandbox`,
`ClearingChainState`, TLCL/TLCC, or authenticated replay. Existing
stage25H/I/J remain unchanged except crate-private reuse of the
same already-tested script and DER shape helpers.

## Offline verification

```sh
python3 scripts/audit_p2sh_hash160.py
python3 scripts/check_rust_supply_chain.py
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen --test channel_p2sh_audit
cargo test --frozen
cargo build --all-targets --release --frozen
```

No new Cargo crate/build scripts/keys, node scans, testnet, paid APIs,
wallets, funds, external agents or deployment.
