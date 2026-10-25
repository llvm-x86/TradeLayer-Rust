#!/usr/bin/env python3
"""Stage25K offline independent RIPEMD160/HASH160 known-answer gate.

hashlib providers RIPEMD160 come from system OpenSSL. No network,
pip packages, nodes, signatures, wallet keys or mutable ledger inputs.
Unlike local Rust src/hash160.rs, this harness calls hashlib directly.
"""
import hashlib


def ripemd160(data: bytes) -> bytes:
    try:
        digest = hashlib.new("ripemd160")
    except ValueError as error:
        raise SystemExit(
            "FAIL: hashlib/OpenSSL provider has no RIPEMD160; do not skip "
            "independent P2SH validation"
        ) from error
    digest.update(data)
    return digest.digest()


def hash160(data: bytes) -> bytes:
    return ripemd160(hashlib.sha256(data).digest())


def main() -> None:
    vectors = [
        (b"", "9c1185a5c5e9fc54612808977ee8f548b2258d31"),
        (b"a", "0bdc9d2d256b3ee9daae347be6f4dc835a467ffe"),
        (b"abc", "8eb208f7e05d987a9b044a8e98c6b087f15a0bfc"),
        (b"message digest", "5d0689ef49d2fae572b881b123a85ffa21595f36"),
        (b"a" * 55, "0d8a8c9063a48576a7c97e9f95253a6e53ff6765"),
        (b"a" * 56, "e72334b46c83cc70bef979e15453706c95b888be"),
        (b"a" * 1_000_000, "52783243c1697bdbe16d37f97f68f08325dc1528"),
    ]
    for content, expected in vectors:
        assert ripemd160(content).hex() == expected, (
            f"bad RIPEMD160 vector len={len(content)}"
        )

    x = bytes.fromhex(
        "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    )
    left = b"\x02" + x
    right = b"\x03" + x
    scripts = (
        (left, right, "3fb5b5fbb96ad2b002a5ab6f3e05da931641c0f4"),
        (right, left, "62c5a9dce49968f5ebf6691fb6fa43e1ebede59d"),
    )
    for first, second, expected in scripts:
        script = b"\x52\x21" + first + b"\x21" + second + b"\x52\xae"
        assert len(script) == 71
        assert hash160(script).hex() == expected, "unexpected P2SH script hash"
        p2sh = b"\xa9\x14" + hash160(script) + b"\x87"
        assert len(p2sh) == 23
        assert p2sh[0:2] == b"\xa9\x14" and p2sh[-1] == 0x87
    print(
        "PASS: 7 RIPEMD160 known-answer messages and two independently "
        "hashed 2-of-2 P2SH script orders"
    )
    print("BOUNDARY: no ECDSA, legacy sighash, chain UTXO or liquidation authority")


if __name__ == "__main__":
    main()
