# Lean audit of Bitcoin Core PR #36629

Upstream: https://github.com/bitcoin/bitcoin/pull/36629
Pinned source head: cf9b981fcc7ae52c7864beae35df5d60b5c322b4
Local proof toolchain: leanprover/lean4:v4.34.1

## Reproduce

Change to formal/bitcoin_core_36629 and run:

    lake build --wfail

The dedicated Bitcoin Core script-index Lean proofs workflow performs this
build in GitHub Actions and rejects proof placeholders.

## Scope

The proof code is a functional model extracted from the upstream implementation.
It is not a C++ to Lean refinement proof, a verified C++ binary, or an audit
of every Bitcoin Core API boundary.

IndexModel.lean contains theorems for full hash filtering of prefix candidates,
completeness for available block transactions, matching-output index membership,
fresh-height append and erase rollback, conditional height-range inclusion,
and conserved charged byte budgets. A checked collision witness proves
that a pruned block candidate need not contain the requested script hash.
An arithmetic theorem shows how physical I/O can exceed the accounted
byte budget when block and undo reads are excluded.

OrderedEncoding.lean models the five-width Bitcoin Core OrderedVarInt byte
layout and its inverse. Its universal roundtrip theorem applies to
all natural numbers from 0 to 34,630,287,487 inclusive. Additional checked
vectors cover width cutovers, a four-billion encoding, uint32 maximum,
maximum five-byte value, and rejection of malformed prefix and truncated
representations. Cutover ordering tests are also formalized.

## Assumptions and remaining obligations

- Script hashes are represented by natural numbers. The truncated hash prefix
  is an arbitrary function that can collide. Neither SHA-256 nor SipHash is
  modeled cryptographically.
- The exact result theorem assumes a complete synchronized index and available
  block bytes. A pruned-block return is a prefix candidate without the
  complete script-hash check.
- The index model flattens per-prefix and per-height offset vectors to
  logical rows. Delta-encoded DBValue serialization and its C++ soundness
  have not been mechanically connected to this representation.
- A global proof of strict lexicographic monotonicity between any two
  encoded integers remains a separate proof obligation unless and until
  a corresponding theorem is added to OrderedEncoding.lean.
- Disk flushing, crash recovery, reorg races, file reads, iterator behavior,
  C++ memory safety, RPC spentness, mempool enumeration, and descriptor
  handling are outside this model.
- The nominal work budget charges index key/value serialization and candidate
  transaction bodies. Full block and undo reads can add physical work.
  An upstream Bitcoin Core reviewer independently reported this distinction.

## Review implications

Document pruned blocks as potential matches requiring later block retrieval,
preserve exact-hash filtering after prefix selection, and describe the 50 MiB
limit as an accounted-work threshold until all physical reads are charged.
Keep index rollback and full transaction offsets intact during refactoring.
Treat the pinned upstream head as part of the proof provenance.

Source files reviewed:
- src/index/ordered_varint.h
- src/index/txoscriptindex_key.h
- src/index/txoscriptindex.cpp
- src/rpc/blockchain.cpp
- src/test/txoscriptindex_tests.cpp
- src/test/ordered_varint_tests.cpp

No production Rust or C++ source files are changed by this proof package.
