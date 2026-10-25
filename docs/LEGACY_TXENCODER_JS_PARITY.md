# File 7/49 — txEncoder.js source-string parity

Baseline: TradeLayer-Rust main 4f552176, after File6/49. Historical source: patrickdugan/tradelayer.js@dca0888b827c5bc28fa9189bc9e1d507f5c5670c, src/txEncoder.js (684 lines), src/types.js and src/txDecoder.js. All three byte-exact Git blob archives are frozen in the repository from File5/49.

**Research-only source encodings; NOT a production transaction constructor, authenticated OP_RETURN, protocol admission or signed channel state.** No Rust Ledger, wallet, Registry, chain adapter, orderbook, clearing, AMM or insurance changes. Zero added third-party Rust crates.

## Source interoperability findings

| Source path | Exact behavior | Why it matters |
|---|---|---|
| Type0 activate | Encodes a type list joined by semicolon and a 256-bit hexadecimal code hash converted by BigInt to base36, with complete tl0 prefix | Encoding an activation is not proof of native token genesis or administrator signature |
| Type1 issue | Multiple whitelist IDs joined by literal commas, while decoder splits all commas first and expects one semicolon-delimited whitelist field | Multi-whitelist issue shifts managed/backup/other fields |
| Type2 send | sendAll and multi-array forms return body only; scalar source already returns tl2-framed string | Generic Types.encodePayload always prepends tl2, producing a double type prefix for scalar but not sendAll |
| Fractional amount | Source BigNumber scales by 1e8 but omits tilde suffix on fractional amounts because of nullish-coalescing precedence | An exact 0.5 token encodes as tro8w, which source decoder reads as 50,000,000 whole tokens |
| Type4 commit | Direct encoder is named encodeCommit and produces complete tl4; generic Types calls ABSENT encodeCommitToken | Generic encoding throws. Multi-clearlist array [7,b] is split on comma before decoder bracket detection and yields NaN / shifted fields |
| Type7/8 | Complete prefix, raw text fields, base36 admin target, Boolean flags | Caller fields are not verified addresses or historical administrators |
| Type9 attestation | Directly interpolates revoke: number 1 yields literal 1, Boolean true yields true | Source decoder only treats literal 1 as revoke; Boolean true becomes false |
| Type15 close oracle | Due to operator precedence, encodeCloseOracle ignores its ID and always returns tlf | Generic wrapper produces tlftlf. Types subsequently invokes the wrong publish validator on primitive decoder output |
| Type21 withdrawal | Uses decimal-scaled amount text and literal true/false string flags | Decoder treats amount as base36 and expects 1 for withdrawAll. One token encodes 100000000 decimal, decoded as 2821109907456 atoms; true decodes as false |
| Type22 transfer | Full tlm prefix; base36 atomic amount; destination >42 ASCII chars mapped to claimed ref:N | Source reference text is not UTXO or ownership authentication |
| Type30 fraud/stake/relay | Source 13 comma-separated fields with BigNumber amount scaled by 1e8 and ROUND_DOWN | No oracle authority or fraud proof follows from encoding |
| Types33–35 | Colored coin, bridge, smart-bind encoders return body-only; generic Types adds the selector | Type33 validator is commented out; Types34/35 dispatch to absent decoder exports |

## Rust implementation

New src/legacy_txencoder_audit.rs reconstructs the read-only string returned by 14 source type families: 0,1,2,4,7,8,9,15,21,22,30,33,34,35. Type2 supports sendAll, scalar and multi; Type4 supports scalar, absent and array clearLists, and long-address reference substitution. It performs the actual BigInt 256-bit hexadecimal-to-base36 conversion without any new crates.

The result UnverifiedSourceEncoding has private fields and exposes an exact SOURCE STRING for comparison. Its source_generic_types_concat_unverified method calls the existing File5/49 read-only Types generic concatenation diagnostic, which explicitly refuses Type4 because the missing source export is real. Caveats expose source vulnerabilities, not authenticated actions. Historical transaction, list/activation, co-signed revision and economic execution permission methods ALWAYS return errors.

Defensive differences from JavaScript are declared rather than silently repaired: amounts are exact 1e8-token atoms bounded to JS safe integer 2^53−1; lists and strings are bounded; strings must be printable ASCII; malformed/ambiguous unsupported inputs fail closed. These are not historical consensus rules. A successful source encoding is not send-ready or ledger-approved.

## Exact source tests

fixtures/legacy_txencoder_source_vectors.tsv contains 22 frozen inner/source-generic string pairs. scripts/audit_legacy_txencoder_source.cjs checks the exact archived Git object hashes and runs the original txEncoder.js, types.js and txDecoder.js code in an isolated Node VM with strictly mocked imports and no network, wallet, keys, actual DB or chain. It checks the 22 strings plus real original decoder responses to fractional send, multi-property send, Type1 whitelist drift, Type4 bracketed clearlist, Type9 Boolean revoke, Type15 ID loss and Type21 decimal/base36 mismatch. Rust tests/legacy_txencoder_audit.rs checks the same 22 strings, a true all-ones 256-bit hash, numeric bounds, strict rejections and permanent default-deny authorization.

    node scripts/audit_legacy_txencoder_source.cjs
    python3 scripts/check_rust_supply_chain.py
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test legacy_txencoder_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen

## Remaining production migration

Full txEncoder.js has approximately 40 methods. This implementation intentionally leaves Type3/5/6/10–14/16–20/23–29/31/32 constructions, Base94 and exact JavaScript non-integer/rounding edge cases, and real economic script/ref/UTXO verification for separate slices. Missing type4 generic export, unversioned prefix mixtures and incompatible Type15/21 round-trips need explicit source-versioned historical reconciliation, not silent changes in future consensus. TradeLayer still needs canonical two-signer channel revision bytes, monotone state numbers, supersession/invalidation/dispute/reorg and authenticated authority before any actual settlement. Issue #80 remains open.

**Next one-file turn: 8/49 txIndex.js**, for source ingestion and the known incorrect three-argument call into the nine-argument Types.decodePayload function. The seven PRs so far represent tested source-subset coverage, not seven complete production consensus ports.
