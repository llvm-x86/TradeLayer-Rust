#!/usr/bin/env python3
"""Stage25O: independent public-only Bitcoin P2WPKH sender/Type9 ECDSA fixture.

Pure Python stdlib reconstructs serialized transaction hash, P2WPKH BIP143,
HASH160, BIP173 bech32, relative synthetic Merkle and PoW. Preinstalled
system OpenSSL mathematically verifies the signature against the exact
pubkey. No private keys, signing, wallet, live node or economic authorization.
"""

from __future__ import annotations

import base64
import hashlib
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "fixtures/channel_type9_p2wpkh_signed_synthetic.tsv"
CHARS = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"


def sha256d(data: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def hash160(data: bytes) -> bytes:
    return hashlib.new("ripemd160", hashlib.sha256(data).digest()).digest()


def compact(n: int) -> bytes:
    if n < 0 or n > 100000:
        raise ValueError("unsupported CompactSize")
    if n < 253:
        return bytes([n])
    if n <= 0xffff:
        return b"\xfd" + struct.pack("<H", n)
    return b"\xfe" + struct.pack("<I", n)


class Reader:
    def __init__(self, raw: bytes):
        if len(raw) > 1_000_000:
            raise ValueError("oversized raw transaction")
        self.raw = raw
        self.offset = 0

    def take(self, n: int) -> bytes:
        if n < 0 or self.offset + n > len(self.raw):
            raise ValueError("truncated transaction")
        out = self.raw[self.offset:self.offset + n]
        self.offset += n
        return out

    def count(self, nonzero: bool = False) -> int:
        first = self.take(1)[0]
        if first < 253:
            n = first
        elif first == 253:
            n = struct.unpack("<H", self.take(2))[0]
            if n < 253:
                raise ValueError("noncanonical CompactSize")
        elif first == 254:
            n = struct.unpack("<I", self.take(4))[0]
            if n <= 0xffff:
                raise ValueError("noncanonical CompactSize")
        else:
            n = struct.unpack("<Q", self.take(8))[0]
            if n <= 0xffffffff:
                raise ValueError("noncanonical CompactSize")
        if n > 100000 or (nonzero and n == 0):
            raise ValueError("invalid input/output count")
        return n

    def blob(self) -> bytes:
        n = self.count()
        return self.take(n)


def parse_transaction(raw: bytes):
    r = Reader(raw)
    version = r.take(4)
    segwit = len(raw) > 5 and raw[4] == 0
    if segwit and r.take(2) != b"\x00\x01":
        raise ValueError("unsupported SegWit marker")
    start = r.offset
    inputs = []
    for _ in range(r.count(True)):
        inputs.append((r.take(36), r.blob(), r.take(4)))
    outputs = []
    for _ in range(r.count(True)):
        origin = r.offset
        sats = struct.unpack("<Q", r.take(8))[0]
        script = r.blob()
        outputs.append((sats, script, raw[origin:r.offset]))
    stop = r.offset
    witness = []
    if segwit:
        for _ in inputs:
            witness.append(tuple(r.blob() for _ in range(r.count())))
        if not any(item for stack in witness for item in stack):
            raise ValueError("superfluous SegWit marker")
    locktime = r.take(4)
    if r.offset != len(raw):
        raise ValueError("trailing bytes")
    unsigned = version + raw[start:stop] + locktime if segwit else raw
    return {
        "version": version,
        "inputs": inputs,
        "outputs": outputs,
        "witness": witness,
        "locktime": locktime,
        "segwit": segwit,
        "txid": sha256d(unsigned),
        "wtxid": sha256d(raw),
    }


def bech32_polymod(values):
    chk = 1
    for value in values:
        top = chk >> 25
        chk = ((chk & 0x1ffffff) << 5) ^ value
        for i, g in enumerate((0x3b6a57b2, 0x26508e6d, 0x1ea119fa,
                               0x3d4233dd, 0x2a1462b3)):
            if (top >> i) & 1:
                chk ^= g
    return chk


def address_from_program(program: bytes) -> str:
    # BIP173 version zero, five-bit groups padded with zeros.
    values = [0]
    acc, bits = 0, 0
    for byte in program:
        acc = (acc << 8) | byte
        bits += 8
        while bits >= 5:
            bits -= 5
            values.append((acc >> bits) & 31)
    if bits:
        values.append((acc << (5 - bits)) & 31)
    hrp = "bc"
    prefix = [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp]
    checksum = bech32_polymod(prefix + values + [0] * 6) ^ 1
    suffix = [(checksum >> (5 * (5 - i))) & 31 for i in range(6)]
    return hrp + "1" + "".join(CHARS[n] for n in values + suffix)


def signed_der(item: bytes) -> bytes:
    if len(item) < 9 or len(item) > 73 or item[-1] != 1:
        raise ValueError("unsupported signature hash or length")
    sig = item[:-1]
    if sig[0] != 0x30 or sig[1] != len(sig) - 2 or sig[2] != 0x02:
        raise ValueError("bad BIP66 DER signature")
    rlen = sig[3]
    if rlen == 0 or rlen + 5 >= len(sig) or sig[4 + rlen] != 0x02:
        raise ValueError("bad signature R")
    slen = sig[5 + rlen]
    if slen == 0 or rlen + slen + 6 != len(sig):
        raise ValueError("bad signature S")
    for begin, length in ((4, rlen), (6 + rlen, slen)):
        if sig[begin] & 0x80 or (
            length > 1 and sig[begin] == 0 and sig[begin + 1] & 0x80 == 0
        ):
            raise ValueError("negative/nonminimal signature integer")
    return sig


def bip143_p2wpkh(fund, spend, pubkey: bytes, sats: int) -> bytes:
    if len(pubkey) != 33 or pubkey[0] not in (2, 3):
        raise ValueError("compressed signer pubkey required")
    program = hash160(pubkey)
    if fund["outputs"][0][0] != sats:
        raise ValueError("spent amount does not match raw funding output")
    if fund["outputs"][0][1] != b"\x00\x14" + program:
        raise ValueError("witness signer does not match parent output")
    if not spend["segwit"] or len(spend["witness"][0]) != 2:
        raise ValueError("non-native-witness sender")
    if spend["inputs"][0][1]:
        raise ValueError("nonempty native scriptSig")
    if spend["inputs"][0][0] != fund["txid"] + struct.pack("<I", 0):
        raise ValueError("first vin does not point to funded output")
    if spend["witness"][0][1] != pubkey:
        raise ValueError("witness pubkey mutation")
    scriptcode = b"\x76\xa9\x14" + program + b"\x88\xac"
    preimage = (
        spend["version"]
        + sha256d(b"".join(vin[0] for vin in spend["inputs"]))
        + sha256d(b"".join(vin[2] for vin in spend["inputs"]))
        + spend["inputs"][0][0]
        + compact(len(scriptcode))
        + scriptcode
        + struct.pack("<Q", sats)
        + spend["inputs"][0][2]
        + sha256d(b"".join(o[2] for o in spend["outputs"]))
        + spend["locktime"]
        + struct.pack("<I", 1)
    )
    return preimage


def public_spki(pubkey: bytes) -> bytes:
    alg = bytes.fromhex("301006072a8648ce3d020106052b8104000a")
    bitstr = b"\x03\x22\x00" + pubkey
    spki = bytes([0x30, len(alg) + len(bitstr)]) + alg + bitstr
    return (b"-----BEGIN PUBLIC KEY-----\n" +
            base64.encodebytes(spki) +
            b"-----END PUBLIC KEY-----\n")


def verify(openssl: str, pubkey: bytes, signature: bytes, digest: bytes) -> bool:
    with tempfile.TemporaryDirectory(prefix="tl-stage25o-") as temp:
        folder = Path(temp)
        (folder / "pub.pem").write_bytes(public_spki(pubkey))
        (folder / "sig.der").write_bytes(signature)
        (folder / "digest.bin").write_bytes(digest)
        completed = subprocess.run(
            [openssl, "pkeyutl", "-verify", "-pubin", "-inkey", str(folder / "pub.pem"),
             "-sigfile", str(folder / "sig.der"), "-in", str(folder / "digest.bin"),
             "-pkeyopt", "digest:sha256"],
            capture_output=True, timeout=15, check=False,
        )
        return completed.returncode == 0


def read_fixture():
    fields = {}
    for line in FIXTURE.read_text(encoding="utf8").splitlines():
        if not line or line.startswith("#"):
            continue
        key, value = line.split("\t")
        if key in fields:
            raise ValueError("duplicate fixture key")
        fields[key] = value
    return fields


def main() -> None:
    if sys.argv[1:] not in ([], ["--check"]):
        raise SystemExit("usage: audit_type9_p2wpkh_openssl.py [--check]")
    openssl = shutil.which("openssl")
    if openssl is None:
        raise SystemExit("FAIL: system OpenSSL unavailable for signature mathematics")
    f = read_fixture()
    if f["network"] != "Bitcoin" or f["consensus_authenticity"] != "synthetic-only":
        raise ValueError("unexpected fixture origin")
    parent = parse_transaction(bytes.fromhex(f["funding_hex"]))
    spend_raw = bytes.fromhex(f["spending_hex"])
    spend = parse_transaction(spend_raw)
    assert parent["txid"].hex() == f["funding_txid_raw"]
    assert spend["txid"].hex() == f["spending_txid_raw"]
    assert spend["wtxid"].hex() == f["spending_wtxid_raw"]
    pubkey = bytes.fromhex(f["pubkey"])
    assert hash160(pubkey).hex() == f["hash160"]
    assert address_from_program(hash160(pubkey)) == f["sender_bech32"]
    assert parent["outputs"][0][1].hex() == f["p2wpkh_script"]
    assert spend["outputs"][1][1] == b"\x6a" + bytes([len(f["payload_ascii"])]) + f["payload_ascii"].encode()
    assert spend["witness"][0][0].hex() == f["signature_der_sighash"]
    preimage = bip143_p2wpkh(parent, spend, pubkey, 20000)
    digest = sha256d(preimage)
    assert preimage.hex() == f["expected_preimage"]
    assert digest.hex() == f["expected_digest"]
    signature = signed_der(spend["witness"][0][0])
    assert verify(openssl, pubkey, signature, digest), "genuine synthetic ECDSA rejected"

    # Witness signature corruption does not change the SegWit txid or digest.
    # This establishes why TXID-Merkle proof cannot authenticate witness data.
    tampered_signature = bytearray(signature)
    tampered_signature[9] ^= 1
    assert not verify(openssl, pubkey, bytes(tampered_signature), digest)
    mutated_witness = spend_raw.replace(
        spend["witness"][0][0],
        bytes(tampered_signature) + b"\x01",
    )
    assert len(mutated_witness) == len(spend_raw) and mutated_witness != spend_raw
    forged = parse_transaction(mutated_witness)
    assert forged["txid"] == spend["txid"]
    assert forged["wtxid"] != spend["wtxid"]
    assert sha256d(bip143_p2wpkh(parent, forged, pubkey, 20000)) == digest

    # Alter output value inside transaction, still parseable; math fails.
    altered = list(spend["outputs"])
    original = altered[0]
    modified = (original[0] + 1, original[1],
                struct.pack("<Q", original[0] + 1) + original[2][8:])
    altered[0] = modified
    changed_outputs = dict(spend, outputs=altered)
    modified_digest = sha256d(bip143_p2wpkh(parent, changed_outputs, pubkey, 20000))
    assert modified_digest != digest and not verify(openssl, pubkey, signature, modified_digest)

    # The BIP143 funding amount is directly signed, unlike legacy P2SH.
    changed_sats = 20001
    changed_funding_digest = sha256d(
        spend["version"] +
        sha256d(b"".join(i[0] for i in spend["inputs"])) +
        sha256d(b"".join(i[2] for i in spend["inputs"])) +
        spend["inputs"][0][0] +
        b"\x19\x76\xa9\x14" + hash160(pubkey) + b"\x88\xac" +
        struct.pack("<Q", changed_sats) +
        spend["inputs"][0][2] +
        sha256d(b"".join(o[2] for o in spend["outputs"])) +
        spend["locktime"] + struct.pack("<I", 1)
    )
    assert changed_funding_digest != digest
    assert not verify(openssl, pubkey, signature, changed_funding_digest)

    # Two real txids are locally merkle-bound to one fabricated PoW header.
    header = bytes.fromhex(f["header_hex"])
    assert len(header) == 80
    assert header[4:36] == bytes.fromhex(f["anchor_hash_display"])[::-1]
    assert header[36:68] == sha256d(parent["txid"] + spend["txid"])
    assert bytes.fromhex(f["funding_merkle_sibling_raw"]) == spend["txid"]
    assert bytes.fromhex(f["spending_merkle_sibling_raw"]) == parent["txid"]
    assert sha256d(header)[::-1].hex() == f["block_hash_display"]
    assert int.from_bytes(sha256d(header), "little") <= 0x7fffff << (8 * (32 - 3))
    print("PASS: exact first-input outpoint, HASH160, Bech32-v0 and BIP143 differential fixture")
    print("PASS: genuine single-signer OpenSSL ECDSA, forgery/output/amount negative tests")
    print("BOUNDARY: fabricated block/UTXO, missing historical admin and channel revision")


if __name__ == "__main__":
    main()
