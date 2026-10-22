# Legacy TradeLayer wire-format reconnaissance

Scope: the `tl` ASCII envelope and syntactic decoding of transaction types **0, 1, 2, 11, 12**. The Rust implementation deliberately stops before consensus rules, wallet/network code, or funds handling.

## Source revision

Pinned JavaScript snapshot: [`patrickdugan/tradelayer.js@dca0888`](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c) inspected October 22, 2026.

- `src/txEncoder.js` (blob `94201b4eaedcd240b9a9f6d54d11b55206615dff`)
- `src/txDecoder.js` (blob `4728f1bdce1190428f91ae5b02f380584073c431`)
- `src/types.js` (blob `fb894e8a25d560cc9481f9913aedf003fdde382f`)
- `src/txIndex.js` and `src/main.js` (indexing and dispatch)

The [fixture generator](../scripts/generate_vectors.mjs) transcribes the selected encoder expressions without importing any application modules or npm packages. It is an **offline reconstruction**, with deliberately simple numeric inputs that avoid ambiguous JavaScript floating-point rounding. Regenerate with:

```sh
node scripts/generate_vectors.mjs > fixtures/wire_vectors.tsv
node scripts/generate_vectors.mjs --check
```

The fixtures are source-derived examples, **not historical chain transactions or fully independent differential-test oracles**. Live corpus and cross-language differential tests come later.

## Envelope

The block processor decodes the OP_RETURN content to ASCII and strips the first `tl` marker; the first remaining character selects transaction type in base 36. A direct application payload therefore has the form:

```text
tl<one base-36 transaction type character><type-specific ASCII body>
```

The Rust `parse_frame()` checks printable ASCII and a defensive 4,096-byte bound, returns a borrowed body, and recognizes type IDs 0-35. It has no Bitcoin transaction or OP_RETURN script decoder yet. Uppercase base-36 selectors are accepted because JavaScript `parseInt(..., 36)` accepts them.

`parse_typed()` further decodes only types 0, 1, 2, 11, and 12. An unimplemented type returns `UnsupportedType` explicitly. This parser does **not** determine whether a transaction is valid, activated, authorized, signed, or canonical for consensus.

## Selected layouts

| Type | Direct `txEncoder.js` layout | Arithmetic |
| --- | --- | --- |
| 0 | `tl0<decimal type IDs separated by ;>,<code-hash base36>` | Type IDs decimal in the activation body |
| 1 | `tl1<initial amount base36>,<ticker>,<whitelists>,<managed>,<backup>,<nft>,<colored>,<procedural>` | Initial amount in whole token units |
| 2 | `tl20;<recipient>;<property ID base36>;<amount>` | Whole token base36 amount, or atomic units with a trailing `~` |
| 11 | `tlb<property ID>,<atomic amount>,<recipient>,<legacy hash>,<DLC template>,<DLC contract>,<settlement>,<DLC hash>` | Amount always expressed in 1e-8 atomic units |
| 12 | `tlc<property ID>,<atomic amount>,<DLC template>,<DLC contract>,<settlement>` | Amount always expressed in 1e-8 atomic units |

For type 2, the intended body `1;<recipient>` represents send-all. The direct legacy encoder returns this body **without** a `tl2` prefix; `tl21;<recipient>` is included as a separately labeled constructed fixture and remains unvalidated as an on-chain encoding.

## Legacy discrepancies to resolve

1. **Duplicated envelope.** `src/types.js::encodePayload` prepends `tl` and the type ID, then appends a per-type encoder result. Several per-type encoders already return the same prefix; some `src/txUtils.js` builders also add a prefix. For example, a direct type-2 send `tl20;alice;5;2` becomes `tl2tl20;alice;5;2` through the generic wrapper. The typed Rust parser rejects recognized nested headers pending a compatibility decision. Numeric fields can themselves begin `tl...` in base 36, so this rejection also conservatively excludes some otherwise syntactically possible values.
2. **Missing decimal marker.** In `encodeSend` and `encodeAmount`, the JavaScript expression `scaled.integerValue()?.toString(36) ?? '0' + '~'` appends `~` only when the left operand is nullish. For one atomic unit (0.00000001 tokens), the emitted type-2 payload ends in `;1`. `decodeSend` interprets the same bytes as **one whole token** (100,000,000 atomic units). Rust preserves both byte-level interpretations in tests and does not claim that the source encoder intended this transfer.
3. **Whitelist layout mismatch.** `encodeTokenIssue` joins whitelist IDs using commas, while `decodeTokenIssue` splits the whitelist field using semicolons. A multi-whitelist issue shifts subsequent fields. The Rust typed parser rejects the nine-field example rather than guessing intent.
4. **Missing send-all and multi-send envelope.** `encodeSend` can return `1;<address>` or a `0;;...` body directly in special branches. Callers must supply a prefix for a valid envelope; the original code takes multiple paths.
5. **JavaScript number precision.** Some decoders feed `parseInt(..., 36)` into `Number` or `BigNumber`, so large values may lose precision before other checks. Rust parses into checked integers. Consensus-equivalence bounds and handling of noncanonical numeric spellings remain open.

## Review gates before expanding

- Collect bytes from real previously accepted Litecoin/Bitcoin/Dogecoin transactions with chain/network and block height recorded.
- Extract actual JavaScript decoder outcomes from a **sandboxed** legacy environment and compare at the transaction-field level.
- Decide how to handle double-prefix transactions, fractional transfers, large base-36 integers, and multi-whitelist layouts.
- Only after those decisions, implement strict, versioned encoding and state transitions.

All special cases are tracked as tests and documentation. This release provides parsing infrastructure for protocol archaeology, rather than deployable consensus code.
