# Stage 10: Channel escrow (offline research model)


Stage 11 adds [a deferred-queue parity oracle and finalized-height replay fence](WITHDRAWAL_PARITY.md), documenting concrete differences with historical JavaScript and a type-21 wire amount bug.

This layer extends Stage 9's **unverified** replay projection with typed
accounting for legacy transaction types 4, 21, and 22. The intent comes from
the archived JavaScript implementation:

- [Type 4, commitToken in logic.js](https://github.com/patrickdugan/tradelayer.js/blob/master/src/logic.js): debits available and credits channel accounting.
- [Type 21, withdrawal queue in channels.js](https://github.com/patrickdugan/tradelayer.js/blob/master/src/channels.js): requests mature at a later block; JavaScript processes the queue after block operations with a seven-block delay.
- [Type 22, channel transfer in logic.js](https://github.com/patrickdugan/tradelayer.js/blob/master/src/logic.js): sends a specified channel-column balance to another channel.

This is a **restricted model**, not exact JavaScript consensus parity.
A chain adapter is still required to authenticate signatures, transaction
origins, output references, two-party multisig eligibility, and encoding.
The current typed operations omit legacy payment references, clearlist
flags, whitelist policy and variable column tie-breaking.

## Supply and ownership rules

For every property p:

    total_supply[p] =
        sum(available[p, account])
      + sum(escrow[p, channel, side])
      + sum(reserved_pending_withdrawal[p, request_txid])

Every movement uses checked u128 arithmetic. A channel has at most two
distinct participants (columns A and B), bound to explicit account IDs.
A commit establishes a column owner or adds to a previously owned column.
A sender cannot take over a column held by another participant, nor occupy
both sides of the same channel.

Type 4 moves funds from the claimed participant's available balance into
one channel column. Type 22 requires the *claimed actor* to equal the source
channel address. It moves only free channel escrow, keeps the source column's
participant as beneficial owner, and requires the destination column to be
vacant or owned by that same participant. Transfers with identical source
and destination channels are rejected. The use of a typed, explicit column
is provisional pending historically authenticated channel-address assignment.

Type 21 must be claimed by the channel column's participant. The request
immediately moves the specified amount from *free escrow* into a reserved
pending-withdrawal record associated with the request txid. It is credited
to that participant's available balance only after seven subsequent blocks
(the request at height H matures at H+7). The amount is part of issued
supply throughout the wait.

**Known semantic difference:** historical JavaScript enqueues a withdrawal
without initially reserving its tokens, and may eject requests with
insufficient escrow at maturity. Stage 10 reserves immediately, to make
the projection non-double-spendable and deterministic. Historical replay
parity is blocked until the queue/race semantics are explicitly resolved.

## Persistence and rollbacks

The ledger snapshot retains TLL version 1 for states with no channel book
or pending withdrawals. TLL version 2 appends canonical sorted channels,
column owners, nonzero escrow entries and reserved withdrawal requests.
The outer TLPS and TLUC checkpoint framing is unchanged. Existing Stage 9
snapshots therefore remain byte-compatible, and snapshots containing
channel state can be restored by Stage 10.

All snapshot decoding enforces counts, canonical ascending keys, valid
column owners and supply conservation; CRC32 is for incidental corruption
only. It is not a cryptographic proof of state authenticity.

UnifiedChainState finalizes all due withdrawals at the end of each
externally authenticated block, including blocks with no TL transactions.
The entire pre-block protocol state, pending reserves and maturation
effects are rolled back together on a reorg. Direct users of the offline
ProtocolState API must call finalize_block_unverified after each completed
block. Those inputs remain untrusted.

## Explicitly unimplemented

- Actual BTC/LTC/DOGE signature and UTXO provenance, valid channel multisig
  membership, transaction inclusion and chain selection
- Exact historical address ordering, column bump rules, transfer payment
  references, output-reference resolution, fees and restricted-property
  compliance rules
- Withdraw-all, multi-property requests and competing same-block queue
  semantics; settlement of channel trades and derivatives (types 19/20/23)
- Channel closing / occupied-slot release, cross-property netting,
  authenticated roots and cryptographic Cairo/STWO transition verification
- Full archive-backed JS/Rust differential replay

CI's synthetic tests check accounting and checkpoint integrity. They
establish no live-chain authorization or consensus correctness.
