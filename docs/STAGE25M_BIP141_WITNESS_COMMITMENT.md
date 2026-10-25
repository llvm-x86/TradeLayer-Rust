# Stage 25M: bounded Bitcoin BIP141 witness-commitment evidence

**Research diagnostic only.** The public `UnverifiedBitcoinWitnessBlock` is
private-field and `require_authenticated_channel_spend()` always returns
`ActiveChainAndChannelAuthorizationUnavailable`. It cannot be passed to any
trusted chain adapter, channel settlement, margin, liquidation or live-funds path.

## Witness commitment evidence

The checker consumes a bounded **complete ordered transaction list** and a
locally checked Bitcoin SHA256d header segment. It verifies the non-witness
transaction-ID Merkle root against the header's root, rejects duplicate txids,
and recomputes Bitcoin's witness Merkle root with the coinbase's witness leaf
fixed at **32 zero bytes**. True odd-width Merkle padding duplicates the final
leaf; a real repeated pair/subtree is rejected as mutation.

The first transaction must have a canonical one-input coinbase prevout and
coinbase scriptSig length 2..100 bytes. Its sole witness stack item must be
exactly 32 reserved bytes. From its outputs the checker selects the **highest
matching index** with the BIP141 commitment prefix
`OP_RETURN PUSH36 0xaa21a9ed <32-byte-commitment>`, allowing trailing script
bytes as specified by BIP141. It checks:

```text
SHA256d(witness_merkle_root_raw || coinbase_reserved_value_32)
  == committed_witness_hash
```

The selected non-coinbase transaction must carry SegWit witness bytes. The
report provides its raw txid/wtxid and both checked roots for **untrusted**
cross-reference with Stage25L's P2WSH funding/spend Merkle report.

This subset caps the number of supplied transactions and aggregate serialized
bytes. It checks canonical Bitcoin-style transactions, rejects a second
coinbase, invalid block heights and unsupported Litecoin/Dogecoin network
routes, and preserves the existing dependency-free Cargo policy.

## Independent adversarial fixtures

`scripts/audit_bip141_witness_fixture.py` uses only Python's stdlib
`hashlib` to rebuild and verify the frozen
`fixtures/stage25m_bip141_vectors.tsv`. The underlying Stage25J public-only
synthetic P2WSH spending transaction is paired with a fabricated coinbase and
regtest-like Bitcoin SHA256d headers. These are deliberately not valid fully
mined Bitcoin blocks. The fake coinbase's unrelated BIP34/UTXO details are
outside this scoped diagnostic.

The Rust suite independently checks the fixture's fixed expected roots,
higher-index commitment precedence, trailing bytes, altered coinbase reserved
value, malformed CompactSize, wrong block height and orphaned header, three
leaf odd-tree padding, reordered/omitted/duplicated transactions and
unsupported networks.

**Falsifier:** changing one DER-shaped signature byte preserves the spending
**txid** and Stage25L's txid-Merkle inclusion, but changes its **wtxid** and
fails the original BIP141 witness commitment. Independently regenerating a
matching fake coinbase commitment lets the altered-witness block pass this
limited SHA256d structural audit. Its signature remains invalid, and the
report still refuses spend authorization.

## Remaining independent proof requirements

A block passing these checks can still have an invented header anchor, invalid
difficulty or active chain, invalid UTXO inputs, rejected Script or ECDSA
signatures, outdated channel state, revoked signatures, incorrect Type-4/19
historical admission, forged oracle data, unauthorized maker/AMM fills or
economically invalid liquidation. Bitcoin Core-consistent **full-block
consensus**, cumulative-work/best-chain authentication, spending script,
historical UTXO set and channel revision/revocation must be checked
independently. Litecoin Scrypt and Dogecoin AuxPoW require their own validators.

This module does not validate all BIP34, BIP141 activation, block weight,
witness Script, time/MTP and UTXO state consensus requirements. The in-tree
SHA256d implementation has independent known-answer tests but is not itself
a production-audited cryptographic primitive.

## Evidence and build discipline

Exact source was run on an ephemeral GitHub checkout with the concurrent
Stage25M historical-admission main tree retained. The full
`bash scripts/ci-preflight.sh --full`, frozen cargo/no-third-party-crate
audit, independent Python vectors, dedicated BIP141 and existing channel
regressions and release build passed on
[staged prepublication run 40162951864](https://github.com/patrickdugan/TradeLayer-Rust/actions/runs/40162951864).
This source tree includes the unchanged historical Type-4/19 admission
module added by PR #101.

The next acceptance gate is fresh CI on the **permanent source PR**.
Keep formal/Lean/Kani refinement and finance authorization separate.
Related: [issue #100](https://github.com/patrickdugan/TradeLayer-Rust/issues/100),
[issue #47](https://github.com/patrickdugan/TradeLayer-Rust/issues/47),
[issue #80](https://github.com/patrickdugan/TradeLayer-Rust/issues/80).
