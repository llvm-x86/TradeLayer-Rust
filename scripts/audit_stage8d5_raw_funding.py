#!/usr/bin/env python3
"""Independent Bitcoin raw funding txid and selected output byte oracle.

Proves no Bitcoin funding inclusion, UTXO membership, signatures or TL state.
This is a pure hashlib/struct reference for Stage8D5's Rust fixture.
"""
import hashlib
import struct

FUNDING_TXID_RAW = "01adfa763d8cb8bdf850869cd34797535ecd02ff76ff6ee7d28e97d35e85ffd8"
SEED = 0x53
EXPECTED_AMOUNTS = (13000, 42000, 90000)
EXPECTED_SCRIPTS = (
    bytes([0x51]),
    bytes.fromhex("0014") + bytes([SEED]) * 20,
    bytes([0x6A, 1, SEED]),
)

def make_raw_funding():
    raw = struct.pack("<I", 2) + b"\x01"
    raw += bytes([SEED]) * 32 + struct.pack("<I", 0)
    raw += b"\x01\x51" + struct.pack("<I", 0xFFFFFFFF)
    raw += bytes([len(EXPECTED_AMOUNTS)])
    for amount, script in zip(EXPECTED_AMOUNTS, EXPECTED_SCRIPTS):
        raw += struct.pack("<Q", amount) + bytes([len(script)]) + script
    return raw + struct.pack("<I", 0)


def main():
    raw = make_raw_funding()
    assert len(raw) == 105
    digest = hashlib.sha256(hashlib.sha256(raw).digest()).digest()
    assert digest.hex() == FUNDING_TXID_RAW

    # Independent byte-level output extraction. All fixture CompactSize
    # lengths are <253 and encoded in a single byte, unlike arbitrary
    # Bitcoin transaction parsing in the Rust production reference.
    offset = 4 + 1 + 32 + 4 + 1 + 1 + 4
    assert raw[offset] == 3
    offset += 1
    for index, (amount, script) in enumerate(zip(EXPECTED_AMOUNTS, EXPECTED_SCRIPTS)):
        assert struct.unpack_from("<Q", raw, offset)[0] == amount, index
        offset += 8
        assert raw[offset] == len(script), index
        offset += 1
        assert raw[offset:offset + len(script)] == script, index
        offset += len(script)
    assert offset == len(raw) - 4

    def txid(candidate):
        return hashlib.sha256(hashlib.sha256(candidate).digest()).digest()

    mutated = bytearray(raw)
    mutated[-9] ^= 1
    assert txid(mutated) != digest
    assert txid(raw[:4] + b"\x00\x01" + raw[4:-4] + b"\x01\x01\x51" + raw[-4:]) != digest

    print("stage8d5_raw_funding_sha256d=INDEPENDENT_PYTHON_MATCH")
    print("stage8d5_raw_funding_prevouts=3")
    print("funding_chain_inclusion=UNVERIFIED")
    print("script_consensus=UNVERIFIED")


if __name__ == "__main__":
    main()
