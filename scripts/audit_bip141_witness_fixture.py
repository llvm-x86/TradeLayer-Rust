#!/usr/bin/env python3
"""Independent hashlib BIP141 witness-commitment fixture oracle.

The coinbase and regtest-like headers are fabricated. Even a matching block
commitment does NOT authenticate real Bitcoin chain, UTXOs, signatures or
any TradeLayer channel/settlement operation.
"""
from hashlib import sha256
from pathlib import Path
from struct import pack

ROOT = Path(__file__).resolve().parents[1]
STAGE25J = ROOT / "fixtures/channel_p2wsh_ecdsa_signed_synthetic.tsv"
VECTORS = ROOT / "fixtures/stage25m_bip141_vectors.tsv"
PREFIX = bytes.fromhex("6a24aa21a9ed")
ANCHOR = bytes.fromhex(
    "02000000" + "34" * 32
    + "123874d4a84664ee8e041e639c7ff01f999953bcce8335ca2a35abece40f2bc0"
    + "00f15365ffff7f2000000000"
)
RESERVED = bytes.fromhex("12" * 32)


def ds(data: bytes) -> bytes:
    return sha256(sha256(data).digest()).digest()


def compact(n: int) -> bytes:
    assert 0 <= n < 253
    return bytes([n])


def push(data: bytes) -> bytes:
    return compact(len(data)) + data


def spend_id(raw: bytes) -> bytes:
    # Parse the exact public two-input fixture's canonical variable-length
    # scripts and strip all witness bytes to recover the real non-witness TXID.
    assert raw[4:6] == b"\x00\x01"
    offset = 6
    n_inputs = raw[offset]
    offset += 1
    assert n_inputs == 2
    for _ in range(n_inputs):
        offset += 36
        size = raw[offset]
        offset += 1 + size + 4
    n_outputs = raw[offset]
    offset += 1
    assert n_outputs == 2
    for _ in range(n_outputs):
        offset += 8
        size = raw[offset]
        offset += 1 + size
    nonwitness = raw[:4] + raw[6:offset] + raw[-4:]
    for _ in range(n_inputs):
        stack_items = raw[offset]
        offset += 1
        for _ in range(stack_items):
            size = raw[offset]
            offset += 1 + size
    assert offset + 4 == len(raw)
    return ds(nonwitness)


def merkle(leaves: list[bytes]) -> bytes:
    assert leaves
    while len(leaves) > 1:
        next_level = []
        for i in range(0, len(leaves), 2):
            if i + 1 < len(leaves):
                assert leaves[i] != leaves[i + 1], "mutated Merkle subtree"
            right = leaves[i + 1] if i + 1 < len(leaves) else leaves[i]
            next_level.append(ds(leaves[i] + right))
        leaves = next_level
    return leaves[0]


def witness_commit(other: list[bytes], reserved: bytes = RESERVED) -> bytes:
    leaves = [bytes(32)]
    for data in other:
        leaves.append(ds(data) if data[4:6] == b"\x00\x01" else ds(data))
    return ds(merkle(leaves) + reserved)


def coinbase(commitments: list[bytes], reserved: bytes = RESERVED, trailing: bool = False) -> bytes:
    # Fake coinbase is BIP141-consistent but not a validated Bitcoin block.
    outputs = [pack("<Q", 5_000_000_000) + push(b"\x51")]
    for i, commitment in enumerate(commitments):
        script = PREFIX + commitment + (b"\x51" if trailing and i == len(commitments)-1 else b"")
        outputs.append(pack("<Q", 0) + push(script))
    return (
        pack("<I", 2) + b"\x00\x01" + b"\x01" + bytes(32)
        + pack("<I", 0xffffffff) + push(bytes.fromhex("03660001"))
        + pack("<I", 0xffffffff) + compact(len(outputs))
        + b"".join(outputs) + b"\x01" + push(reserved) + pack("<I", 0)
    )


def coinbase_txid(raw: bytes) -> bytes:
    assert raw[:6] == b"\x02\x00\x00\x00\x00\x01"
    assert raw[-38:-36] == b"\x01\x20"
    return ds(raw[:4] + raw[6:-38] + raw[-4:])


