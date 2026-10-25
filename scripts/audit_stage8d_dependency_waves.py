#!/usr/bin/env python3
"""Source-independent offline wavefront reference for raw Bitcoin-like bytes.

No PoW, UTXO snapshot, script signatures, coinbase value or consensus
acceptance. Input order and hashes are verified only for synthetic fixture.
"""
import hashlib
import struct
from pathlib import Path

FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "raw_block_ingest_synthetic.hex"


def sha256d(data):
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def varint(raw, offset):
    tag = raw[offset]
    offset += 1
    if tag < 253:
        return tag, offset
    sizes = {253: 2, 254: 4, 255: 8}
    n = sizes[tag]
    value = int.from_bytes(raw[offset:offset + n], "little")
    if value < (253 if n == 2 else 65536 if n == 4 else 4294967296):
        raise ValueError("noncanonical compactsize")
    return value, offset + n


def transaction(raw, offset):
    start = offset
    version = raw[offset:offset + 4]
    offset += 4
    witness = raw[offset:offset + 2] == b"\x00\x01"
    if witness:
        offset += 2
    stripped_start = offset
    n_in, offset = varint(raw, offset)
    inputs = []
    for _ in range(n_in):
        parent = raw[offset:offset + 32]
        vout = struct.unpack_from("<I", raw, offset + 32)[0]
        inputs.append((parent, vout))
        offset += 36
        n, offset = varint(raw, offset)
        offset += n + 4
    n_out, offset = varint(raw, offset)
    for _ in range(n_out):
        offset += 8
        n, offset = varint(raw, offset)
        offset += n
    stripped_end = offset
    if witness:
        for _ in range(n_in):
            n, offset = varint(raw, offset)
            for _ in range(n):
                length, offset = varint(raw, offset)
                offset += length
    locktime = raw[offset:offset + 4]
    offset += 4
    full = raw[start:offset]
    stripped = version + raw[stripped_start:stripped_end] + locktime if witness else full
    return {"raw": full, "txid": sha256d(stripped), "inputs": inputs,
            "outputs": n_out, "has_witness": witness, "end": offset}


def read_block(raw):
    count, offset = varint(raw, 80)
    result = []
    for _ in range(count):
        item = transaction(raw, offset)
        offset = item["end"]
        result.append(item)
    assert offset == len(raw), "trailing block bytes"
    return result


def waves(items):
    txid_to_index = {item["txid"]: i for i, item in enumerate(items)}
    spent = {}
    levels = []
    external = edges = 0
    for i, item in enumerate(items):
        inputs = item["inputs"]
        coinbase = len(inputs) == 1 and inputs[0] == (bytes(32), 0xffffffff)
        assert coinbase == (i == 0)
        wave = 0
        if not coinbase:
            for outpoint in inputs:
                assert outpoint not in spent, "duplicate input"
                spent[outpoint] = i
                parent = txid_to_index.get(outpoint[0])
                if parent is None:
                    external += 1
                    continue
                assert parent < i, "forward reference"
                assert outpoint[1] < items[parent]["outputs"], "invalid vout"
                assert parent != 0, "coinbase immature"
                wave = max(wave, levels[parent] + 1)
                edges += 1
        levels.append(wave)
    return levels, external, edges


def mutate_parent(tx, parent):
    # Witness marker precedes the input count on SegWit serialized TXs.
    offset = 7 if tx[4:6] == b"\x00\x01" else 5
    copy = bytearray(tx)
    copy[offset:offset + 32] = parent
    copy[offset + 32:offset + 36] = bytes(4)
    return bytes(copy)


def main():
    raw = bytes.fromhex(FIXTURE.read_text(encoding="ascii").strip())
    initial = read_block(raw)
    assert len(initial) == 4
    assert waves(initial) == ([0, 0, 0, 0], 3, 0)
    assert [tx["has_witness"] for tx in initial] == [False, False, True, True]
    raw_txs = [x["raw"] for x in initial]
    raw_txs[2] = mutate_parent(raw_txs[2], initial[1]["txid"])
    middle = transaction(raw_txs[2], 0)
    raw_txs[3] = mutate_parent(raw_txs[3], middle["txid"])
    chained = [transaction(tx, 0) for tx in raw_txs]
    assert waves(chained) == ([0, 0, 1, 2], 1, 2)
    print("PASS: independent Python TXID-derived dependency waves 0,0,0,0 / 0,0,1,2")
    print("PASS: 3 external unknown prevouts initially, 2 in-block edges in chained fixture")
    print("BOUNDARY: fabricated Bitcoin-style TXs, no authenticated UTXO or script execution")


if __name__ == "__main__":
    main()
