# Stage 15: TLPT-to-zkTL cross-namespace audit reconciliation

Stage 14 commits Rust TLPS snapshots with TLPT v1 metadata. Earlier
zkTL tooling checks a JavaScript-generated Cairo batch receipt chain.
These are **different state machines and different root namespaces**.

This stage implements an offline alignment audit that ties both sets
of local metadata to the *same externally claimed block heights and
transaction IDs*. It makes discrepancies visible and keeps all
economic, L1 and proof-verification claims explicitly UNVERIFIED.

Passing this audit is **not** evidence that the transactions occurred,
that TradeLayer accepted them, that a signer had authority, that a
Cairo transition executed, or that STWO proofs were verified.

## Inputs and bounds

The CLI consumes three files:

1. TLPT v1 rows: lower-case 64-character TLPT ID, TAB, lower-case
   554-character encoded 277-byte TLPT statement; one per block.
2. The existing TLZK1 bridge TSV exported by export_tlzk_batch.py.
   It carries ordered transaction receipts and zkTL root metadata.
3. Observed receipt rows: O, height, block hash, transaction ID,
   transaction type, and zero-based order of *zkTL-relevant protocol
   receipts* inside that block, separated by tabs.

The observation file is an **unverified declaration**, even if its
fields originated with a node-backed collector. Source authentication
must be performed independently before such observations can become
trustworthy. The CLI also requires expected chain namespace, previous
anchor height/hash, and final height/hash as arguments. Those values
are assertions from the caller, not an L1 best-chain proof.

At most 500 blocks, 10,000 receipts, and 4 MiB per text file are
admitted, with a 2 MiB limit on zkTL bridge input.

## Checked internal consistency

The read-only reconciliation checks:

- The complete zkTL batch passes its existing canonical JSON digest,
  state-root-chain, receipt ordering, support level and tx34 guards.
- Exactly one valid TLPT statement exists for each height covered by
  the batch, with no omissions or duplicate heights.
- All TLPT records share the configured chain ID; consecutive
  parent/child tips and Rust before/after roots match exactly.
- The first TLPT parent matches the separately supplied anchor, and
  the last child matches the supplied terminal tip.
- For each zkTL receipt, the matching declared observation has the
  same height, block hash, transaction ID, transaction type and
  within-block receipt position.
- zkTL's old/new root chain advances in receipt order. For a block
  with no zkTL receipts, the zkTL logical root remains unchanged.
  The Rust TLPS root can change independently, for example when the
  finalized block height advances.
- A domain-separated SHA-256 audit digest commits the configured chain
  interval, zkTL batch ID, both root endpoints, every TLPT statement
  ID and per-block root pair, and every matched observation and receipt
  ID.

**Critically, no equality is asserted between Rust TLPS roots and
zkTL Merkleized state roots.** These represent different data. A
legitimate cross-implementation equivalence proof requires a
separately reviewed semantic map of properties, channels, supply,
settlements and state roots.

## Synthetic runnable example

CI generates deliberately *invented* block IDs, Rust protocol roots
and observations that borrow the 26 receipt IDs and transaction
metadata from the repository's zkTL sample. This tests transport and
comparison only. It is not a historical TradeLayer trace.

From the repository root:

    scratch="$(mktemp -d)"
    python3 scripts/make_synthetic_tlpt_bridge.py "$scratch"
    cargo run --locked --bin tradelayer-root-reconcile -- \
      "$scratch/synthetic_tlpt.tsv" \
      fixtures/zkTL_sample_batch.tsv \
      "$scratch/synthetic_observations.tsv" \
      "$(printf '0b%.0s' {1..32})" \
      132899 "$(printf '13%.0s' {1..32})" \
      132907 "$(printf '1b%.0s' {1..32})"

Expected output includes:

    root_alignment=AUDIT_ONLY
    chain_consensus=UNVERIFIED
    transaction_inclusion=UNVERIFIED
    economic_validity=UNVERIFIED
    stwo_proof=UNVERIFIED
    root_namespaces=SEPARATE
    blocks=8
    receipts=26

A fully fabricated but self-consistent transcript also passes those
checks. The example intentionally demonstrates that limitation.

The Rust test suite verifies the source-derived 26-receipt fixture
against eight locally reversible synthetic TLPT blocks and includes
failures for missing heights, foreign chains, wrong anchors,
discontinuous Rust states, forged observations, swapped transaction
order, malformed TSV, and corrupted zkTL metadata.

Stage 16 adds a [separate partial Bitcoin SHA256d header and transaction
Merkle checker](BITCOIN_SPV_INCLUSION.md). A new reconciliation entry
point can consume its opaque checked transaction-inclusion objects
instead of freely supplied IDs or ordering. The header anchor, network
selection, consensus difficulty, best chain, scripts and economic
validity are not established by that checker.

## Production path still required

The next step is an externally authenticated per-block observation
source that verifies L1 consensus, active best-chain status, Merkle
transaction membership, UTXO/script signatures, sender selection
and protocol payload decoding. For Litecoin, the existing node-backed
collector gives a starting read-only provenance trail but cannot
yet prove all of these boundaries. For Bitcoin-family roots, Raito
and STWO need their real verifier implementations.

To prove Rust and zkTL economically equivalent, define an explicit
namespace-by-namespace mapping plus witness constraints linking
each accepted protocol operation to both before/after states. Verify
those proofs against independently authenticated public inputs.
The type32/type34 opcode conflict also remains unresolved.

This stage does not change protocol execution, account balances,
checkpoint encoding or the Stage 13 default-deny economic gate.
