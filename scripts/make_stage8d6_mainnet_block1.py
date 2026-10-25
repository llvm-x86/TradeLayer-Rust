#!/usr/bin/env python3
"""Build raw Bitcoin mainnet block 1 from pinned published header + coinbase.

Only a local header/Merkle equality check is performed; no chain authorization.
"""
import hashlib
import sys
from pathlib import Path


def sha256d(payload):
    return hashlib.sha256(hashlib.sha256(payload).digest()).digest()


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: make_stage8d6_mainnet_block1.py OUTPUT.hex")
    root = Path(__file__).resolve().parents[1]
    headers = (root / "fixtures/bitcoin_mainnet_genesis_block1_headers.txt").read_text().splitlines()
    assert len(headers) >= 2
    header = bytes.fromhex(headers[1].strip())
    tx = bytes.fromhex((root / "fixtures/bitcoin_mainnet_block1_coinbase.hex").read_text().strip())
    assert len(header) == 80
    assert sha256d(tx) == header[36:68], "published block-1 header/coinbase Merkle mismatch"
    payload = header + b"\x01" + tx
    Path(sys.argv[1]).write_text(payload.hex() + "\n")
    print("mainnet_block1_merkle=LOCAL_CHECK_ONLY")
    print("bitcoin_consensus=UNVERIFIED")


if __name__ == "__main__":
    main()
