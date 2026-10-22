#!/usr/bin/env python3
"""Independent standard-library audit for Stage 18 published Bitcoin headers.

This validates genuine mainnet genesis through height 3 without importing the
Rust header implementation. It does not validate transactions or best-chain
membership and never conveys settlement permission.
"""
from __future__ import annotations

import hashlib
from pathlib import Path

FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "bitcoin_mainnet_genesis_block3_headers.txt"
EXPECTED_HASHES = (
    "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
    "00000000839a8e6886ab5951d76f411475428afc90947ee320161bbf18eb6048",
    "000000006a625f06636b8bb6ac7b960a8d03705d1ace08b1a19da3fdcc99ddbd",
    "0000000082b5015589a3fdf2d4baff403e6f0be035a5d9742c1cae6295464449",
)
EXPECTED_TIMES = (1231006505, 1231469665, 1231469744, 1231470173)
EXPECTED_NONCES = (2083236893, 2573394689, 1639830024, 1844305925)
BITS = 0x1D00FFFF


def main() -> None:
    lines = FIXTURE.read_text(encoding="ascii").splitlines()
    assert len(lines) == len(EXPECTED_HASHES), "expected exactly genesis plus three descendants"
    previous_hash_raw = bytes(32)
    past_times: list[int] = []
    chainwork = 0
    for height, (encoded, expected_hash, expected_time, expected_nonce) in enumerate(
        zip(lines, EXPECTED_HASHES, EXPECTED_TIMES, EXPECTED_NONCES, strict=True)
    ):
        assert len(encoded) == 160 and encoded == encoded.lower()
        raw = bytes.fromhex(encoded)
        assert len(raw) == 80
        assert int.from_bytes(raw[:4], "little") == 1
        assert raw[4:36] == previous_hash_raw, f"bad parent at {height}"
        timestamp = int.from_bytes(raw[68:72], "little")
        bits = int.from_bytes(raw[72:76], "little")
        nonce = int.from_bytes(raw[76:80], "little")
        assert timestamp == expected_time and nonce == expected_nonce
        assert bits == BITS
        digest = hashlib.sha256(hashlib.sha256(raw).digest()).digest()
        assert digest[::-1].hex() == expected_hash, f"bad SHA256d hash at {height}"
        # Mainnet powLimit with the historical canonical compact target.
        target = (bits & 0x007FFFFF) << (8 * ((bits >> 24) - 3))
        assert 0 < int.from_bytes(digest, "little") <= target
        if past_times:
            median = sorted(past_times[-11:])[len(past_times[-11:]) // 2]
            assert timestamp > median, f"MTP violation at {height}"
        past_times.append(timestamp)
        chainwork += (1 << 256) // (target + 1)
        previous_hash_raw = digest
    assert chainwork == 4 * 4_295_032_833
    print(f"Stage 18 historical headers verified: {len(lines)} (tip height 3)")
    print(f"independent chainwork: {chainwork:064x}")
    print("best_active_chain=UNVERIFIED; economic_validity=UNVERIFIED")


if __name__ == "__main__":
    main()
