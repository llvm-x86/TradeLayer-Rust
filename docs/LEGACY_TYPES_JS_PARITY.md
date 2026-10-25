# File 5/49 — types.js historical dispatch parity

Pinned JS source: patrickdugan/tradelayer.js@dca0888b827c5bc28fa9189bc9e1d507f5c5670c, specifically src/types.js (496 lines), txEncoder.js, txDecoder.js, txIndex.js and logic.js. Baseline Rust main 91f4872c. This PR implements **read-only dispatch research**, not a fully activated transaction parser or consensus validator.

## Complete 36-type wiring

fixtures/legacy_types_routes.tsv has an exhaustive indexed table of all 36 transaction IDs, actual encoder/decoder names and their exported presence, plus the ACTUALLY INVOKED validity callback. All three underlying pinned JS source blobs are byte-preserved in fixture files with their expected Git blob SHA1 values. Independent scripts/audit_legacy_types_dispatch.cjs verifies those blobs, parses the source switch arms, checks every method export and runs actual Types methods in a VM with **mock-only dependencies**. No wallet, third-party JS modules, live validity logic, RPC, DB or network is allowed to run. Its validity callbacks are spies used only to check control-flow wiring; a true mock result has no protocol meaning.

New Rust src/legacy_types_audit.rs independently freezes those routes in a compile-time 36-entry constant. It reports what a claimed source dispatch would invoke, classifies generic envelope concatenation (one prefix, duplicated same-type prefix or foreign nested prefix) and reproduces Types.resolveReferenceAddress ordering. These are observations, not actual wire decoding or authorization. Every capability method always returns an error. The existing strict Rust wire parser, registry, Ledger, clearing and channel code remain untouched.

## Important pinned source discrepancies

| Type or path | Exact source finding | Safety boundary |
|---|---|---|
| Generic encoding | Types.encodePayload FIRST prepends tl plus base36 selector, then concatenates the selected txEncoder return unchanged. Some inner encoders already prepend the same marker/type; others return body-only bytes | Never use this observation as send-ready or trusted consensus bytes |
| Type 2 | sendAll inner result is body-only, yielding tl21;alice; scalar send independently prepends tl2, yielding nested tl2tl20;alice;5;2 | Source wire ambiguity, no automatic fallback |
| Type 4 | Types calls missing encodeCommitToken although txEncoder exports encodeCommit | Missing method, not a valid generic encoding |
| Type 9 | Direct encoder emits tl90,7,alice,OK; generic Types emits tl9tl90,7,alice,OK | Distinct paths; no source-independent nested repair |
| Type 11 | Reference helper prefers nonempty string; first nonempty array entry.address; object.address before object.senderAddress | Only a claimed output recipient, not ownership/authentication |
| Type 15 | Generic encoder calls encodeCloseOracle WITHOUT the oracle ID; historical encodeCloseOracle returns tlf, giving generic tlftlf. decodeCloseOracle returns a primitive integer, replacing the earlier object carrying block. Types wrongly invokes validatePublishOracleData, not validateCloseOracle | Misrouted validator and missing block remain hard blockers |
| Type 3 | If both payment/delivery reference vouts do not exist, Types returns decoded fields without invoking validateTradeTokenForUTXO. Absent reference array can throw before validator. Source has separate tagWithdraw identifier and validity-reset issues | No missing-reference admission or amount/price authority |
| Types 28, 29 | Referenced decoder exports for Bai Urbun and Murabaha do NOT exist | Missing-method throw, no contract acceptance |
| Type 23 | Source sets channelAddress equal to caller-supplied sender and constructs a nested txParams object | Not a co-signed state revision |
| Type 31 | Types dispatches King Settle, but older activation and logic naming diverge | Do not infer settlement permissions from number |
| Types 32, 33 | Actual decoders exist; validator invocation is commented out. Results may have valid undefined, and downstream logic.js only blocks when valid == false | Decoded-without-validator is NEVER acceptable as authority |
| Types 34, 35 | Types invokes missing decodeCrossLayerBridge and decodeSmartContractBind. For 34, the exported decoder is named decodeAbstractionBridge, and the type number conflicts with zkTL interpretations | No implicit alias or protocol activation |
| TxIndex handoff | txIndex.processTransaction passes only 3 arguments to the 9-argument Types.decodePayload. The marker and type arguments shift, commonly yielding a returned Error object rather than a thrown exception. The full signature also forwards encodedPayload.substr(0) without stripping the selector. Thus raw after-tl payload 91,7,alice,ok changes the revoke field compared with the post-type body 1,7,alice,ok | Caller contract, method type, carrier and source version must be proved independently |

The source dispatch map has validators invoked for type IDs 0–31 except that Type3 may skip validation when references are absent and Type15 invokes the wrong validator. IDs 32–35 contain commented validity calls; IDs 34 and 35 also name absent decoders. A validator being invoked is not evidence that it returned valid=true, authenticated the sender or had correct state.

## Exact test contract

The archived original files are verified by their Git SHA1 object IDs (types.js fb894e8a25d560cc9481f9913aedf003fdde382f; txEncoder.js 94201b4eaedcd240b9a9f6d54d11b55206615dff; txDecoder.js 4728f1bdce1190428f91ae5b02f380584073c431). The source VM has only mocked imports and tests deterministic branch selection, not economic effects. The compiled Rust tests verify each of the 36 route records against the TSV, Type2/9/15 prefix behavior, Type4 encoder absence, Type3 skipped validation, Type15 wrong validator, types28/29/34/35 missing decoders, types32/33 unvalidated returns, TxIndex three-argument mismatch, type11 reference precedence and permanent denial.

    node scripts/audit_legacy_types_dispatch.cjs
    python3 scripts/check_rust_supply_chain.py
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test legacy_types_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen

Zero new Rust crates, signing keys, node RPC, actual chain inputs, testnet/mainnet funds or trusted transaction state changes.

## Remaining migration steps

types.js full production equivalence requires source-versioned and independently authenticated input and chain/UTXO sender; complete txDecoder and txEncoder implementations; correct mapping of parsed fields to actual verified validators; synchronous/async method and exception compatibility; safe handling of historical type 3, 15, 23, 31–35; ratified fixes for missing exports and nested prefixes; full list and oracle history; and a canonical co-signed channel-state revision/invalidation and dispute protocol. This is the dispatch metadata and fail-closed interface, **not** a completed validated transaction interpreter. Next one-file turn is txDecoder.js followed by txEncoder.js. Keep Issue 80 open.
