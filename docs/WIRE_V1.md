# Proposed TradeLayer v1 binary envelope

**Status: proposal and parser testbed; no mainnet or testnet activation.** This document does not authorize broadcasting transactions or modifying existing consensus. The historical JS payloads and any accepted chain history need a separate, read-only replay path.

## Motivation

The legacy JS implementation reads a single base-36 character after lowercase `tl` to select a transaction type, sometimes adds the marker twice, and mishandles decimal suffixes. These are bugs. The Rust port will not copy them into its new serialization format. Existing legacy bytes remain evidence when checking historical network state.

## Frame

```text
byte offset  size  meaning
0            3     ASCII uppercase "TLR" (54 4c 52)
3            1     version = 0x01
4            1     numeric transaction type, 0..=35
5            N     opaque type-specific binary body
```

- Version and type occupy fixed-width bytes. There is no base-36 selector, nested envelope, or automatic fallback to historical `tl`.
- Reject unknown types (`>= 36`) and unknown versions. Unknown *known* type bodies require their own validation before state application.
- Defensive upper bound in the stand-alone parser: 4,096 bytes, including header. Actual block acceptance and carrier size limits must be specified per supported chain.
- The new marker's case intentionally differs from lowercase `tl` to avoid dispatching messages to legacy nodes under the previous protocol.

## Type 2: send

First byte of the body selects a variant:

| Variant | Fields (big-endian where multi-byte) |
| --- | --- |
| `0x00` direct transfer | `u32 property_id` · `u64 amount_atoms` · `u8 recipient_length` · address bytes |
| `0x01` send all | `u8 recipient_length` · address bytes |

`amount_atoms` counts indivisible units (1 token = 100,000,000 atoms). A direct send requires a positive property ID and amount. Recipient encoding is 1–90 printable, non-space ASCII bytes. The parser requires exact declared length and rejects trailing data. Address validation, send-all semantics, vesting restrictions, issuer authorization, and ledger transitions are **separate** from wire decoding.

**Example, send one atomic unit of property 5 to `alice`:**

```text
54 4c 52 01 02  00 00 00 00 05 00 00 00 00 00 00 00 01 05 61 6c 69 63 65
\_______/ \_/ \_/  \_/ \_________/ \_____________________/ \_/ \____________/
 marker    ver typ  direct  prop=5       amount=1 atom           len   alice
```

The fully concatenated hex value is:

```text
544c5201020000000005000000000000000105616c696365
```

Type 2 send-all to `bob`: `544c5201020103626f62`.

## Compatibility and rollout

The Rust legacy `wire` module is retained for **diagnostics only**. It can explain historical transaction bytes, including malformed encoder output. The v1 parser accepts exactly its own header; code that changes balances must explicitly select a protocol version under activation rules and should never guess the version from malformed input.

Before implementing state transitions or broadcasting v1, define an activation plan for each chain, verify historical accepted transactions against actual blocks, and formalize type-specific message schemas, reorg rules, and address validation. A new format without a coordinated activation mechanism can create consensus divergence.

## Reproducibility

`node scripts/generate_v1_vectors.mjs --check` validates 15 proposed v1 fixtures using only Node built-ins. `cargo test --locked` checks Rust parsing against those bytes and adversarial cases. Fixtures are deterministic proposals, **not on-chain attestations**.
