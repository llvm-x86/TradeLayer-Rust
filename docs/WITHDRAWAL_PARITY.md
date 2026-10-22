# Stage 11: Withdrawal queue divergence and finalized-block fence

Stage 10 introduced a **reserving** channel-escrow ledger. Stage 11 records
and tests differences with the historical JavaScript **deferred** withdrawal
queue. Its oracle is **read-only**: no legacy queue decisions authorize state
changes in the unified Rust ledger.

Historical source reviewed at
[tradelayer.js dca0888](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c):

- src/validity.js, validateWithdrawal
- src/channels.js, addToWithdrawalQueue, processWithdrawals, processWithdrawal
- src/logic.js, withdrawal and transfer
- src/main.js, post-block queue processing
- src/txEncoder.js and src/txDecoder.js, type 21

## Divergence: when is escrow spent?

For one property and one channel column, the JS accepts a request after
checking the current column balance and queues it without debiting that
balance. Seven blocks later, it checks the **then-current** balance.
Requests are considered in insertion order, and a request whose balance
is insufficient gets dropped. Channel operations from the maturity block
execute before the queue check.

Stage 10 reserves at request time, so pending requests are unable to
compete with subsequent channel transfers or other withdrawal requests.
For example, an escrow of 100 atoms and two 60-atom requests at height 103:

| At height | Stage 10 reserving ledger | JS-like deferred shadow |
| --- | --- | --- |
| 103 | First 60 accepted; second rejected for insufficient free escrow | Both 60-atom requests queued against the same 100 atoms |
| 110 | Reserved 60 released | First 60 paid; second 60 dropped after only 40 remains |

A second counterexample reserves 80 atoms in Stage 10 at H=103, while
the JS-like shadow permits 80 to transfer out at H=104. The request then
drops at H=110. Returning the 80 before the maturity-block queue pass
allows it to pay.

These cases are represented by the pure
src/withdrawal_parity.rs LegacyWithdrawalShadow and
tests/withdrawal_parity.rs. The oracle tracks available escrow,
another-channel funds, outstanding requests, final outcomes and
conservation. It assumes **one property and one participant/column**.
It also assumes the global reserve covers the channel balance.
Historical JS additionally calls TallyMap.hasSufficientReserve, which can
change the payout. It handles withdraw-all separately, can remap channel
columns and is subject to database behavior. Those cases still need
recorded historical replay and actual-JS comparison.

**No consensus-mode switch is introduced.** Production parity would
require a chain-height-gated, explicit migration plan based on accepted
history. The safe reserving semantics remain unchanged in TradeLayer-Rust.

## Legacy type-21 encoder / decoder amount mismatch

The historical encodeWithdrawal multiplies amountOffered by 1e8, then
writes a decimal string. decodeWithdrawal parses that field in **base 36**
and divides by 1e8. The discrepancy is reproducible for exact integer
atoms without executing untrusted legacy npm dependencies:

    100000000 atoms (1.00000000 token)
       encoded "100000000" (decimal)
       decoded 2821109907456 atoms (28211.09907456 tokens)

The diagnostic is independent of actual transaction validation and only
covers atom counts up to JavaScript's Number.MAX_SAFE_INTEGER. A separate
problem is that the encoder passes the BigNumber through a JS Number,
which can lose precision at greater amounts.

Run:

    node scripts/audit_legacy_withdrawal_codec.cjs
    cargo test --locked --test withdrawal_parity

Both implementations check the same read-only
fixtures/withdrawal_codec_vectors.tsv. This is a **synthetic decoder
diagnostic**, not proof that any particular historic transaction was
accepted or its withdrawal completed. Establish actual on-chain payloads
and the legacy node's observed verdicts before choosing a wire migration.

## Stage 11 finalized-block fence

Stage 10's direct unverified ProtocolState finalization processed
withdrawals but did not record the highest finalized block. A later
caller could apply a transaction at that earlier height and thereby
change a past maturity result.

ProtocolState now records finalized_height. It rejects operations at or
below a finalized height and requires subsequent finalizations to be
contiguous. It takes a clone before finalizing to retain all-or-nothing
accounting. The first finalization can be at a later block when resuming
an existing TLPS v1 snapshot whose chain tip advanced on non-TL blocks;
its caller must provide the verified current tip.

- TLPS v1 reads and writes unchanged for a state without finalized block
  history. Pre-Stage-11 checkpoints remain readable.
- TLPS v2 appends a canonical finalized height after the existing
  transaction-height field. It rejects zero, invalid, and inconsistent
  heights. CRC32 detects accidental corruption only.
- Existing TLUC checkpoint framing remains unchanged. Restored TLUC
  snapshots with TLPS v2 must have a finalized height equal to the
  corresponding main/history chain tip.
- UnifiedChainState processes completed blocks in a sequential
  adapter-checked order, so the new fence catches accidental misuse
  across the combined registry/ledger replay path.

## Still required before network use

A real authenticated chain adapter must validate block inclusion,
signatures, scripts, UTXO provenance and channel control. Reconciliation
must cover historical encoded type-21 bytes, exact validity semantics,
withdraw-all, partial channel reserve payouts, competing maturity
requests, payment references and type 23 clearing. Cairo/STWO economic
transition verification and zkTL state-root agreement remain separate
blocked requirements. The cloned in-memory state remains experimental.
