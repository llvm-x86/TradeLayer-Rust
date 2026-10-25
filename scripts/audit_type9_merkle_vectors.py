#!/usr/bin/env python3
"""Stage25N stdlib SHA256d verification of synthetic type9 relative Merkle corpus.

Independent of Rust. All headers/inputs/OP_RETURNs are fabricated regtest-like
vectors and DO NOT establish Bitcoin active-chain/UTXO/admin/channel authority.
"""
from pathlib import Path
import hashlib
import struct

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "fixtures/channel_type9_merkle_synthetic.tsv"
PAYLOADS = (
    b"tl90,7,alice,OK",
    b"tl90,b,bob,OK",
    b"tl91,7,alice,OK",
    b"tl40,7",
    b"tlj1,2",
)


def sha256d(value: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(value).digest()).digest()


def synthetic_tx(index: int, payload: bytes) -> bytes:
    if len(payload) > 75:
        raise ValueError("this fixture accepts only short direct pushes")
    script = b"\x6a" + bytes([len(payload)]) + payload
    return (
        struct.pack("<I", 2) + b"\x01" + bytes([index + 1]) * 32
        + struct.pack("<I", 0) + b"\x00" + struct.pack("<I", 0xfffffffe)
        + b"\x01" + struct.pack("<Q", 0) + bytes([len(script)]) + script
        + struct.pack("<I", 0)
    )


def siblings_at(txids: list[bytes], selected: int) -> list[bytes]:
    siblings = []
    level = txids[:]
    index = selected
    while len(level) > 1:
        sibling_index = index + 1 if index % 2 == 0 else index - 1
        siblings.append(level[sibling_index] if sibling_index < len(level) else level[index])
        level = [
            sha256d(level[i] + (level[i + 1] if i + 1 < len(level) else level[i]))
            for i in range(0, len(level), 2)
        ]
        index //= 2
    return siblings


def merkle_root(txids: list[bytes]) -> bytes:
    level = txids[:]
    while len(level) > 1:
        level = [
            sha256d(level[i] + (level[i + 1] if i + 1 < len(level) else level[i]))
            for i in range(0, len(level), 2)
        ]
    return level[0]


def branch_root(leaf: bytes, index: int, count: int, siblings: list[bytes]) -> bytes:
    h = leaf
    for sibling in siblings:
        if count < 2:
            raise ValueError("too many siblings")
        has_real_sibling = index % 2 == 1 or index + 1 < count
        if not has_real_sibling and h != sibling:
            raise ValueError("invalid synthetic merkle padding")
        if has_real_sibling and h == sibling:
            raise ValueError("real duplicate Merkle siblings")
        h = sha256d(h + sibling if index % 2 == 0 else sibling + h)
        index //= 2
        count = (count + 1) // 2
    if count != 1:
        raise ValueError("incomplete Merkle proof")
    return h


def load() -> dict[str, str]:
    fields = {}
    for line in FIXTURE.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        key, value = line.split("\t")
        if key in fields:
            raise ValueError("duplicate fixture field")
        fields[key] = value
    return fields


def main() -> None:
    f = load()
    assert f["anchor_height"] == "100"
    assert f["anchor_hash_display"] == "6a" * 32
    assert f["tx_count"] == "5"
    txs = [synthetic_tx(i, payload) for i, payload in enumerate(PAYLOADS)]
    txids = [sha256d(tx) for tx in txs]
    root = merkle_root(txids)
    for i, (tx, txid) in enumerate(zip(txs, txids)):
        assert bytes.fromhex(f[f"tx{i}_hex"]) == tx
        assert bytes.fromhex(f[f"tx{i}_id_raw"]) == txid
        frozen = [bytes.fromhex(x) for x in f[f"tx{i}_merkle"].split(",")]
        assert frozen == siblings_at(txids, i)
        assert branch_root(txid, i, len(txs), frozen) == root

    header = bytes.fromhex(f["header_hex"])
    assert len(header) == 80 and header[4:36] == bytes([0x6a]) * 32
    assert header[36:68] == root
    assert struct.unpack("<I", header[72:76])[0] == 0x207fffff
    assert sha256d(header)[::-1].hex() == f["block_hash_display"]
    target = 0x7fffff << (8 * (0x20 - 3))
    assert int.from_bytes(sha256d(header), "little") <= target

    # Falsify a Merkle sibling in the independent reference computation.
    wrong = [bytes([siblings_at(txids, 1)[0][0] ^ 1]) + siblings_at(txids, 1)[0][1:]] + siblings_at(txids, 1)[1:]
    assert branch_root(txids[1], 1, len(txs), wrong) != root

    # The same signed/type9 claim cannot be assigned another transaction's
    # index or another payload while keeping the frozen Merkle commitment.
    assert branch_root(txids[1], 0, len(txs), siblings_at(txids, 1)) != root
    changed = bytearray(txs[1])
    changed[55] ^= 1
    assert sha256d(bytes(changed)) != txids[1]
    assert branch_root(sha256d(bytes(changed)), 1, len(txs), siblings_at(txids, 1)) != root

    print("PASS: independent SHA256d matches all 5 synthetic transaction IDs and 5 Merkle paths")
    print("PASS: wrong sibling, altered exact raw payload, wrong transaction index reject")
    print("BOUNDARY: header anchor, script/sender, type9 admin and state revision NOT authenticated")


if __name__ == "__main__":
    main()
