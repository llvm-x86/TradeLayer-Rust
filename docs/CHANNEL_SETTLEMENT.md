# Stage 12: NET_SETTLE journal and KING_SETTLE escrow clearing (offline)

Stage 13 adds a [separate, default-deny economic authorization seam](SETTLEMENT_AUTHORIZATION.md)
for type 23/31 when replaying through the chain adapter. The direct
unverified simulation API stays available for offline tests.

This is a **synthetic research projection**, not live Bitcoin/Litecoin/
Dogecoin consensus, a deployable settlement bridge, or economic proof
verification. All caller-supplied identities, trade references, signatures,
roots and chain histories still require independent authentication.

## Legacy semantic basis

At the reviewed source revision
[tradelayer.js/dca0888](https://github.com/patrickdugan/tradelayer.js/tree/dca0888b827c5bc28fa9189bc9e1d507f5c5670c):

- Type 23 has multiple settleType branches in validity.js: keep alive (0),
  close position (1), NET_SETTLE (2), and KING_SETTLE (3).
- scaling.js processNetSettle records **pending** net settlements and
  neutralizes references without moving channel balances.
- scaling.js processKingSettle does move a net payment between a channel's
  A/B token columns and marks a block range as swept.
- Type 31 is separately dispatched as King Settle in the transaction
  catalog. Legacy behavior is internally inconsistent across logic.js
  versus scaling.js and will require actual-source differential replay.

This stage ports an **explicit safe subset**: type23 settleType=2 as a
non-spending, one-property journal entry, and type31 as an atomic net
sweep of those journaled entries. Other type23 modes, zero-net sweeps,
type31 multi-property aggregation, and actual position pricing stay
blocked. No legacy JavaScript parsing is wired into the execution path.

## Accounting model

The type23 ChannelNetSettle operation is submitted by the claimed
channel actor and names a property, A/B payer side, positive u128
amount in atomic units, unique trade reference and expiration block.

- The channel must have distinct A/B participants and the claimed payer
  must have enough **free** escrow at claim time.
- Recording the claim uses a replay txid, but **moves no tokens**.
- Other claims can refer to the same collateral, as in the source's
  pending settlements. Trade references are unique per channel even
  after sweeping.
- A submitted claim expiring before a future king sweep cannot be
  cleared by that sweep. Expired journal pruning/cancellation is future
  work; claims are retained to expose protocol lifecycle decisions.

The type31 ChannelKingSettle operation supplies an inclusive block
range, claimed expected root, count, side, and net atomic amount.
The ledger selects **all pending claims** for that channel, property,
and block range in ascending txid order. It checks:

1. The range ends at or before the sweep's enclosing block height.
2. Each claim has an unexpired trade reference and valid bounds.
3. Positive A-to-B and B-to-A pending amounts are summed with checked
   u128 arithmetic. The difference determines the actual net payer.
4. Claimed root, count, direction and amount match the independently
   recomputed preview exactly.
5. The payer has enough **currently unreserved** channel escrow to
   satisfy the net; receiver-side addition cannot overflow.
6. Claim txids move from the pending journal into an immutable swept
   journal referenced by a king-settlement receipt.

An overdrawn sweep fails atomically, including its replay marker and
every claim status. Already swept claims cannot be swept again. Both
records and receipts are bounded at 10,000 entries. Historical
compaction and pruning remain future work.

The existing conservation equation remains:

    supply = available + channel columns + reserved pending withdrawals

A net sweep subtracts from one channel column and adds the same number
of atomic units to the other. Pending claims and sweep receipts are
**metadata** and contribute zero to issued supply.

## Roots and persistence

The digest is SHA-256 of a domain-separated binary transcript that
contains the channel, property, inclusive block range and each selected
claim's txid, side, amount, original trade reference, height and
expiration. A matching digest establishes only internal consistency.
It does not verify that the referenced trade occurred, that a signer
controlled the channel, or that Cairo/STWO proved valid economics.

The ledger snapshot format is versioned:

- TLL v1: unchanged historical available-balance and replay format.
- TLL v2: unchanged channel escrow/withdrawal format from Stage 10.
- **TLL v3:** channel state plus sorted pending claims, settled claims
  and king-sweep receipts. Decoder checks references, replay membership,
  ledger conservation, receipt roots, lengths, ordering, and canonical
  reserialization.

The enclosing TLPS (v1/v2) and TLUC frames remain unchanged, allowing
the same unified checkpoints to store the full stage 12 state. Reorgs
restore the ledger and journal together.

## Important remaining blockers

- A real authenticated chain adapter that derives the actor, txid,
  inclusion, channel control and parent/header validity from consensus.
- Actual provenance of NET_SETTLE claims, neutralization/keepalive
  semantics, pricing and expiry under the different type23 modes.
- Existing JS implementation's payment references, margin, liquidation,
  fee and whitelist/attestation policies. Full type31 channel-root
  commitments and alternative type23 settleType=3.
- Economic proof verification in Cairo/STWO and canonical state-root
  agreement with zkTL and the Bitcoin-family replay corpus.
- Zero-net cancellation, expiry pruning, durable transaction logs,
  stress/fuzz testing, historical settlement audit and protocol activation.

Run the synthetic regression suite with:

    cargo test --locked --test channel_settlement
    cargo test --locked --test unified_chain

No third-party Rust crates, wallet RPC or network access were added.
