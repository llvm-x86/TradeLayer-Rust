# File 6/49 — txDecoder.js bounded typed source parity

Pinned upstream: patrickdugan/tradelayer.js@dca0888b827c5bc28fa9189bc9e1d507f5c5670c, src/txDecoder.js (659 lines). Its byte-identical Git-blob-verified archive already exists as fixtures/legacy_types_source_txDecoder.txt after File5/49. Baseline Rust main 761f4678.

This is a read-only, bounded implementation of selected **post-type-selector BODY decoders**, not a consensus parser. An independently authenticated historical transaction requires chain source, actual first-input sender and UTXO evidence, historical activation/list state, the *correct* versioned wire carrier, and economic and channel authorization. None is supplied here. The current crate-wide strict wire parser, Registry, Ledger, clearing, channels and market adapters are unchanged, with zero third-party Rust crates.

## Implemented source decoder subset

New src/legacy_txdecoder_audit.rs consumes explicitly selected source type number and caller-provided body text (NOT a serialized transaction or OP_RETURN). It returns an UnverifiedDecodedBody and **never an accepted transaction**.

| Source type | Source-pinned fields | Compatibility boundaries |
|---|---|---|
| 4 — Commit Token | Numeric or s-1-a synthetic property ID, 1e-8 token-atomic amount, literal direct channel or raw text after ref:, pay flag, singleton or bracketed clearLists, colored flag, optional commitClearlistId | The JS decoder FIRST splits every comma. For [7,b], the index holding the list becomes [7 and numeric parseInt yields NaN; following fields shift. Rust refuses explicit BracketedListSplitBeforeParsing. Missing parts[2] makes JS startsWith throw. The normal direct and singleton-list paths are characterized, not authorized. |
| 7 — Create Whitelist | Backup, name, URL, description with JS missing-field empty-string fallbacks | No independent address, list allocation or sender verification |
| 8 — Update Admin | New address, three literal ==1 flags, base36 target ID, updateBackup flag | No historic getList admin/list authority, in-memory DB continuity or address checks |
| 9 — Issue/Revoke Attestation | Only literal first field 1 is revoke, base36 list ID, target string, uppercase metadata | Calling parts[3].toUpperCase throws when fourth field absent. If caller passes raw post-marker type-selector 91 as the revoke field instead of 1, revoke becomes false. No claimed attestation may authorize clearlist execution |
| 15 — Close Oracle | Standalone primitive parseInt(base36) number | Source Types.js also calls the WRONG publish-oracle validator on this primitive. No oracle-close authority |
| 21 — Withdrawal | withdrawAll, property, 1e-8 atoms, source column flag, direct channel or ref with **decimal parseInt** and NaN -> false | Withdrawal numeric ref has different semantics from Type4/22 text refs. No queue mutation or payout |
| 22 — Channel Transfer | property, atoms, isColumnA flag, direct destination or raw text ref, pay and payRef | Missing parts[3].startsWith throws. No channel balance movement or whitelist approval |
| 30 — Oracle Stake/Fraud/Relay | 13 ordered source action, oracle, amount, actor, hash, DLC, relay and auto-roll fields | No actual proof verification, oracle state or validator permissions |
| 32 — ZK Batch Move | Original opaque ordinalRevealJSON | Types.js does not invoke the Type32 validator; no proof verification |
| 33 — Colored Coin | Decimal action, base36 property/ratio, home address | Types.js does not invoke the Type33 validator; no colored-UTXO/procedural authority |

Additional non-authorizing helper source_decode_property_id_unverified models both the source synthetic identifiers s-1-a and s1-a as collateral ID 1, contract ID 10. The standalone decodeAbstractionBridge helper is characterized separately, but the source Types.js Type34 dispatcher calls a DIFFERENT absent decoder named decodeCrossLayerBridge. No implicit alias is installed. The file also freezes the fact that a second, later base94-based decodeSettleChannelPNL declaration shadows the first base256-based declaration in the source JavaScript object literal. Actual Type23 economic settlement bytes are **not decoded or admitted** by this module.

## Independent pinned-source test evidence

fixtures/legacy_txdecoder_body_vectors.tsv freezes 17 shared direct body cases, including normal and erroneous inputs. scripts/audit_legacy_txdecoder_source.cjs independently checks the Git blob ID for the archived *exact historical source*, runs that source in an isolated Node VM with mock-only BigNumber/Base94/Base256 dependencies, and checks:
- Normal type4 direct, synthetic property/ref and one-member list, and the comma-bracket NaN misparse
- Actual Type4/22 indexed-field TypeError and Type9 missing metadata TypeError
- Type7/8/9 field booleans, id base36, metadata uppercase, and selector/body confusion
- Distinct Type4/22 **raw text** references versus Type21 **decimal number/false** reference
- Numeric oracle-close return and Type30 13-field mapping
- Type32/33 decoder values with no valid flag or invoked validator
- The later duplicate Type23 base94 decoder being called and the shadowed base256 method never invoked
- A separate source decodeSend multi-send shape which is intercepted by the scalar four-field branch, dropping multi-property values
- Source Type34 exported helper name vs missing dispatch method

tests/legacy_txdecoder_audit.rs exercises exactly the same 17 cases plus synthetic property forms, unsupported types, malformed numeric IDs, blocked oversized/Unicode/control text, ambiguous framed or nested payloads, missing source exports, the Type23 shadow and all permanent authorization denials. No historical source JS library is imported in Rust production, and no crypto credentials, RPC or live chain calls are used.

### Explicit defensive divergence from legacy permissive JS

The original decode methods often use parseInt with partial-string acceptance, JavaScript Number rounding/NaN, permissive Unicode or missing-field defaults. Rust currently supports only canonical whole-field base36 and decimal numbers, printable ASCII body text bounded by 4096 bytes, and safely representable 1e-8 atomic amounts at most 2^53-1. Malformed source values are **errors** and never auto-coerced into economic amounts. These are source-research safety restrictions, NOT equivalent historical consensus validation. The read-only result records caller-claimed fields and no source permission.

Authorization methods require_historically_accepted_transaction(), require_accepted_registry_transition(), require_canonical_signed_channel_revision(), require_trusted_execution(), and the standalone bridge helper's authorization method ALWAYS return an error, including for syntactically valid bodies.

## Remaining txDecoder.js port

The existing Rust wire parser covers limited earlier Type0/1/2/11/12 layouts. This stage adds the ten methods above for diagnostic parity. Remaining complete typed decoders include Type3 UTXO trade, Types5/6 spot and cancellation, Types10/13/14 AMM/oracle, Types16-20 derivative/channel trading, Types23/24-29 economics, Type31 King Settle and other source helper variants; many require base94/base256, full state, BigNumber corner cases and versioned syntactic structures. Type28/29/34/35 are ABSENT source method exports rather than functional decoding code to port. Strict standalone base36 amount, type2 multi-send, Type23 duplicated method and missing source validity flags need their own compatibility/audit decisions before any production handler is enabled.

The one-file-per-turn sequence next addresses **7/49 txEncoder.js**. It must independently verify source inner-prefix variants and malformed numerical encodings before choosing a new canonical wire format. Never let a read-only decoder report or apparent source JavaScript decoded object enter a trusted ledger, margin engine, chain reorg or off-chain channel settlement without independent proofs and ratified protocol versioning. Issue #80 remains open.

## Offline acceptance

    node scripts/audit_legacy_txdecoder_source.cjs
    python3 scripts/check_rust_supply_chain.py
    cargo fmt --all -- --check
    cargo clippy --all-targets --frozen -- -D warnings
    cargo test --frozen --test legacy_txdecoder_audit
    cargo test --frozen
    cargo build --all-targets --release --frozen

No new Rust crates, wallet keys, nodes, signet transactions, mainnet funds or production execution.
