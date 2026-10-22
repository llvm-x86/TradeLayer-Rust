#!/usr/bin/env python3
"""Independent Python SHA256d/merkle known-answer audit for Stage 16.

Bitcoin mainnet genesis is a published historical block. All later PoW
headers are deliberately FABRICATED under regtest-like difficulty and do
not establish best-chain membership or economic/consensus authority.
"""
from __future__ import annotations
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GENESIS_HEADER = (
    "01000000" + "00" * 32
    + "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a"
    + "29ab5f49ffff001d1dac2b7c"
)
EXPECTED_GENESIS_HASH = (
    "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f"
)
EXPECTED_GOLDEN_SYNTHETIC_HASHES = (
    "7bb43dfaf7ee531a3d762a871620b1c98097d5ad5d0aa4c08bc0b57176fec5f6",
    "32b5e3e509902ab61ce6e0d96e1e53c5721be388eee22a836cd55ac4c0d5ac23",
)


def sha256d(data: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def all_fixture_transactions(path: str) -> list[bytes]:
    return [
        bytes.fromhex(line.split("\t")[-1])
        for line in (ROOT / "fixtures" / path).read_text().splitlines()
        if line and not line.startswith("#")
    ]


def hash_hex_display(digest: bytes) -> str:
    return digest[::-1].hex()


def merkle_root(leaves: list[bytes]) -> bytes:
    assert leaves
    level = leaves[:]
    while len(level) > 1:
        if len(level) % 2:
            level.append(level[-1])
        level = [sha256d(level[i] + level[i + 1])
                 for i in range(0, len(level), 2)]
    return level[0]


def verify_pow(header: bytes, bits: int) -> None:
    assert len(header) == 80
    assert int.from_bytes(header[72:76], "little") == bits
    exponent, coefficient = bits >> 24, bits & 0x007fffff
    assert coefficient and not (bits & 0x00800000)
    target = coefficient * (1 << (8 * (exponent - 3)))
    assert int.from_bytes(sha256d(header), "little") <= target


def mined_header(parent_digest_raw: bytes, root: bytes, time: int) -> bytes:
    prefix = (
        (2).to_bytes(4, "little") + parent_digest_raw + root
        + time.to_bytes(4, "little") + (0x207fffff).to_bytes(4, "little")
    )
    target = 0x7fffff * (1 << (8 * 29))
    for nonce in range(10_000):
        candidate = prefix + nonce.to_bytes(4, "little")
        if int.from_bytes(sha256d(candidate), "little") <= target:
            return candidate
    raise AssertionError("mining deterministic test vector failed")


def main() -> None:
    published = all_fixture_transactions("real_transactions.tsv")
    coinbase = published[0]
    genesis = bytes.fromhex(GENESIS_HEADER)
    assert sha256d(genesis)[::-1].hex() == EXPECTED_GENESIS_HASH
    verify_pow(genesis, 0x1d00ffff)
    assert sha256d(coinbase) == genesis[36:68]
    assert hash_hex_display(sha256d(coinbase)) == (
        "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b"
    )

    transactions = all_fixture_transactions("synthetic_transactions.tsv")
    assert len(transactions) == 3
    leaves = [sha256d(tx) for tx in transactions]
    assert hash_hex_display(merkle_root(leaves)) == (
        "2e43a1566c5b6b8f671dd4d8887b00652a4c5ee5e27e14c3b186b371fe90bedb"
    )
    first = mined_header(bytes([0x12]) * 32, leaves[0], 1_700_000_000)
    second = mined_header(sha256d(first), merkle_root(leaves), 1_700_000_001)
    assert second[4:36] == sha256d(first)
    for current, expected in zip((first, second), EXPECTED_GOLDEN_SYNTHETIC_HASHES):
        verify_pow(current, 0x207fffff)
        assert hash_hex_display(sha256d(current)) == expected
    assert second[36:68] == merkle_root(leaves)

    # Independently evaluate the two branches asserted by Rust tests.
    pair_01 = sha256d(leaves[0] + leaves[1])
    pair_22 = sha256d(leaves[2] + leaves[2])
    assert sha256d(pair_01 + pair_22) == second[36:68]
    assert sha256d(
        sha256d(leaves[0] + leaves[1]) + sha256d(leaves[2] + leaves[2])
    ) == second[36:68]
    print("Bitcoin genesis and 2 synthetic SHA256d PoW/Merkle vectors agree.")


if __name__ == "__main__":
    main()
