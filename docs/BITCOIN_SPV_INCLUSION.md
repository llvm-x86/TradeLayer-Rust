# Stage 16: Bitcoin SHA256d header and transaction inclusion proofs (offline)

This stage introduces a *bounded SPV-style evidence checker* for the
Bitcoin SHA256d proof-of-work family and connects its checked transaction
identities to the Stage 15 TLPT/zkTL alignment audit.

**This is NOT a full Bitcoin consensus verifier, production wallet backend,
trusted chain adapter, or economic authorization path.**

## What the Rust module checks

- Exact 80-byte Bitcoin-style block headers and independent SHA256d
  block identifiers, in the correct internal/display byte orders.
- Positive, minimal, nonnegative, 256-bit compact nBits targets, rejecting
  invalid encodings and targets above a declared Bitcoin-mainnet-like or
  Bitcoin-regtest-like proof-of-work limit.
- Header SHA256d against that declared target. The profile checks a
  **maximum permitted target**, NOT the actual mainnet/testnet difficulty
  adjustment rules at the claimed height.
- A contiguous parent-block-hash chain for up to 500 headers from an
  explicitly supplied anchor. That anchor is a **caller assertion**.
- Non-witness transaction IDs computed directly from strict Bitcoin
  transaction serialization; SegWit transactions strip witness data when
  computing their Merkle leaf.
- Exact non-witness txid Merkle inclusion against the corresponding
  checked header root, with validated transaction count, tree index,
  sibling depth, odd-leaf self-padding, and duplicate real-sibling
  mutation rejection.
- An optional syntactic type hint derived from **one** legacy TradeLayer
  OP_RETURN carrier with a successfully parsed typed body. Proposed
  binary-v1 carriers and invalid/ambiguous legacy bodies are not
  promoted to type evidence.

A published Bitcoin-mainnet genesis header and coinbase from the existing
historical fixture provide a genuine pinned known-answer test. More
complex header-chain, multisibling, SegWit and zkTL-alignment tests
use **synthetic** regtest-PoW-like blocks mined under permissive targets.
They are not asserted to be blocks from any active Bitcoin network.

## Stage 15 integration

The new reconcile_checked_bitcoin_inclusions_unverified API accepts
opaque IncludedTransaction objects created only after the parser and
Merkle checks. Unlike the earlier audit-only path, callers cannot supply
their own txid, transaction type, or receipt position in this API.

For each zkTL receipt, the new path:

- Matches the checked header and canonical txid and derives the type
  hint from the raw on-chain carrier.
- Enforces **strictly increasing full-block transaction indices** for
  each block; unrelated transactions may exist between relevant ones.
- Requires all reported relevant receipts in one block to share its
  total transaction count.
- Performs all Stage 15 block/Rust TLPT root and zkTL receipt-root
  continuity checks.
- Computes an additional domain-separated SHA256d-inclusion *audit*
  digest binding the earlier report, header commitments and full-block
  transaction positions/counts.

This digest is **not a validity proof** or new state root. The previous
caller-observation API remains explicitly named reconcile_unverified for
offline comparison; the economic state machine is unchanged.

## Critical limitations and why they matter

A fabricated but self-consistent Bitcoin-regtest-like header chain can
satisfy every hash, nBits ceiling, Merkle and parent-link check here.
There is no cumulative chainwork or active-best-chain selection,
difficulty retargeting, median-time-past, block transaction validity,
script/signature or UTXO authorization. A caller-supplied anchor does
not become trusted merely because descendants link to it.

Litecoin and Dogecoin use Scrypt rather than Bitcoin SHA256d proof of
work, so **neither chain's PoW is validated** by this module. Their
transaction Merkle algorithm may be similar, but no Litecoin/Dogecoin
chain identity can be promoted into the Bitcoin SHA256d profile. A
future adapter must implement or trust an audited appropriate validator.

The project's bundled SHA-256 implementation has published known-answer
tests but is not independently security audited. Actual Raito/Cairo/STWO
economic transition and L1 inclusion proof verification, activation
rules, chain selection, trade provenance and signer authority remain
external blocked requirements. Merkle inclusion is not proof of a
TradeLayer transaction's validity or authorization.

## Test and future archive workflow

    cargo test --locked --test bitcoin_inclusion
    cargo test --locked

Use a real verified node or historical archive to collect complete
raw 80-byte header sequences, canonical raw transactions, original
block indices and Merkle branch siblings. Retain a declared network
profile and authenticated checkpoint in provenance. For chain
identities other than Bitcoin SHA256d, use chain-specific validation
instead of this header module.

Before production, couple an audited consensus checker and
independently trusted active-chain checkpoint to the evidence format,
validate all required script/UTXO and TradeLayer sender semantics,
reconcile the Rust/zkTL state namespaces, and verify actual Cairo/STWO
proof bytes. Only then could this evidence support the existing
default-deny settlement authorization gate.
