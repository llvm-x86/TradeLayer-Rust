#!/usr/bin/env python3
"""Independent, synthetic TLPT v1 vector. Audits exact Rust wire bytes.

This uses only Python's hashlib and standard-library byte encoding. It never
parses real chain data, verifies Cairo/STWO proofs, or authorizes balances.
"""
import hashlib
import pathlib
import sys

OUT = pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "tlpt_v1_vectors.tsv"


def digest(payload: bytes) -> bytes:
    return hashlib.sha256(payload).digest()


def head_root(chain: bytes, height: int, block: bytes, protocol: bytes) -> bytes:
    return digest(b"TLR_CHAIN_HEAD_V1\x00" + chain + height.to_bytes(8, "big") + block + protocol)


def build() -> str:
    chain = bytes([1]) * 32
    parent_height, child_height = 123, 124
    parent_id, child_id = bytes([2]) * 32, bytes([3]) * 32
    old_root, new_root = bytes([9]) * 32, bytes([8]) * 32
    old_head = head_root(chain, parent_height, parent_id, old_root)
    new_head = head_root(chain, child_height, child_id, new_root)
    body = (
        b"TLPT\x01" + chain
        + parent_height.to_bytes(8, "big") + parent_id
        + child_height.to_bytes(8, "big") + child_id
        + old_root + new_root + old_head + new_head
    )
    wire = body + digest(body)
    assert len(wire) == 277
    return digest(body).hex() + "\t" + wire.hex() + "\n"


if __name__ == "__main__":
    text = build()
    if len(sys.argv) == 2 and sys.argv[1] == "--check":
        if OUT.read_text(encoding="ascii") != text:
            raise SystemExit("TLPT canonical fixture differs from independent SHA-256 reference")
        print("TLPT v1 canonical fixture matches independent Python SHA-256 reference.")
    elif len(sys.argv) == 2 and sys.argv[1] == "--write":
        OUT.write_text(text, encoding="ascii")
    else:
        raise SystemExit("usage: generate_tlpt_vectors.py --check|--write")
