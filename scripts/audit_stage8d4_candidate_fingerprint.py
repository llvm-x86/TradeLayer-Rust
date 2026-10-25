#!/usr/bin/env python3
"""Independent Python SHA256d oracle for the untrusted Stage8D4 witness codec.

Only a candidate-table checksum is checked. This is NOT a Bitcoin UTXO
commitment, chain proof, amount/signature verification or protocol admission.
"""
import hashlib
import struct

DOMAIN = b"TRDL-PARENT-UTXO-WITNESS-v1\x00"
PARENT_HASH = bytes.fromhex("0123456789abcdef" * 4)
PARENT_HEIGHT = 903_000
EXPECTED = "6b8b85dde893bbc5adf6c3e50e3cae6d11984038d25bc3d329e6dc76ea4fe198"
RECORDS = (
    (bytes([0x11]) * 32, 2, 12_345_678, 902_500, False, bytes.fromhex("0014" + "aa" * 20)),
    (bytes([0x22]) * 32, 0, 5_000_000_000, 902_000, True, b"\x51"),
)


def fingerprint(records=RECORDS, parent_hash=PARENT_HASH, height=PARENT_HEIGHT):
    raw = DOMAIN + parent_hash + struct.pack("<II", height, len(records))
    for txid, vout, amount, created, coinbase, script in records:
        assert len(txid) == 32
        raw += txid + struct.pack("<IQIBI", vout, amount, created, int(coinbase), len(script))
        raw += script
    return raw, hashlib.sha256(hashlib.sha256(raw).digest()).hexdigest()


def main():
    raw, digest = fingerprint()
    assert len(DOMAIN) == 28
    assert len(raw) == 197, len(raw)
    assert digest == EXPECTED, (digest, EXPECTED)
    assert fingerprint(tuple(reversed(RECORDS)))[1] != digest
    assert fingerprint(parent_hash=bytes(32))[1] != digest
    altered = list(RECORDS)
    txid, vout, amount, created, coinbase, script = altered[0]
    altered[0] = (txid, vout, amount + 1, created, coinbase, script)
    assert fingerprint(tuple(altered))[1] != digest
    print("stage8d4_candidate_witness_sha256d=INDEPENDENT_PYTHON_MATCH")
    print("parent_utxo_authentication=UNVERIFIED")
    print("bitcoin_script_consensus=UNVERIFIED")


if __name__ == "__main__":
    main()
