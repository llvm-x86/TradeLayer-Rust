#!/usr/bin/env python3
"""Offline same-host CPU / peak-RSS comparison on local Bitcoin Core raw hex blocks.

No RPC, wallets or remote hosts are contacted. Each variant starts in a fresh
process so VmHWM measures peak RSS independently, including shared parse costs.
"""
import argparse
import json
import statistics
import subprocess
from pathlib import Path


def bench(binary, block, workers, repeats, variant):
    cmd = [str(binary), str(block), str(workers), str(repeats), variant]
    result = subprocess.run(cmd, check=True, capture_output=True, text=True)
    output = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    required = {
        "variant",
        "block_hash",
        "transaction_count",
        "raw_block_bytes",
        "prepared_only_median_ms",
        "process_peak_rss_kib",
        "txid_merkle",
        "full_bitcoin_consensus",
    }
    missing = required - output.keys()
    if missing:
        raise ValueError(f"missing metrics from {variant}: {sorted(missing)}")
    if output["variant"] != variant or output["txid_merkle"] != "LOCAL_CHECK_ONLY":
        raise ValueError(f"invalid output from {variant}")
    if output["full_bitcoin_consensus"] != "UNVERIFIED":
        raise ValueError("unexpected authority field")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("blocks", nargs="+", type=Path, help="local getblock(hash,0) hex files")
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--trials", type=int, default=4)
    parser.add_argument(
        "--binary",
        type=Path,
        default=Path("target/release/tradelayer-real-block-reduce-bench"),
    )
    args = parser.parse_args()
    if not (1 <= args.workers <= 16 and 1 <= args.repeats <= 20 and 1 <= args.trials <= 20):
        parser.error("workers 1..16, repeats 1..20 and trials 1..20 required")
    if not args.binary.is_file():
        parser.error(f"build the release binary first: {args.binary}")

    for block in args.blocks:
        if not block.is_file():
            parser.error(f"missing local block: {block}")
        measurements = {"tree": [], "hash": []}
        identities = set()
        for trial in range(args.trials):
            order = ("tree", "hash") if trial % 2 == 0 else ("hash", "tree")
            for variant in order:
                observed = bench(args.binary, block, args.workers, args.repeats, variant)
                identities.add(
                    (
                        observed["block_hash"],
                        observed["transaction_count"],
                        observed["raw_block_bytes"],
                    )
                )
                measurements[variant].append(observed)
        if len(identities) != 1:
            raise ValueError("source file changed or parity metadata differs during trials")
        block_hash, count, byte_size = next(iter(identities))
        def median_metric(variant, key):
            values = [row[key] for row in measurements[variant]]
            if key == "process_peak_rss_kib" and "UNAVAILABLE" in values:
                return None
            return statistics.median(float(value) for value in values)

        tree_ms = median_metric("tree", "prepared_only_median_ms")
        hash_ms = median_metric("hash", "prepared_only_median_ms")
        record = {
            "source_file": str(block),
            "block_hash": block_hash,
            "transaction_count": int(count),
            "raw_block_bytes": int(byte_size),
            "workers": args.workers,
            "repetitions_per_process": args.repeats,
            "paired_process_trials": args.trials,
            "tree_median_ms": round(tree_ms, 3),
            "hash_median_ms": round(hash_ms, 3),
            "tree_over_hash_ratio": round(tree_ms / hash_ms, 4),
            "tree_peak_rss_kib": median_metric("tree", "process_peak_rss_kib"),
            "hash_peak_rss_kib": median_metric("hash", "process_peak_rss_kib"),
            "source_trust": "LOCAL_MERKLE_ONLY",
        }
        print(json.dumps(record, sort_keys=True))


if __name__ == "__main__":
    main()
