# Stage 25I: BIP143 SIGHASH_ALL digest for an unverified P2WSH channel

**Scope:** exact *message-digest* calculation for the already bounded
Stage25H native P2WSH 2-of-2 structural research path, with independently
generated offline test vectors. There is no ECDSA verifier, key material,
channel consent, UTXO inclusion, trusted liquidation, or economic settlement.

Baseline: `TradeLayer-Rust@16e3576d72863da1db75a5fb062f4b05f6fa8083`.
Pinned historical `tradelayer.js@dca0888b` source:
`txUtils.js:1419–1490` distinguishes P2WSH from P2SH,
`channels.js:754–792` reconstructs channel address from two pubkeys,
and `validity.js:473–653` applies type-4 sender and commitment controls.
Specification: [BIP143](https://bitcoin.org/bip/143/).

## Implemented

`src/channel_bip143_audit.rs` first invokes the existing
`audit_native_p2wsh_outpoint_unverified`, which checks an actual
serialized funding TXID and spending input outpoint, a committed
71-byte two-pubkey native P2WSH script, canonical witness script and
two syntactically DER-shaped SIGHASH_ALL signature elements.

Then, from the **raw serialized spending transaction**, it calculates:

```text
hashPrevouts = SHA256d(all input outpoints, serialized in vin order)
hashSequence = SHA256d(all input nSequence fields)
hashOutputs = SHA256d(all serialized outputs, amount+CompactSize+scriptPubKey)

preimage = nVersion (LE 4)
         || hashPrevouts (raw 32)
         || hashSequence (raw 32)
         || outpoint (selected input txid LE + vout LE)
         || CompactSize(71) || actual 71-byte witnessScript
         || funding-output value (base-chain satoshis, LE 8)
         || selected input sequence (LE 4)
         || hashOutputs (raw 32)
         || nLockTime (LE 4)
         || SIGHASH_ALL (01000000)
digest = SHA256d(preimage)
```

The selected vin can be the *second* input while the first input
still contributes to `hashPrevouts` and `hashSequence`.
The amount comes **only** from the exact raw funding output, never
from a caller-provided margin or price figure.

This bounded implementation supports only SegWit-v0 **native P2WSH**,
the source's fixed compressed 2-of-2 `CHECKMULTISIG` script with
**no executable CODESEPARATOR**, and `SIGHASH_ALL` (0x01) with
no `ANYONECANPAY`. Other sighash modes, P2SH, nested P2WSH,
non-canonical script templates, Taproot and Dogecoin remain denied.

Byte `0xab` inside a **pushed compressed pubkey** is data, *not*
an executable OP_CODESEPARATOR; the Stage25H script template checks
the actual opcodes separately. No incorrect blanket scan of 71 raw
script bytes is used.

## Independent verification

The fixture in `tests/channel_witness_audit.rs` is reproduced using
Python stdlib `hashlib.sha256` in
`scripts/audit_bip143_channel_fixture.py`, independently of Rust
serialization and cryptography code.

Frozen vector for the synthetic *second* signed input:

| BIP143 component | Raw digest (hex) |
| --- | --- |
| `hashPrevouts` | `ea816a41f5f26c3bf6798e1120432065e3ff7679880d45d1bc24820e23d3d308` |
| `hashSequence` | `752adad0a7b9ceca853768aebb6965eca126a62965f698a0c1bc43d83db632ad` |
| `hashOutputs` | `7ce42f6d181802782fc3ea5329d76e8fe344cbc1aa49742806d5a71528d6cd49` |
| **Final BIP143 digest** | **`5dc9f28f470cf591f0e7aacd431bbbdd31c8011ba96f3eb13c17e1ec8d408210`** |

The Python audit also checks a published BIP143 example's complete
preimage and known digest
`c37af31116d1b27caf68aae9e3ac82f1477929014d5b917657d0eb49478cb670`.

Adversarial Rust fixtures change the funding output *amount* (with
correctly updated funding TXID), spending transaction version, first
input sequence, output amount and locktime; each must alter the BIP143
digest. Changing *only the signature witness bytes* leaves the digest
and non-witness TXID unchanged but changes the `wtxid` — BIP143 signs
the transaction contents, not its own signatures.

Wrong prevouts, wrong witness input, unsupported sighash, malformed
serialization and unsupported network continue to fail closed.

## What a computed digest does NOT prove

The output is named `UnverifiedP2wshBip143Digest` and contains
only read-only hashes and the Stage25H structural observation.
The method `require_verified_channel_signatures()` **always
returns `SignaturesNotVerified`**, even for correct BIP143 hashes.

**Two fake, DER-shaped signatures pass** the structural checks and
still yield a valid SHA256d message digest. They do not verify ECDSA
or prove that either participant consented. The code never constructs
a verified signer object or authorization capability. It does not
validate a secp256k1 public point, implement BIP143 script execution
or a full BIP66/BIP146 chain consensus signature policy. Its current
internal SHA256 is known-answer tested, but not independently audited
as a production cryptographic primitive.

Also unproven: full Litecoin/Bitcoin best-chain confirmation,
actual unspent commitment and revocation status, type-4 token
whitelist at commitment height, source-authorized type-19 channel
trade with both parties, co-signed off-chain state revision and
expiry, oracle/native mark provenance, AMM/regular maker priority
and economic liquidation policy. A separate audited ECC+script
verifier or independently authenticated full-node evidence would be
required before treating any signature as valid. P2SH needs a
dedicated reviewed HASH160/redeemScript and legacy sighash path.

No funding or spending transaction in the test corpus sends funds:
all test bytes are synthetic and never submitted to a network.
No wallet keys, testnet/node work, new dependencies or external
tool installations. Existing type3/23/31 trusted replay and
liquidation continue to deny these observations.

## Offline commands

```sh
python3 scripts/check_rust_supply_chain.py
python3 scripts/audit_bip143_channel_fixture.py
cargo fmt --all -- --check
cargo clippy --all-targets --frozen -- -D warnings
cargo test --frozen --test channel_witness_audit
cargo test --frozen
cargo build --all-targets --release --frozen
```
