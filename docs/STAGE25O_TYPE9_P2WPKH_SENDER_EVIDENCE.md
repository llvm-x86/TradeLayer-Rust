# Stage 25O: synthetic P2WPKH first-input cryptography and type-9 source routes

**Public-only, offline research. No wallets, real network, third-party Rust crates, private keys, production cryptographic verifier or economic authority.**

Baseline: [TradeLayer-Rust main 530a2839](https://github.com/patrickdugan/TradeLayer-Rust/commit/530a2839c05ca842e6ba155fe5a45b604d1380b1), after Stage25N. Historical source: [tradelayer.js dca0888b](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c).

## Exact source-path finding

The pinned JavaScript has **two different type-9 encoding routes**.

- src/txUtils.js:912–955 calls Encode.encodeIssueOrRevokeAttestation(contractParams) directly, puts its UTF-8 bytes into litecore.Script.buildDataOut, signs the base transaction and broadcasts via sendrawtransaction. utils/attest.js:69–79 and utils/createAttestation.js:98–105 call the direct builder/encoder. The direct expression from src/txEncoder.js:228–237 returns **tl90,7,alice,OK** for the synthetic list-7 issue and **tl91,7,alice,OK** for revoke, with a single marker/type prefix.
- src/types.js:27–61 instead prepends tl plus the base36 transaction ID before concatenating the already-prefixed encoder, giving **tl9tl90,7,alice,OK** for generic type-9 encoding. In src/txIndex.js:249–281, marker extraction begins after the first tl. Later in src/txIndex.js:301–315, processTransaction calls Types.decodePayload with just (txId, marker, payload), although Types.decodePayload accepts (txId, type, marker, encodedPayload, sender, reference, senderAmount, referenceAmount, block). That indexing path is an unreliable historical authorization oracle. Neither branch establishes which past raw type-9 transactions were broadcast.

The independent, dependency-free Node diagnostic scripts/audit_type9_source_paths.cjs reproduces these exact pinned source expressions and marker/decoder field behavior. The Stage25N strict raw parser supports the single-prefix subset and refuses nested-source ambiguity pending historic raw samples and consensus-version review.

**Sender source rule:** src/txUtils.js:170–203 gets the claimed sender from the address of the previous output referenced by the transaction's first input. For a positive list ID, src/validity.js:1003–1098 separately requires list-admin equality and active type-9 activation; revocation also needs a current attestation. List-zero country/BANLIST and xpub/merkle extensions have different semantics. This stage is Bitcoin P2WPKH and nonzero direct-type9 only.

## Exact research evidence

New Rust module src/channel_type9_sender_audit.rs consumes two complete synthetic Bitcoin-style serialized transactions and respective txid-Merkle paths under one **caller-anchored locally checked** header segment, the exact funding vout and a claimed Bitcoin sender address.

It recomputes actual funding SHA256d TXID; checks the spender's **first** input references that output; requires the actual funding script to be a positive-value native P2WPKH (OP_0 PUSH20). The first native SegWit input must have empty scriptSig, exactly one BIP66-shaped DER+SIGHASH_ALL and one 33-byte compressed public-key witness item. The key's independently computed HASH160 must match the parent output. A narrow BIP173 Bech32-v0 decoder checks the lower-case mainnet bc1q checksum, witness version, padding and 20-byte hash before binding the claimed sender address to the committed public key. It rejects malformed checksums, other valid addresses, Litecoin/testnet HRPs, Taproot and nested scripts.

It reconstructs a BIP143 SIGHASH_ALL message from every vin outpoint/sequence, exact 25-byte P2WPKH scriptCode, actual funded sats, all serialized outputs, selected first-vin sequence, locktime and version. The raw type9 OP_RETURN must also pass Stage25N's one-carrier source subset. Funding must precede its spend under the **same locally checked Bitcoin header segment**. Bitcoin PoW, transaction inclusion and computed digests are **relative to a user-selected anchor**, and the module neither validates mainnet consensus nor verifies secp256k1 signature mathematics itself.

Public-only fixture fixtures/channel_type9_p2wpkh_signed_synthetic.tsv contains one actually mathematically valid synthetic secp256k1 signature created offline with a throwaway fixture key. It spends a synthetic 20,000-sat native P2WPKH parent output and pays 17,000 sats change plus a zero-sat type-9 OP_RETURN. No private key, signing tool, real UTXO or actual block appears in the repository. A fabricated regtest-like header and arbitrary claimed ancestor carry both TXIDs as two leaves.

Independent Python-stdlib and **system OpenSSL** CLI in scripts/audit_type9_p2wpkh_openssl.py recomputes SHA256d, HASH160, Bech32, the BIP143 preimage/digest and synthetic Merkle root. It mathematically verifies the genuine DER signature and rejects forged signature and changed signing digest from outputs/amount. A signature-only witness mutation preserves the SegWit TXID and BIP143 digest while changing wtxid. Thus a TXID-Merkle proof does not itself authenticate SegWit witness contents. System OpenSSL is a **test-only** dependency, absent from Rust production consensus.

## Fail-closed obligations

UnverifiedP2wpkhType9FirstInput is a private-field research report. Its accessors expose the raw pubkey, HASH160, claimed sender address, funded satoshis, BIP143 digest, type-9 candidate and relative Merkle position.

- require_authenticated_sender() **always returns SenderSignatureAndScriptNotVerified**.
- require_historical_admin_permission() **always returns HistoricalListAdministratorNotVerified**.
- require_canonical_channel_revision() **always returns CanonicalStateRevisionUnspecified**.

A valid synthetic ECDSA signature does not prove real chain inclusion, a valid UTXO, full Bitcoin Script flags, BIP141 witness commitment, first-input ownership or that the alleged source sender actually administered a clearlist at that height. The historical list and admin are **not independently authenticated** by a caller-supplied expected address. Complete type-7/8/9 list state, country, banlist, xpub-derived witnesses, type-4 and type-19 economic context, source-compatible activation, rollback and the canonical two-signature off-chain revision/invalidation transcript remain separate blockers. The Stage25G liquidation authority remains unavailable; types3/23/31 and all actual Ledger/clearing mutations remain disabled.

## Reproduce

    python3 scripts/check_rust_supply_chain.py
    node scripts/audit_type9_source_paths.cjs
    python3 scripts/audit_type9_p2wpkh_openssl.py --check
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test channel_type9_sender_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen

Next gate: independently authenticated historical parent UTXO, actual Bitcoin full-block/active-chain/Script validation, first-input sender and type-9 administrator/activation from complete type-7/8/9 event replay, followed by a ratified signed channel-state revision/supersession and commitment invalidation protocol. Litecoin Scrypt and Dogecoin AuxPoW require their independent source adapters.
