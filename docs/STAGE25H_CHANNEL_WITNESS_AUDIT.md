# Stage 25H — Native P2WSH 2-of-2 funding/spend structural audit

**Research only — no channel-signature, liquidation, or live transaction authorization.**

Baseline: TradeLayer-Rust main 7e6da8b89da1ef28340473f053a2bd353ef2bd5a.
Historical source: tradelayer.js@dca0888b827c5bc28fa9189bc9e1d507f5c5670c.

## Actual pinned channel script formats

The historical txUtils.js lines 1419–1490 selects TWO constructions:

- Native P2WSH: bitcoinjs-lib p2ms(m=2, two public keys) inside p2wsh.
- Legacy P2SH: bitcore-lib for Bitcoin, bitcore-lib-ltc for Litecoin.
- The method does not support Dogecoin.
- channels.js lines 754–792 extracts a public key from a type-4
  commitment transaction's input and checks both participant-key
  orderings against the registered multisig channel address.
- validity.js lines 473–653 checks token eligibility, commit sender,
  amount/balance, activation, property clearlist, optional commitment
  clearlist and the channel address.
- validity.js lines 1804–1960 separately checks type-19 channel trades:
  reference price and leverage, expiry, participants, activation,
  margin, and its at-least-one member channel clearlist rule.

These checks do not prove both signatures on a particular channel state.

## Implemented read-only native P2WSH slice

The new src/channel_witness_audit.rs parses actual serialized Bitcoin-style
funding and spending transaction bytes and two claimed compressed keys.
It checks the following strict structural properties:

1. Canonically serialized funding/spend transactions with bounded
   CompactSize and the existing read-only transaction parser.
2. Recomputed funding non-witness TXID and exact spending input outpoint
   equal to that funding TXID in raw little-endian bytes and vout index.
3. Positive base-chain funding coin output and the exact native P2WSH
   scriptPubKey of length 34, bytes 00 20 plus SHA256(script).
4. Empty native SegWit scriptSig and exactly four witness items:
   empty CHECKMULTISIG dummy, two BIP66-DER-shaped signatures ending
   in SIGHASH_ALL, and the witness script.
5. Exact compressed-key template: OP_2 PUSH33(pubkeyA) PUSH33(pubkeyB)
   OP_2 OP_CHECKMULTISIG. Keys must be distinct and equal to the
   two claimed participant keys, accepting the two source orderings.
6. SHA256 of those exact 71 script bytes equals the hash committed by
   the funding output, without trusting a caller-supplied hash flag.

The frozen script-program test vector is independently calculated
with Python standard-library hashlib.sha256. The Rust internal SHA256
has known-answer tests but is not independently audited.

### The critical boundary

Two syntactically correct DER byte sequences can be fake signatures.
THEY DO NOT PROVE ECDSA, ownership, or channel co-signature.

The result type UnverifiedP2wshOutpointSpend exposes only diagnostic
observations. Its require_verified_channel_signatures method ALWAYS
returns SignaturesNotVerified. The test intentionally supplies fake
DER-looking signatures and confirms the denial.

No BIP143 sighash or secp256k1 ECDSA validation is implemented.
Pubkey compressed encoding alone does not prove the key lies on the
curve or that any signer knows its secret.

There is no proof of chain inclusion, best-work selection, script
execution, state revision, revocation, commitment invalidation or
channel signer consent. The output is not connected to a Ledger,
ClearingSandbox, ClearingChainState, JointChainState or type-4/19
trusted dispatcher. Funding base-chain satoshis are NOT TradeLayer
token atoms and do not establish property commitments or margin.

Historical P2SH exists, but HASH160/scriptSig and validated
cryptographic authorization are NOT implemented. Legacy P2SH,
nested P2SH-P2WSH, Dogecoin, wrong sighash types, unknown witness
forms or claimed signatures as booleans are explicitly unsupported.

## Test inventory

tests/channel_witness_audit.rs covers:

- Exact independently hashed witness program, computed funding TXID,
  outpoint/vin selection, positive funding output and both key orders.
- Wrong parent TXID, wrong output index, changed script program,
  P2SH funding output, malformed script and identical/uncompressed keys.
- Empty CHECKMULTISIG dummy, signature count, strict DER shape,
  nonminimal/negative R/S, unsupported signature hash flag.
- Native versus legacy/nested spend, synthetic Dogecoin input,
  nonminimal CompactSize, truncated/trailing transaction bytes.
- Fake DER-shaped signatures which pass structural checks but
  NEVER become cryptographically verified.

Offline commands:

    python3 scripts/check_rust_supply_chain.py
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test channel_witness_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen

No third-party Rust crates, network requests, node/testnet/wallet
operations, funds, new external agents, or deployment.

## Missing before real liquidation

A reviewed BIP143 sighash implementation and independently audited
secp256k1 signature verification (or authenticated full-node spend
evidence); actual type-4 commitment and historical clearlist rules;
exact 2-of-2 channel revision and revocation/invalidation proof;
full applicable Bitcoin/Litecoin/Dogecoin consensus/header status;
the source-authorized oracle/native price; verified AMM/orderbook
fills; and ratified bankruptcy/insurance/ADL activation.

Stage 25G authentication remains unavailable. An apparently valid
funding/spending script is not an authorized economic proof.
