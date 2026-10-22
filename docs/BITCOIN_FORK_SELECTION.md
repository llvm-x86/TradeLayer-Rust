# Stage 18: bounded observed Bitcoin-mainnet header fork choice

Stage 17 checked one contiguous sequence of Bitcoin-mainnet SHA256d
headers with exact difficulty transitions, median-time-past (MTP), and
256-bit cumulative work. Stage 18 introduces a bounded **observed**
header graph that allows a branch to attach to any previously admitted
header, including one below the graph's current best observed tip.

A header graph is **not** an authenticated live Bitcoin chainstate.
It contains only the branches supplied to this process, which might
omit the actual best chain or even originate from a forged checkpoint.
This API remains for audit, simulation and testing. There is no
TrustedChainAdapter or TrustedProtocolAdapter implementation.

## Construction and trust roots

`ObservedMainnetForks::new(root_chain, limit)` takes a
`MainnetWorkChain` that Stage 17 has already constructed. Its root
may derive from published Bitcoin mainnet genesis, an already checked
continuous header history, or a caller-supplied checkpoint. The new
graph inherits `AnchorStatus::PublishedGenesis` or
`AnchorStatus::ClaimedExternal` without ever changing it.

The limit counts **new descendant headers**, excluding the root.
It must be 1 through 4,096. Each `observe_segment(parent_id, headers)`
call accepts 1 through 500 headers and requires an observed parent.
Every header must have genuine SHA256d PoW, a matching parent, expected
mainnet nBits, valid MTP and representable accumulated work.

The graph verifies every header and saves all intermediate states in
temporary memory before committing the entire segment. Duplicate
headers, disconnected parents, too-long segments and quota exhaustion
return explicit errors. Failed calls leave the graph unchanged. The
graph refuses overflow rather than evicting history needed to audit
fork ancestry.

## Selection and reorganization reports

After a successful insertion, the graph compares the candidate tip's
exact 256-bit cumulative work with the incumbent **observed** tip.
Only strictly greater work changes the selected tip. On equal work,
it retains the incumbent to avoid nondeterministic reorg churn.

`ObservedBestChange` reports:

- The previous and newly selected observed tips.
- The common ancestor among locally stored, individually checked
  headers.
- A disconnected path ordered from former tip back toward the
  common ancestor, excluding that ancestor.
- A connected path ordered forward from the common ancestor to the
  new selected tip, excluding that ancestor.

A simple extension yields no disconnected blocks. Admission of a
shorter-work branch can be successful while producing no best-tip
change. Accessors report the graph root, provenance, header count,
best observed tip and chainwork, plus read-only chain state for each
known header. There is no network discovery, durable chain database,
transaction validation or checkpoint promotion.

## Independent evidence and tests

Fixtures extend the original genuine Bitcoin mainnet genesis and
block 1 through published blocks 2 and 3.

| Height | Block hash (display order) |
| --- | --- |
| 0 | `000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f` |
| 1 | `00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048` |
| 2 | `000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd` |
| 3 | `0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449` |

The standard-library Python independent oracle hashes all four
serialized headers and checks parent hashes, historical timestamps,
nonces, MTP, PoW thresholds and chainwork. Rust integration tests cover
actual staged admission, future-parent extension, capacity failures,
bad PoW with otherwise valid earlier headers, duplicate blocks,
unknown parents, and deliberately untrusted checkpoints.

The branch switch reporter and strict-work tie-breaker also have
small isolated unit tests using **synthetic graph paths/work values**.
These test data-structure invariants only; no genuine historical
competing Bitcoin mainnet fork has been supplied to this fixture.

```sh
cargo test --locked --test bitcoin_forks
python3 scripts/audit_bitcoin_forks.py
cargo test --locked
```

Published historical block header fields: Bitcoin mainnet block
explorers at [height 2](https://mempool.space/block/000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd)
and [height 3](https://mempool.space/block/0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449).
These fields are independently SHA256d-verified from raw header bytes.

## Trust boundary and future stages

An actual Bitcoin Core active best chain can be selected only from
verified and sufficiently complete network/full-node context, which
this bounded set of caller-provided headers cannot establish.
Additional full-block checks, script/UTXO economic verification,
deployment rules, trusted time bounds and independently authenticated
high-height checkpoints are still absent. Litecoin and Dogecoin use
Scrypt proof-of-work and have separate consensus parameters.
Cairo/STWO economic proofs remain unverified.

The Stage 13 settlement gate stays default-deny. Choosing a tip in
this graph cannot authorize a TradeLayer transaction or payout.
