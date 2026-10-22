#!/usr/bin/env python3
"""Offline assertion that Stage 21 mainnet genesis pins match upstream Core.

Sources (pinned at retrieval):
Bitcoin Core 152a55397e451fed9c10041bddf01aa6839847d5
  src/kernel/chainparams.cpp
Litecoin Core ec1b6489a900d09cf5991e220dce089c77a232a2
  src/chainparams.cpp
Dogecoin Core 3c1679e0fd4a99f5d640c317671ad9c0f341ea65
  src/chainparams.cpp

Compares byte arrays to independent hardcoded published mainnet identities.
No network access, dependency installation, or protocol authorization.
"""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "src" / "network_profile.rs"

EXPECTED = {
    "BITCOIN_MAINNET_GENESIS":
        "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
    "LITECOIN_MAINNET_GENESIS":
        "12a765e31ffd4059bada1e25190f6e98c99d9714d334efa41a195a7e7e04bfe2",
    "DOGECOIN_MAINNET_GENESIS":
        "1a91e3dace36e2be3bf030a65679fe821aa1d6ef92e7c9902eb318182c355691",
}


def main() -> None:
    source = SOURCE.read_text(encoding="utf-8")
    for label, expected in EXPECTED.items():
        match = re.search(
            rf"pub const {label}:\s*BlockId\s*=\s*BlockId\(\[\s*(.*?)\s*\]\)",
            source,
            flags=re.DOTALL,
        )
        if not match:
            raise AssertionError(f"missing pinned mainnet genesis: {label}")
        literals = re.findall(r"0x[0-9a-fA-F]{2}", match.group(1))
        assert len(literals) == 32, f"{label}: expected exactly 32 bytes"
        remaining = re.sub(r"0x[0-9a-fA-F]{2}|[\s,]", "", match.group(1))
        assert not remaining, f"{label}: unexpected literal content: {remaining!r}"
        actual = bytes(int(s, 16) for s in literals).hex()
        assert actual == expected, f"{label}: {actual} differs from upstream Core"
    assert len(set(EXPECTED.values())) == 3
    print("Three pinned, distinct mainnet genesis hashes verified from Rust constants.")
    print("Raito/Cairo integration: Bitcoin only (research; no proof authorization).")


if __name__ == "__main__":
    main()
