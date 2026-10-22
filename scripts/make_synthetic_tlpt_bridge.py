#!/usr/bin/env python3
"""Generate explicitly SYNTHETIC TLPT and observation inputs for an audit smoke test.

Uses the source-derived 26-receipt zkTL metadata fixture but invents ALL
block hashes, state hashes, and transaction observations. A passing CLI audit
from this output has ZERO chain/proof/economic authority.
"""
from __future__ import annotations

import hashlib
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "fixtures" / "zkTL_sample_batch.tsv"
DOMAIN = b"TLR_CHAIN_HEAD_V1\x00"
CHAIN = bytes([11]) * 32
ANCHOR_ID = bytes([19]) * 32


def sha256(payload: bytes) -> bytes:
    return hashlib.sha256(payload).digest()


def head_root(height: int, block: bytes, protocol: bytes) -> bytes:
    return sha256(DOMAIN + CHAIN + height.to_bytes(8, "big") + block + protocol)


def make_inputs() -> tuple[str, str]:
    lines = SOURCE.read_text(encoding="ascii").splitlines()
    header = lines[0].split("\t")
    assert len(header) == 7 and header[0] == "TLZK1"
    start, end = int(header[2]), int(header[3])
    assert (start, end) == (132900, 132907)
    blocks = end - start + 1
    prev_id, prev_height = ANCHOR_ID, start - 1
    old_root = bytes([80]) * 32
    out_tlpt = []
    block_ids = {}
    for i in range(blocks):
        height = start + i
        block_id = bytes([20 + i]) * 32
        after_root = bytes([81 + i]) * 32
        old_head = head_root(prev_height, prev_id, old_root)
        new_head = head_root(height, block_id, after_root)
        body = (
            b"TLPT\x01" + CHAIN
            + prev_height.to_bytes(8, "big") + prev_id
            + height.to_bytes(8, "big") + block_id
            + old_root + after_root + old_head + new_head
        )
        wire = body + sha256(body)
        assert len(wire) == 277
        out_tlpt.append(sha256(body).hex() + "\t" + wire.hex())
        block_ids[height] = block_id
        prev_id, prev_height, old_root = block_id, height, after_root

    out_observations = []
    per_height = {}
    for line in lines[1:]:
        fields = line.split("\t")
        assert len(fields) == 12 and fields[0] == "R"
        txid, ty, height_text = fields[2], fields[3], fields[4]
        height = int(height_text)
        assert height in block_ids
        index = per_height.get(height, 0)
        per_height[height] = index + 1
        out_observations.append(
            "\t".join([
                "O", str(height), block_ids[height].hex(), txid, ty, str(index),
            ])
        )
    assert len(out_observations) == 26
    return "\n".join(out_tlpt) + "\n", "\n".join(out_observations) + "\n"


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: make_synthetic_tlpt_bridge.py <output-dir>")
    target = Path(sys.argv[1])
    if not target.is_dir():
        raise SystemExit("output directory missing")
    tlpt, observed = make_inputs()
    (target / "synthetic_tlpt.tsv").write_text(tlpt, encoding="ascii")
    (target / "synthetic_observations.tsv").write_text(observed, encoding="ascii")
    print("SYNTHETIC audit fixtures only: NO confirmed block or authorized receipt.")


if __name__ == "__main__":
    main()
