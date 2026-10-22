# Stage 17: Bitcoin-mainnet SHA256d difficulty and cumulative header work

Stage 16 could demonstrate transaction Merkle membership *relative to*
PoW-checked 80-byte headers, but did not check mainnet difficulty
transitions, cumulative work, or median-time-past (MTP). An attacker
could present a self-consistent easier-difficulty history.

This stage adds bounded **Bitcoin mainnet header-only** validation
for exact difficulty changes, MTP, and accumulated work from a
configured starting checkpoint. It is not a full Bitcoin node,
a trusted best-chain locator, a settlement economic verifier,
or an authenticated Litecoin/Dogecoin adapter.

## Implemented rules

Sources reviewed include Bitcoin Core src/pow.cpp
(GetNextWorkRequired, CalculateNextWorkRequired, DeriveTarget)
and chain.cpp GetBlockProof. The implementation uses checked private
256-bit unsigned integers, high-limb numeric comparison, long
division, and overflow-checked arithmetic.

- Header SHA256d, PoW and parent links reuse Stage 16 checks.
- For **Bitcoin mainnet only**, the next nBits must be unchanged
  except at heights divisible by 2,016. At adjustment heights,
  the previous target is multiplied by the bounded timestamp
  interval, divided by 1,209,600, capped at mainnet powLimit and
  compact-encoded as Bitcoin Core does.
- The interval goes from the first block of the outgoing
  2,016-block period to its last block, covering 2,015 intervals.
- A header timestamp must exceed the median of the 11 preceding
  timestamps, or all prior headers available near genesis.
- The exact chainwork increment is floor(2^256/(target+1)),
  matching Bitcoin Core GetBlockProof.
- Each extension is all-or-nothing across linked headers. Wrong
  difficulty, parent, MTP, PoW or work overflow leaves the previous
  accumulated checkpoint intact.
- Work can be compared between extensions only if their original
  checkpoint and declared provenance exactly match. That
  comparison is a *local order*, not live best-chain verification.

## Checkpoint provenance is explicit

The MainnetWorkChain type has two constructors:

1. from_published_genesis pins the actual Bitcoin-mainnet genesis
   header against its published SHA256d block hash and derives
   its initial work, timestamp and nBits.
2. from_claimed_checkpoint admits separately supplied tip,
   cumulative work, nBits, first timestamp of the ongoing
   difficulty epoch, and prior 11 timestamps. Such state
   remains an **UNTRUSTED declaration** unless independently
   authenticated. Shape validation cannot establish authenticity.

The source is visible through AnchorStatus. An arbitrary
caller-provided checkpoint must not be promoted to a trusted
chain root merely because its descendants pass PoW. Validated
states can be extended through multiple bounded segments.

## Historical and adversarial tests

The suite checks genuinely published Bitcoin mainnet genesis
and block 1 headers and reproduces Bitcoin Core work:

    genesis work: 0x100010001
    through block 1: 0x200020002

Published block 1 hash:

    00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048

Other tests cover retarget upper/lower clamps, quarter/half/two/four
timespan cases, malformed checkpoints, wrong parents, atomic
rejection, and cross-anchor work comparisons.

An independent Python bigint implementation checks real header
hashes, chainwork, compact target and retarget arithmetic.

    cargo test --locked --test bitcoin_chainwork
    python3 scripts/audit_bitcoin_chainwork.py
    cargo run --locked --bin tradelayer-bitcoin-work -- \\
      fixtures/bitcoin_mainnet_genesis_block1_headers.txt
    cargo test --locked

The read-only CLI accepts a bounded file with genuine Bitcoin mainnet
genesis as its first lowercase 160-hex-character header, then up to
500 consecutive raw mainnet headers. It refuses any non-genesis anchor
and prints the resulting cumulative work plus explicit UNVERIFIED
statuses for active-chain membership and economic validity.

## Remaining limitations before trust promotion

There is no validation of full transaction blocks, coinbase,
UTXO balances, witness commitments, script/signature authorization,
maximum-future-time against a trusted clock, full deployment/version
activation, peer selection, or active best-chain selection.
Claimed high-height checkpoints do not establish correct historical
MTP or difficulty provenance. An unaudited SHA-256 implementation
remains a security dependency in this experimental module.

Bitcoin testnets have special min-difficulty and BIP94 rules;
regtest has its own parameters. Litecoin and Dogecoin PoW
uses Scrypt, not Bitcoin SHA256d. Those networks are unsupported
by this strict Bitcoin-mainnet validator.

Merkle inclusion is not TradeLayer consensus acceptance.
The Stage 13 economic gate still rejects settlement operations
without independent trade/clearing verification. No STWO proof
verification, new Rust dependencies, or economic state mutations
are introduced by this stage.
