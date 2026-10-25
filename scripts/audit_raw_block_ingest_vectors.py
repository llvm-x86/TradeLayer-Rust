#!/usr/bin/env python3
"""Independent stdlib byte oracle for Stage8C one-fetch raw Bitcoin block.

Fabricated parent inputs, fake header and no genuine PoW, UTXO, signature,
witness commitment, best chain or TradeLayer authorization. The frozen block
has valid canonical serialization for the read-only scanner.
"""
import hashlib
import struct
from pathlib import Path

PATH = Path(__file__).resolve().parents[1] / "fixtures" / "raw_block_ingest_synthetic.hex"


def digest(data):
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def compact(count):
    if count < 253:
        return bytes([count])
    if count <= 0xffff:
        return b"\xfd" + struct.pack("<H", count)
    return b"\xfe" + struct.pack("<I", count)


def tx(n, segwit, script, witness=b"", coinbase=False):
    raw = bytearray(struct.pack("<I", 2))
    if segwit:
        raw.extend(b"\x00\x01")
    raw.extend(b"\x01")
    raw.extend(b"\x00" * 32 if coinbase else bytes([n]) * 32)
    raw.extend(struct.pack("<I", 0xffffffff if coinbase else 0))
    sig = b"\x01\x01" if coinbase else b""
    raw.extend(compact(len(sig)) + sig)
    raw.extend(struct.pack("<I", 0xffffffff if coinbase else 0xfffffffe))
    raw.extend(b"\x01")
    raw.extend(struct.pack("<Q", 50 * 100_000_000 if coinbase else 0))
    raw.extend(compact(len(script)) + script)
    if segwit:
        raw.extend(b"\x01" + compact(len(witness)) + witness)
    raw.extend(struct.pack("<I", 0))
    return bytes(raw)


def base(tx_bytes, segwit):
    if not segwit:
        return tx_bytes
    pos = 6
    start = pos
    assert tx_bytes[pos] == 1
    pos += 1 + 36
    script_size = tx_bytes[pos]
    pos += 1 + script_size + 4
    assert tx_bytes[pos] == 1
    pos += 1 + 8
    output_size = tx_bytes[pos]
    pos += 1 + output_size
    return tx_bytes[:4] + tx_bytes[start:pos] + tx_bytes[-4:]


def merkle(hashes):
    items = hashes[:]
    while len(items) > 1:
        items = [
            digest(items[i] + (items[i + 1] if i + 1 < len(items) else items[i]))
            for i in range(0, len(items), 2)
        ]
    return items[0]


def main():
    carrier = b"\x6a\x0etl20;alice;5;2"
    records = [
        tx(0, False, b"\x51", coinbase=True),
        tx(1, False, carrier),
        tx(2, True, carrier, b"abc"),
        tx(3, True, b"\x51", b"XY"),
    ]
    stripped = [base(t, i >= 2) for i, t in enumerate(records)]
    txids = [digest(s) for s in stripped]
    wtxids = [digest(t) for t in records]
    root = merkle(txids)
    header = (
        struct.pack("<I", 1)
        + b"\x66" * 32
        + root
        + struct.pack("<III", 1_600_000_000, 0x207fffff, 0)
    )
    block = header + compact(4) + b"".join(records)
    frozen = bytes.fromhex(PATH.read_text(encoding="ascii").strip())
    assert frozen == block, "original-source-independent fixture bytes changed"
    assert len(block) == 370
    assert 80 + 1 + sum(map(len, stripped)) == 357
    assert len(block) + 3 * (80 + 1 + sum(map(len, stripped))) == 1441
    assert root.hex() == "760e6d8636fd5304a65989b461a27a88f48545bc1cc6e2b4c2016a638d2e8a9c"
    assert digest(header).hex() == "09667b766e1bccf918412b65f62966d7a2803613592fed34ba9d27ac608eb0c0"
    assert txids[0].hex() == "8647c745029df431acac0ff4f8a91ff00d6194ef87a705946f784af9f213bffa"
    assert txids[1].hex() == "49f3b25cef04ddad2b71d1dbfa5cf1b5b2f0bc39ae73eaa395de79e0a17c4af3"
    assert txids[2].hex() == "f878d360c2a1b812ef5aa5b2706f0f56aa34dba00b4f40fe07000571f00208b1"
    assert wtxids[2].hex() == "087f19b8125e0a640993f6add2ee73bf2f2189edb5e2f6f4d9a64e36b4f553df"
    assert txids[3].hex() == "4f61a8c818de93316dc5b69b57d8fdae42c774056908109397d8381e7e8dcfac"
    assert wtxids[3].hex() == "49952b6ed3eee478863a259530e17709f104577e32a9521155abf5e5fa2784c1"
    assert all(txids[i] == wtxids[i] for i in (0, 1))
    assert all(txids[i] != wtxids[i] for i in (2, 3))
    # Mutating only a witness byte changes wtxid but preserves the stripped txid.
    altered = bytearray(records[2])
    altered[-5] ^= 1
    assert digest(altered) != wtxids[2]
    assert digest(base(altered, True)) == txids[2]
    print("PASS: exact 370-byte fake block, 4 Python-built raw TXs and SHA256d txids")
    print("PASS: TXID Merkle 760e6d86..., block digest, total=370, base=357, weight=1441")
    print("PASS: witness-only mutation changes wtxid but not stripped TXID")
    print("BOUNDARY: fabricated coinbase/UTXOs and PoW; no full Bitcoin consensus")


if __name__ == "__main__":
    main()
