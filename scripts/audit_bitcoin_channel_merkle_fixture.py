#!/usr/bin/env python3
"""Independent stdlib SHA256d reference for Stage25L UNTRUSTED fake headers.

Validates the fixed Stage25J serialized signed fixture, canonical non-witness
txids, header parent/Merkle/PoW and the SegWit witness-commitment gap.
No keys, nodes, network, wallet, broadcast, consensus or settlement.
"""

from hashlib import sha256
from pathlib import Path
from struct import pack

ROOT = Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "fixtures/channel_p2wsh_ecdsa_signed_synthetic.tsv"
ANCHOR_RAW = bytes([0x34]) * 32
REGTEST_TARGET = 0x7FFFFF << (8 * (0x20 - 3))


def dhash(data: bytes) -> bytes:
    return sha256(sha256(data).digest()).digest()


def compact(buf: bytes, off: int) -> tuple[int, int]:
    prefix = buf[off]
    if prefix < 253:
        return prefix, off + 1
    if prefix == 253:
        value = int.from_bytes(buf[off + 1:off + 3], "little")
        assert value >= 253
        return value, off + 3
    if prefix == 254:
        value = int.from_bytes(buf[off + 1:off + 5], "little")
        assert value > 0xFFFF
        return value, off + 5
    value = int.from_bytes(buf[off + 1:off + 9], "little")
    assert value > 0xFFFFFFFF
    return value, off + 9


def inspect(tx: bytes) -> tuple[bytes, bytes, list[bytes], list[bytes]]:
    assert len(tx) >= 10
    segwit = tx[4:6] == b"\x00\x01"
    off = 6 if segwit else 4
    input_count, off = compact(tx, off)
    assert 1 <= input_count <= 32
    inputs: list[bytes] = []
    for _ in range(input_count):
        start = off
        off += 36
        slen, off = compact(tx, off)
        off += slen + 4
        inputs.append(tx[start:off])
    output_count, off = compact(tx, off)
    assert 1 <= output_count <= 32
    outputs: list[bytes] = []
    for _ in range(output_count):
        start = off
        off += 8
        slen, off = compact(tx, off)
        off += slen
        outputs.append(tx[start:off])
    out_end = off
    if segwit:
        for _ in range(input_count):
            nstack, off = compact(tx, off)
            for _ in range(nstack):
                size, off = compact(tx, off)
                off += size
    assert off + 4 == len(tx), "trailing or truncated witness"
    nonwitness = tx[:4] + tx[6:out_end] + tx[-4:] if segwit else tx
    return dhash(nonwitness), dhash(tx), inputs, outputs


def synthetic_header(parent: bytes, root: bytes, stamp: int) -> bytes:
    # The published fixture uses the first nonce, zero, for easy independent
    # checking. Target 0x207fffff is a FAKE regtest-like research profile.
    header = pack("<I", 2) + parent + root + pack("<III", stamp, 0x207FFFFF, 0)
    assert len(header) == 80
    assert int.from_bytes(dhash(header), "little") <= REGTEST_TARGET
    return header


def main() -> None:
    fields = dict(
        line.split("\t", 1)
        for line in FIXTURE.read_text().splitlines()
        if line and not line.startswith("#")
    )
    funding = bytes.fromhex(fields["funding_hex"])
    spending = bytes.fromhex(fields["spending_hex"])
    fund_txid, _, _, fund_outputs = inspect(funding)
    spend_txid, spend_wtxid, spend_inputs, _ = inspect(spending)
    assert fund_txid.hex() == (
        "123874d4a84664ee8e041e639c7ff01f999953bcce8335ca2a35abece40f2bc0"
    )
    assert spend_txid.hex() == (
        "66471b2c34f5c2b2c02becf4c6f557af02c2cf94b468e4a1f61a89042a40fd61"
    )
    assert len(spend_inputs) == 2
    assert spend_inputs[1][:36] == fund_txid + pack("<I", 0)
    assert int.from_bytes(fund_outputs[0][:8], "little") == 20_000

    h101 = synthetic_header(ANCHOR_RAW, fund_txid, 1_700_000_000)
    h102 = synthetic_header(dhash(h101), spend_txid, 1_700_000_001)
    together = dhash(fund_txid + spend_txid)
    both = synthetic_header(ANCHOR_RAW, together, 1_700_000_010)

    expected_101 = bytes.fromhex(
        "020000003434343434343434343434343434343434343434343434343434343434343434"
        "123874d4a84664ee8e041e639c7ff01f999953bcce8335ca2a35abece40f2bc0"
        "00f15365ffff7f2000000000"
    )
    expected_102 = bytes.fromhex(
        "020000008703d309262d3cd9760351af10d252dcef5a8a2b140c05a193378dbc0230a272"
        "66471b2c34f5c2b2c02becf4c6f557af02c2cf94b468e4a1f61a89042a40fd61"
        "01f15365ffff7f2000000000"
    )
    expected_both = bytes.fromhex(
        "020000003434343434343434343434343434343434343434343434343434343434343434"
        "7fdf54fdcd9fefe4dc36b905be2658220e10aaa4839dc9ea0d8a2b192ee8f332"
        "0af15365ffff7f2000000000"
    )
    assert (h101, h102, both) == (expected_101, expected_102, expected_both)

    # A forged signature changes wtxid but leaves txid and ALL txid-only
    # Merkle proofs intact. Full block witness commitment and consensus are
    # genuinely separate obligations.
    needle = bytes.fromhex("3044022021d7c087")
    assert spending.count(needle) == 1
    start = spending.index(needle)
    altered = bytearray(spending)
    altered[start + 5] ^= 1
    forged_txid, forged_wtxid, _, _ = inspect(bytes(altered))
    assert forged_txid == spend_txid
    assert forged_wtxid != spend_wtxid
    print("PASS: 2 linked fake headers and intra-block Merkle pair")
    print("PASS: witness tampering reuses txid Merkle proof, never authorizes spend")
    print("BOUNDARY: claimed anchor, active chain, witness commitment and ECDSA UNVERIFIED")


if __name__ == "__main__":
    main()