def fake_header(coin: bytes, other: list[bytes], stamp: int) -> bytes:
    txids = [coinbase_txid(coin)]
    for tx in other:
        txids.append(spend_id(tx) if tx[4:6] == b"\x00\x01" else ds(tx))
    root = merkle(txids)
    for nonce in range(2000):
        raw = pack("<I", 2) + ds(ANCHOR) + root + pack("<III", stamp, 0x207fffff, nonce)
        target = 0x7fffff << (8 * (0x20 - 3))
        if int.from_bytes(ds(raw), "little") <= target:
            return raw
    raise AssertionError("could not find synthetic header nonce")


def audit() -> None:
    fields = dict(
        row.split("\t", 1)
        for row in STAGE25J.read_text().splitlines()
        if row and not row.startswith("#")
    )
    funding = bytes.fromhex(fields["funding_hex"])
    spending = bytes.fromhex(fields["spending_hex"])
    assert ds(funding).hex() == (
        "123874d4a84664ee8e041e639c7ff01f999953bcce8335ca2a35abece40f2bc0"
    )
    assert spend_id(spending).hex() == (
        "66471b2c34f5c2b2c02becf4c6f557af02c2cf94b468e4a1f61a89042a40fd61"
    )
    assert ds(spending).hex() == (
        "09d5cd5ef2ea0372d250c39359a0b226d542937fee64ee604d79c92e8410e6fa"
    )
    good = witness_commit([spending])
    assert good.hex() == (
        "befe334eec4cb2e7485485564bc2c3dfea31310c10be172b9dbf96c2dc3947e6"
    )
    original = coinbase([good])
    first_header = fake_header(original, [spending], 1_700_000_001)
    assert coinbase_txid(original).hex() == (
        "d670fdaad9583f120cb7af5ba162ee06f33ac3713ecd1eb340bec2516bf1998a"
    )
    assert first_header.hex() == (
        "020000008703d309262d3cd9760351af10d252dcef5a8a2b140c05a193378dbc0230a272"
        "ec4a998d2e5d4bf9389a912920d93451f4f0fcf147ba8189a88f9a171774f978"
        "01f15365ffff7f2003000000"
    )
    needle = bytes.fromhex("3044022021d7c087")
    assert spending.count(needle) == 1
    tampered = bytearray(spending)
    tampered[spending.index(needle) + 5] ^= 1
    tampered = bytes(tampered)
    assert spend_id(tampered) == spend_id(spending)
    assert ds(tampered) != ds(spending)
    assert witness_commit([tampered]) != good
    fake_regenerated = coinbase([witness_commit([tampered])])
    bad_commitment = bytes([0x77]) * 32
    high_good = coinbase([bad_commitment, good])
    high_bad = coinbase([good, bad_commitment])
    with_trailing = coinbase([good], trailing=True)
    odd = coinbase([witness_commit([funding, spending])])
    expected = {
        "baseline_coinbase": original,
        "baseline_header": first_header,
        "higher_good_coinbase": high_good,
        "higher_good_header": fake_header(high_good, [spending], 1_700_000_011),
        "higher_bad_coinbase": high_bad,
        "higher_bad_header": fake_header(high_bad, [spending], 1_700_000_012),
        "trailing_coinbase": with_trailing,
        "trailing_header": fake_header(with_trailing, [spending], 1_700_000_013),
        "tampered_coinbase": fake_regenerated,
        "tampered_header": fake_header(fake_regenerated, [tampered], 1_700_000_014),
        "odd_coinbase": odd,
        "odd_header": fake_header(odd, [funding, spending], 1_700_000_015),
    }
    actual = dict(
        row.split("\t", 1)
        for row in VECTORS.read_text().splitlines()
        if row and not row.startswith("#")
    )
    assert set(actual) == set(expected), "missing, duplicate or extra BIP141 vector"
    for name, value in expected.items():
        assert actual[name] == value.hex(), f"independent BIP141 vector drift: {name}"
    print("PASS: BIP141 zero-coinbase-leaf, high-index, trailing, odd Merkle padding")
    print("PASS: forged-witness wtxid mismatches old commitment; regenerated fake block stays unverified")


if __name__ == "__main__":
    audit()
