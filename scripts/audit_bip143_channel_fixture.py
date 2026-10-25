#!/usr/bin/env python3
"""Independent stdlib SHA256d oracle for Stage25I's fake P2WSH channel.

No network, cryptographic signing, external dependencies, crates, or funding.
Reconstructs the fixture from serialized bytes, rather than consuming Rust's
BIP143 implementation or precomputed hash inputs.
"""

from hashlib import sha256
from struct import pack


def dhash(data: bytes) -> bytes:
    return sha256(sha256(data).digest()).digest()


def compact(data: bytes) -> bytes:
    assert len(data) < 253
    return bytes((len(data),)) + data


def audit() -> None:
    # Published BIP143 P2WPKH SIGHASH_ALL preimage/digest example:
    # https://bitcoin.org/bip/143/
    published = bytes.fromhex(
        "01000000"
        "96b827c8483d4e9b96712b6713a7b68d6e8003a781feba36c31143470b4efd37"
        "52b0a642eea2fb7ae638c36f6252b6750293dbe574a806984b8e4d8548339a3b"
        "ef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a01000000"
        "1976a9141d0f172a0ecb48aee1be1f2687d2963ae33f71a188ac"
        "0046c32300000000"
        "ffffffff"
        "863ef3e1a92afbfdb97f31ad0fc7683ee943e9abcf2501590ff8f6551f47e5e5"
        "11000000"
        "01000000"
    )
    assert dhash(published).hex() == (
        "c37af31116d1b27caf68aae9e3ac82f1477929014d5b917657d0eb49478cb670"
    ), "published BIP143 known-answer digest changed"

    x = bytes.fromhex(
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    )
    witness_script = b"\x52\x21" + b"\x02" + x + b"\x21" + b"\x03" + x + b"\x52\xae"
    assert len(witness_script) == 71
    witness_program = sha256(witness_script).digest()
    assert witness_program.hex() == (
        "c8e20d4b82f2ef132735e3db615393c5c8a1a9564405fd9608466a893fcab68f"
    )
    funding = (
        pack("<I", 2) + b"\x01" + b"\x11" * 32 + pack("<I", 1) +
        compact(b"") + pack("<I", 0xFFFFFFFF) + b"\x01" +
        pack("<Q", 10_000) + compact(b"\x00\x20" + witness_program) +
        pack("<I", 0)
    )
    funding_txid = dhash(funding)
    assert funding_txid.hex() == (
        "e9de48b747b4e24f9aa48640a48da898c6cd02e13ef6212681b7c3ecbd887ac8"
    )

    outpoint_0 = b"\x22" * 32 + pack("<I", 7)
    outpoint_1 = funding_txid + pack("<I", 0)
    all_seq = pack("<II", 0xFFFFFFFF, 0xFFFFFFFF)
    output = pack("<Q", 8_000) + compact(b"\x51")
    h_prevouts = dhash(outpoint_0 + outpoint_1)
    h_sequence = dhash(all_seq)
    h_outputs = dhash(output)
    assert h_prevouts.hex() == (
        "ea816a41f5f26c3bf6798e1120432065e3ff7679880d45d1bc24820e23d3d308"
    )
    assert h_sequence.hex() == (
        "752adad0a7b9ceca853768aebb6965eca126a62965f698a0c1bc43d83db632ad"
    )
    assert h_outputs.hex() == (
        "7ce42f6d181802782fc3ea5329d76e8fe344cbc1aa49742806d5a71528d6cd49"
    )

    # Select vin=1. The sequence and outpoint of vin=0 STILL contribute
    # to hashSequence/hashPrevouts, as SIGHASH_ALL has no ANYONECANPAY.
    preimage = (
        pack("<I", 2) + h_prevouts + h_sequence + outpoint_1 +
        compact(witness_script) + pack("<Q", 10_000) +
        pack("<I", 0xFFFFFFFF) + h_outputs + pack("<I", 0) + pack("<I", 1)
    )
    assert len(preimage) == 228
    assert dhash(preimage).hex() == (
        "5dc9f28f470cf591f0e7aacd431bbbdd31c8011ba96f3eb13c17e1ec8d408210"
    )
    assert dhash(preimage.replace(pack("<Q", 10_000), pack("<Q", 10_001))).hex() != dhash(preimage).hex()
    print("PASS: published BIP143, independently serialized 2-of-2 P2WSH SIGHASH_ALL")


if __name__ == "__main__":
    audit()
