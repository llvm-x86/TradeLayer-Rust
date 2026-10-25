#!/usr/bin/env python3
"""Stage25P independent seven-transaction synthetic Merkle/serialization audit.

Python standard library alone. Every source-like type7/8/9 and type4 byte
vector is fabricated and bound to a fabricated regtest-like SHA256d header.
No accepted-chain, historical sender, admin, signature or registry proofs.
"""
import hashlib
import struct
from pathlib import Path

FILE = Path(__file__).resolve().parents[1] / "fixtures/channel_admin_789_merkle_synthetic.tsv"
PAYLOADS = (
    "tl7backupB,ListOne,https://example.invalid,note",
    "tl90,1,alice,OK",
    "tl40,1",
    "tl8adminC,1,0,0,1,0",
    "tl91,1,alice,OK",
    "tl40,1",
    "tl90,1,alice,OK",
)


def sha256d(data: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def encode(i: int, message: str) -> bytes:
    payload = message.encode("ascii")
    if len(payload) > 75:
        raise ValueError("noncanonical fixture push")
    script = b"\x6a" + bytes([len(payload)]) + payload
    return (
        struct.pack("<I", 2) + b"\x01" + bytes([i + 1]) * 32
        + struct.pack("<I", 0) + b"\x00" + struct.pack("<I", 0xfffffffe)
        + b"\x01" + struct.pack("<Q", 0) + bytes([len(script)])
        + script + struct.pack("<I", 0)
    )


def layers(hashes):
    level = hashes[:]
    while len(level) > 1:
        yield level
        level = [
            sha256d(level[i] + (level[i + 1] if i + 1 < len(level) else level[i]))
            for i in range(0, len(level), 2)
        ]
    yield level


def merkle_branch(hashes, index):
    out = []
    row = hashes[:]
    while len(row) > 1:
        sibling = index - 1 if index & 1 else index + 1
        out.append(row[sibling] if sibling < len(row) else row[index])
        row = [
            sha256d(row[i] + (row[i + 1] if i + 1 < len(row) else row[i]))
            for i in range(0, len(row), 2)
        ]
        index //= 2
    return out


def replay_branch(leaf, index, count, siblings):
    item = leaf
    for sibling in siblings:
        if count <= 1:
            raise ValueError("too many Merkle siblings")
        true_sibling = index & 1 or index + 1 < count
        if true_sibling and item == sibling:
            raise ValueError("ambiguous duplicate sibling")
        if not true_sibling and item != sibling:
            raise ValueError("invalid padding duplicate")
        item = sha256d(item + sibling if index % 2 == 0 else sibling + item)
        index //= 2
        count = (count + 1) // 2
    if count != 1:
        raise ValueError("incomplete proof")
    return item


def load():
    out = {}
    for line in FILE.read_text(encoding="utf8").splitlines():
        if not line or line.startswith("#"):
            continue
        key, value = line.split("\t", 1)
        if key in out:
            raise ValueError("repeated Stage25P fixture key")
        out[key] = value
    return out


def main():
    fields = load()
    assert fields["tx_count"] == "7"
    assert fields["anchor_height"] == "100"
    assert fields["anchor_hash_display"] == "66" * 32
    raw = [encode(i, payload) for i, payload in enumerate(PAYLOADS)]
    txids = [sha256d(tx) for tx in raw]
    root = list(layers(txids))[-1][0]

    for i in range(7):
        assert fields[f"tx{i}_payload"] == PAYLOADS[i]
        assert bytes.fromhex(fields[f"tx{i}_hex"]) == raw[i]
        assert bytes.fromhex(fields[f"tx{i}_id_raw"]) == txids[i]
        got = [bytes.fromhex(part) for part in fields[f"tx{i}_merkle"].split(",")]
        assert got == merkle_branch(txids, i)
        assert replay_branch(txids[i], i, len(txids), got) == root

    header = bytes.fromhex(fields["header_hex"])
    assert len(header) == 80
    assert header[4:36] == b"\x66" * 32
    assert header[36:68] == root
    assert struct.unpack("<I", header[72:76])[0] == 0x207fffff
    assert sha256d(header)[::-1].hex() == fields["block_hash_display"]
    assert int.from_bytes(sha256d(header), "little") <= 0x7fffff << (8 * 29)

    old = merkle_branch(txids, 1)
    wrong = old[:]
    wrong[0] = bytes([old[0][0] ^ 1]) + old[0][1:]
    assert replay_branch(txids[1], 1, 7, wrong) != root
    assert replay_branch(txids[1], 0, 7, old) != root
    assert sha256d(raw[1][:-1] + b"\x01") != txids[1]

    print("PASS: 7 independent SHA256d raw-TXIDs, branch proofs and claimed header PoW")
    print("PASS: wrong sibling/index and altered transaction bytes invalidate evidence")
    print("BOUNDARY: caller-anchored fake block, sender claims, source type8 administrator validation all unverified")


if __name__ == "__main__":
    main()
