# Stage 14: Domain-separated Rust state and transition commitments

Stage 14 adds a read-only format for binding a local TradeLayer-Rust state
projection to a specific chain namespace and block tip. The goal is a
reproducible public-input candidate for future zkTL/Cairo/STWO integration.

**These hashes do not prove consensus, trade authorization or economic
correctness.** The independent chain adapter and Stage 13 settlement
economic verifier remain production requirements. This crate's own
dependency-free SHA-256 implementation has known-answer tests but has
not been independently security audited.

## Three commitment layers

1. Protocol root: SHA-256 over a domain tag, unsigned 32-bit big-endian
   canonical snapshot length, and complete canonical TLPS snapshot bytes.
   The TLPS bytes include a CRC32 for accidental damage, which becomes
   part of the hashed representation. Registry, ledger, global replay
   markers, finalized block height, channel escrow and settlement
   journals are all included when present.

2. Chain head root: SHA-256 over a separate domain tag, the 32-byte
   chain namespace, eight-byte big-endian current height, 32-byte block
   identifier and 32-byte protocol root. Independent chain validity
   remains the responsibility of the authenticated adapter.

3. Transition statement: bound old/new roots and tips, for two locally
   reversible states. A successor must have height parent+1 and its
   retained undo entry must restore the previous tip and entire protocol
   state. No external L1 inclusion or cryptographic economic proof
   follows from this local lineage check.

Domain tags:
- Protocol: TLR_PROTOCOL_STATE_V1 followed by a zero byte
- Chain head: TLR_CHAIN_HEAD_V1 followed by a zero byte
- Transition magic: TLPT plus 0x01

The distinct tags are deliberate. None of these roots purports to
equal zkTL's existing multi-namespace state roots or its Cairo output.

## Fixed TLPT v1 binary public-input candidate

TLPT v1 is exactly **277 bytes** in network byte order:

| Field | Bytes |
| --- | ---: |
| Magic TLPT + version 1 | 5 |
| Chain namespace | 32 |
| Old block height | 8 |
| Old block ID | 32 |
| New block height | 8 |
| New block ID | 32 |
| Old TLPS protocol root | 32 |
| New TLPS protocol root | 32 |
| Old chain-head root | 32 |
| New chain-head root | 32 |
| SHA-256 of the 245 preceding bytes | 32 |

Parser checks exact length, magic, consecutiveness, distinct block IDs,
the two redundant chain-head digests and the final statement digest.
All fields can be authored and rehashed by anyone. They are auditable
metadata, not trusted or signed attestations.

The Rust TransitionStatement constructor requires actual successive
UnifiedChainState projections whose local rollback restores the earlier
state. The typed API can produce a stable canonical byte record and
parse untrusted records with internal consistency checks.

## Independent cross-language fixture

The Python standard-library script in scripts/generate_tlpt_vectors.py
constructs an independently calculated SHA-256 reference vector for
TLPT v1. Its output lives in fixtures/tlpt_v1_vectors.tsv, with the
expected statement ID and complete canonical wire bytes.

The Rust integration test parses the same wire, recomputes both head
roots, checks the statement ID and round-trips every byte.

Run:

    python3 scripts/generate_tlpt_vectors.py --check
    cargo test --locked --test state_commitment

Further tests compare state roots before/after a tip-only block,
checkpoint save/restore, an alternate fork and wrong ancestry/network
failures. Legacy TLL v1/v2/v3, TLPS v1/v2 and TLUC checkpoint file
formats are left byte-for-byte unchanged.

## Proving and authentication boundaries

A real zkTL verifier must first reconcile these Rust TLPS and
channel/settlement journal roots with its economic state namespaces.
It must then prove authorized state transitions, not only metadata
hash equality. Verify the actual Cairo/STWO proof bytes, the chain
of verified L1 headers and UTXOs, relevant signed trades, the before
and after roots, all public inputs, and settlement authorization
through the Stage 13 adapter gate.

For the moment TLPT is a deterministic experimental handoff format.
No changes to active consensus, balances or economic proof acceptance
are made by this stage.
