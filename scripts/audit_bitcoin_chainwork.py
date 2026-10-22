#!/usr/bin/env python3
"""Independent Python bigint audit for Stage 17 Bitcoin-mainnet header rules.

Historical header 0 and 1 are published mainnet data. The retarget points
are arithmetic vectors, NOT mined boundary block samples or validation
of claimed chainwork checkpoints.
"""
from __future__ import annotations
import hashlib

GENESIS = (
    "01000000" + "00" * 32
    + "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a"
    + "29ab5f49ffff001d1dac2b7c"
)
BLOCK_ONE = (
    "01000000"
    + "6fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000"
    + "982051fd1e4ba744bbbe680e1fee14677ba1a3c3540bf7b1cdb606e857233e0e"
    + "61bc6649ffff001d01e36299"
)
TARGET_SPAN = 1209600


def sha256d(data: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def from_compact(bits: int) -> int:
    size = bits >> 24
    mantissa = bits & 0x007fffff
    assert size and mantissa and not bits & 0x00800000
    return (mantissa >> (8 * (3 - size))) if size <= 3 else (
        mantissa << (8 * (size - 3))
    )


def to_compact(target: int) -> int:
    assert target > 0
    size = (target.bit_length() + 7) // 8
    mantissa = (
        target << (8 * (3 - size)) if size <= 3
        else target >> (8 * (size - 3))
    )
    if mantissa & 0x00800000:
        mantissa >>= 8
        size += 1
    return size << 24 | mantissa


def work(bits: int) -> int:
    target = from_compact(bits)
    assert target > 0
    return (1 << 256) // (target + 1)


def retarget(bits: int, first: int, last: int) -> int:
    seconds = min(max(last - first, TARGET_SPAN // 4), TARGET_SPAN * 4)
    new = from_compact(bits) * seconds // TARGET_SPAN
    return to_compact(min(new, from_compact(0x1d00ffff)))


def main() -> None:
    genesis = bytes.fromhex(GENESIS)
    block = bytes.fromhex(BLOCK_ONE)
    assert len(genesis) == len(block) == 80
    assert sha256d(genesis)[::-1].hex() == (
        "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
    )
    assert block[4:36] == sha256d(genesis)
    assert sha256d(block)[::-1].hex() == (
        "00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048"
    )
    assert int.from_bytes(block[68:72], "little") == 1231469665
    assert int.from_bytes(genesis[68:72], "little") == 1231006505
    assert int.from_bytes(block[72:76], "little") == 0x1d00ffff
    assert int.from_bytes(sha256d(block), "little") <= from_compact(0x1d00ffff)
    for bits, target_work in [
        (0x1d00ffff, 0x100010001),
        (0x1c00ffff, 0x10001000100),
        (0x1b0404cb, 0x3fb3ab764c00),
    ]:
        assert work(bits) == target_work
    assert work(0x1d00ffff) * 2 == 0x200020002
    for bits, elapsed, expected in [
        (0x1d00ffff, 0, 0x1c3fffc0),
        (0x1d00ffff, 302400, 0x1c3fffc0),
        (0x1d00ffff, 604800, 0x1c7fff80),
        (0x1d00ffff, 1209600, 0x1d00ffff),
        (0x1d00ffff, 9676800, 0x1d00ffff),
        (0x1c00ffff, 2419200, 0x1c01fffe),
        (0x1c00ffff, 4838400, 0x1c03fffc),
    ]:
        assert retarget(bits, 0, elapsed) == expected
    print("Bitcoin mainnet genesis+block1, exact bigint chainwork and retarget vectors: PASS")


if __name__ == "__main__":
    main()
