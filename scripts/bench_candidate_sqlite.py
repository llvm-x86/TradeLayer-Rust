#!/usr/bin/env python3
"""Repeatable offline throughput probe: raw-block frames -> Rust -> SQLite.

Synthetic blocks derived from the repository's frozen 4-TX fixture.
Not a Bitcoin Core RPC benchmark, not valid PoW, and not consensus evidence.
No performance threshold is asserted.
"""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from time import perf_counter

from candidate_index_sqlite import ingest, init, stats


ROOT = Path(__file__).resolve().parent.parent


def sha256d(value):
    return hashlib.sha256(hashlib.sha256(value).digest()).digest()


def make_frames(count):
    source = bytes.fromhex(
        (ROOT / "fixtures/raw_block_ingest_synthetic.hex").read_text().strip()
    )
    raw_parent = source[4:36]
    initial_anchor = raw_parent[::-1].hex()
    out = []
    for n in range(count):
        header = bytearray(source[:80])
        header[4:36] = raw_parent
        header[68:72] = (1_700_000_000 + n).to_bytes(4, "little")
        header[76:80] = (n + 1).to_bytes(4, "little")
        raw = bytes(header) + source[80:]
        digest = sha256d(header)
        out.append(f"B\t{1000+n}\t{digest[::-1].hex()}\t{raw.hex()}\n")
        raw_parent = digest
    out.append(f"E\tBLOCKS\t{count}\n")
    return initial_anchor, "".join(out)


def run(count, binary, directory):
    if not binary.is_file():
        raise RuntimeError(f"missing precompiled Rust scanner: {binary}")
    anchor, blocks = make_frames(count)
    p = Path(directory)
    bf = p / "blocks.tsv"
    cf = p / "candidates.tsv"
    db = p / "index.sqlite"
    bf.write_text(blocks)
    start = perf_counter()
    scanner = subprocess.run(
        [str(binary)], input=blocks, text=True, capture_output=True, check=True,
        timeout=120,
    )
    scan_secs = perf_counter() - start
    if "END\tCOMPLETE_UNVERIFIED\tBLOCKS" not in scanner.stdout:
        raise RuntimeError("missing successful Rust scanner completion marker")
    cf.write_text(scanner.stdout)
    init(db, "regtest", 999, anchor)
    start = perf_counter()
    amount, txs, carriers, tip = ingest(db, "regtest", bf, cf)
    store_secs = perf_counter() - start
    result = stats(db)
    if (amount, txs, carriers) != (count, count * 4, count * 2):
        raise RuntimeError("candidate counts differ from fixture")
    if result["hash"] != tip.hash or result["trust"] != "UNVERIFIED":
        raise RuntimeError("candidate store tip or trust marker inconsistent")
    disk = sum(f.stat().st_size for f in p.glob("index.sqlite*"))
    print(f"benchmark=OFFLINE_UNVERIFIED_RAW_BLOCKS count={count}")
    print(f"rust_scan_s={scan_secs:.6f}")
    print(f"sqlite_commit_s={store_secs:.6f}")
    print(f"scanner_blocks_per_s={count / max(scan_secs, 1e-9):.2f}")
    print(f"sqlite_blocks_per_s={count / max(store_secs, 1e-9):.2f}")
    print(f"sqlite_candidates_per_s={carriers / max(store_secs, 1e-9):.2f}")
    print(f"database_bytes={disk}")
    print("block_and_merkle_binding=CHECKED")
    print("bitcoin_consensus=UNVERIFIED")


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("--count", type=int, default=32)
    cli.add_argument("--binary", type=Path, default=ROOT / "target/debug/tradelayer-core-index-ingest")
    args = cli.parse_args()
    if not 1 <= args.count <= 256:
        cli.error("count must be between 1 and 256")
    with tempfile.TemporaryDirectory() as tmp:
        run(args.count, args.binary, tmp)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError,
            subprocess.TimeoutExpired) as exc:
        print(f"OFFLINE BENCHMARK FAILED: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
