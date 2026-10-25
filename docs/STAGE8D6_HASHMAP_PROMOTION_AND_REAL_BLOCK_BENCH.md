# Stage 8D6: promote the HashMap-only TXID reducer for one-team CPU preindex

Baseline: merged `TradeLayer-Rust main@c7a37eb90b1ea335051bfc3cb7712116b372cfb7`. This is a narrow successor to exploratory [PR #139](https://github.com/patrickdugan/TradeLayer-Rust/pull/139). Other draft CPU schedulers and UTXO research remain independent.

## Change and trust boundary

The opt-in, block-wide `UnverifiedRawBlockView::inspect_parallel_one_team` now preallocates a standard, randomly seeded `HashMap<[u8; 32], usize>` for duplicate-TXID detection. The workers still hash and classify immutable prepared transactions using the already merged dynamic atomic microbatch scheduler. The coordinator still joins all workers and processes results *in their original block order*, maintaining the original first/second duplicate indices, earliest scan errors, carrier counts and candidate ordering.

`inspect_parallel_one_team_tree_reference` retains the original `BTreeMap` reduction for benchmarks and parity testing. The **default** `inspect_parallel`, serial path, historic owned-transaction APIs, Merkle algorithm, optional dependency waves and trusted/consensus denials remain unchanged. For a one-worker call, both one-team routes intentionally use the original serial code. The in-place Merkle experiment from #139 is excluded: its measured isolated gain was negligible and is irrelevant to this patch.

The extra memory of the preallocated `HashMap` is bounded by `MAX_BLOCK_TRANSACTIONS` after checking the existing `MAX_BLOCK_INPUT_BYTES` limit. The one-team scanner already allocates one result slot per transaction. The standard `HashMap` uses a process-random seed and does not depend on iteration order for any consensus-relevant decision. This produces *unverified observations*, never Bitcoin chain/UTXO/script validation or TradeLayer economic state admission.

## Prior synthetic findings

A [green Stage8D5 counterbalanced run](https://github.com/patrickdugan/TradeLayer-Rust/actions/runs/40242371422) gave original one-team vs HashMap-only prepared-only median times:

| TX / SegWit / workers | BTreeMap | HashMap only |
| --- | ---: | ---: |
| 1,000 / 0% / 1 | 2.075 ms | 2.083 ms |
| 4,000 / 50% / 4 | 7.785 ms | 7.157 ms |
| 10,000 / 95% / 8 | 22.325 ms | 20.724 ms |

These GitHub-hosted synthetic results identify the candidate, not real Bitcoin Core throughput. Stage8D6 independently measures its narrow promotion against the retained reference with alternating variant order. There is no CI speed threshold.

## Real-block CPU and peak-memory harness

`tradelayer-real-block-reduce-bench` reads a *local* Bitcoin Core `getblock(blockhash,0)` hexadecimal file, checks raw syntax and locally recomputed TXID Merkle root using the existing `UnverifiedRawBlockView`. It times **prepared-only CPU scanning** for one of `tree` or `hash`; the block fetch, disk read and first structural parse are excluded from reported scan times. One variant per process permits Linux `/proc/self/status` `VmHWM` peak-RSS measurement, including parsing and allocation. It prints identity and sample count for audit, and explicitly refuses full consensus claims.

`scripts/bench_stage8d6_real_blocks.py` launches independent processes for both reducers, alternates their order across paired trials, validates header identity and transaction count, and reports their median scan times and median process-high-water memory as JSON Lines, one per source block. All inputs are operator-supplied local files; the scripts perform no RPC and require no secrets.

Example on a host that already runs Bitcoin Core:

```sh
# Choose heights using YOUR local node. Select varied transaction counts and
# witness/OP_RETURN mixes; heights below are examples, not claimed TX counts.
for height in 700000 800000 850000; do
  hash="$(bitcoin-cli getblockhash "$height")"
  bitcoin-cli getblock "$hash" 0 > "/tmp/bitcoin-$height.hex"
done

cargo build --all-targets --release --frozen
python3 scripts/bench_stage8d6_real_blocks.py --workers 8 \
  --repeats 5 --trials 5 \
  /tmp/bitcoin-700000.hex /tmp/bitcoin-800000.hex /tmp/bitcoin-850000.hex
```

Each `tree`/ `hash` invocation has the same parsed block and result-slot budget. The printed peak RSS is *whole-process* high-water memory, including startup, input bytes and layout parsing. Evaluate median and tail latency across several real blocks on a controlled CPU; GitHub runner measurements can shift with noise and host contention. Do not infer that reading a raw block from a file authenticates active chain membership or that hashing speed predicts script validation throughput.

A frozen historical **Bitcoin mainnet block 1** header and coinbase already in `fixtures/` are assembled by `scripts/make_stage8d6_mainnet_block1.py` for a fully offline, public-source CI smoke test. The script independently verifies the coinbase TXID equals the published header Merkle. That genuine one-transaction block checks the input format and safety boundary; it is too small to evaluate parallel speedup. Representative recent blocks remain an explicit operator-side measurement requirement.

## Gates before merging

The new Rust regression module compares the HashMap-only path with the original one-team reducer and windowed/serial reference across worker counts 1/2/4/8/16, windows 1/7/64/256/512, 0/50/95%-SegWit blocks, 10k TX stress and duplicate first/second indices. It also checks the source historical block-1 fixture, invalid configurations and all permanent-denial gates.

Require frozen formatting, Clippy, tests, independent release build, strict aggregate check and historical block-1 smoke on the exact PR head. **Keep draft until a repeated, same-host real-block corpus documents CPU latency distribution and peak RSS**, and a reviewer accepts the memory tradeoff. The default windowed and serial paths stay as published.
