# Stage 25L: Real synthetic legacy P2SH two-signer ECDSA evidence

**Offline research evidence only. No authorized channel state, chain spend, or liquidation.**

Baseline main: [Stage25K PR #95](https://github.com/patrickdugan/TradeLayer-Rust/pull/95), merge 1ad1e3791d1fa105fc20f2acb79bba59b6e82c87.
Historical source: [tradelayer.js@dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c) txUtils.js:1419–1490 for Bitcoin/Litecoin P2SH or native P2WSH; channels.js:754–792 for participant-derived addresses.

## Narrow security claim

The frozen public fixture at fixtures/channel_p2sh_ecdsa_signed_synthetic.tsv has raw synthetic funding and spending transactions, two compressed secp256k1 pubkeys (G, 2G), a canonical 71-byte OP_2/PUSH33/PUSH33/OP_2/CHECKMULTISIG redeemScript, 2 genuine DER-encoded ECDSA signatures followed by SIGHASH_ALL, and a 20,000-sat P2SH funded output. Synthetic input #0 is unrelated; input #1 spends output #0 of the serialized funding transaction. Private scalar bytes, key generation code, wallets and live outputs are absent from the committed fixture and CI. The local fixture constructor was deliberately not committed. This vector has no confirmed-chain status.

The independent Python standard-library verifier recomputes SHA256d for both actual raw non-SegWit transactions, checks vin/outpoint to real computed funding TXID, canonical script pushes and HASH160(redeemScript), parses both strict DER+0x01 signatures, and independently derives the legacy message digest from the serialized transaction. It calls **system OpenSSL pkeyutl** to mathematically verify both signer public keys, with swapped, duplicated, forged and altered-payment inputs tested as negatives. The Python verifier imports no Rust or Cargo logic, and it aborts if OpenSSL is missing.

The Rust src/channel_p2sh_sighash_audit.rs starts by calling Stage25K's P2SH structural/outpoint/HASH160 audit, then reconstructs the legacy SIGHASH_ALL preimage with only the selected vin's scriptSig replaced by the actual redeemScript. Every other input scriptSig is encoded empty. Input count, outpoints, input sequences, output count, output values and scripts, version, locktime and the four-byte little-endian 0x00000001 sighash type are committed. The fixed script contains no executable OP_CODESEPARATOR and no script signature to FindAndDelete. Unlike BIP143, legacy SIGHASH_ALL does not directly append the spent output satoshi amount. That value is bound *indirectly* by funding TXID through the selected prevout. A different funding value makes a different funding TXID; a stale outpoint rejects.

The pinned independent expected legacy digest is:

    a182ac7536b9eba1f0c922897fd58d32593ac16c8b69283ccda27d55382cb4e9

The entire expected preimage is separately frozen in the TSV, so tests catch a coincidental digest regression or mismatch in actual committed fields. Signed scriptSig mutation preserves the digest but changes the legacy transaction TXID, the inverse of the SegWit witness-only mutation observation in Stage25J. Mutating an output amount, other input prevout, input sequence, version or locktime changes the digest and makes the original genuine signatures fail OpenSSL verification.

Five focused Rust tests check signed fixture parity, BTC/LTC serialization semantics, signature mutation/digest independence, actual output/other vin/sequence/version/locktime commitments, funding amount-and-prevout rebinding and permanent authorization denial. Python independently checks both actual signatures and malicious changes. No new Rust crates, features, Cargo lock changes, signing capability, privileged API, or trusted execution path were added. OpenSSL remains a test-only **system** dependency.

## Permanent independent proof obligations

Successful Stage25L ECDSA mathematics proves only that **two throwaway synthetic keys signed one synthetic spending transaction**. It does not prove full legacy Bitcoin/Litecoin Script interpreter execution, consensus flags, confirmed transaction inclusion, UTXO membership and unspent status, best-work chain selection, historical activation, actual channel participant ownership, a canonical co-signed off-chain state revision, cancellation/revocation/invalidation, the type-4 sender/token commitment rule or type-19's distinct at-least-one-member clearlist rule. Nor does it verify oracle/native mark, authentic AMM-first matched liquidity, bankruptcy math, insurance waterfall or ADL authorization.

The typed result UnverifiedP2shLegacySighashDigest only exposes a read-only digest and structural observations. Calling require_verified_channel_signatures() **always** returns SignaturesNotVerified. There is no new edge into Ledger, ClearingSandbox, TLCL/TLCC, JointChainState, or trusted type3/23/31 economic replay. Stage25G remains AuthenticationUnavailable. Dogecoin, nested P2SH-P2WSH, other sighash types and noncanonical historical scripts remain outside this deliberately narrow subset.

The next proof gate should independently establish historical type-4/type-19 channel admission and a source-supported signed-state/invalidated-commitment format, then connect actual independently verified base-chain evidence. If the pinned JS never defined a canonical revision/revocation transcript, the next step is protocol specification and ratification rather than guessing a backwards-compatible consensus rule. This job is neither a wallet nor a mainnet/testnet exercise.

## Reproduce offline

    python3 scripts/check_rust_supply_chain.py
    python3 scripts/audit_p2sh_legacy_ecdsa_openssl.py --check
    cargo test --frozen --test channel_p2sh_signed_evidence
    cargo test --frozen --test channel_p2sh_audit
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen
    cargo build --all-targets --release --frozen
