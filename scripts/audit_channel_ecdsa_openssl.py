#!/usr/bin/env python3
"""Stage25J: independently verify TWO genuine secp256k1 ECDSA signatures.

Offline, public-fixture-only, stdlib hashlib/struct and system OpenSSL CLI.
This is a diagnostic test harness, not a consensus verifier or a Rust
authorization API. No wallet/key generation, nodes, network or new crates.

Fixture: fixtures/channel_p2wsh_ecdsa_signed_synthetic.tsv.
Both DER signatures are valid ECDSA for the BIP143 digest. Altering a
signature, output, selected amount or signer makes verification fail.
"""

from __future__ import annotations

import base64
import hashlib
import shutil
import struct
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "fixtures" / "channel_p2wsh_ecdsa_signed_synthetic.tsv"
MAX_RAW = 1_000_000
MAX_ITEMS = 100_000


def dsha(data: bytes) -> bytes:
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def compact(size: int) -> bytes:
    if size < 0:
        raise ValueError("negative size")
    if size < 253:
        return bytes([size])
    if size <= 0xFFFF:
        return b"\xfd" + struct.pack("<H", size)
    if size <= 0xFFFFFFFF:
        return b"\xfe" + struct.pack("<I", size)
    return b"\xff" + struct.pack("<Q", size)


def load_fixture() -> dict[str, str]:
    values = {}
    for line in FIXTURE.read_text(encoding="utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        key, val = line.split("\t")
        if key in values:
            raise ValueError("duplicate fixture key")
        values[key] = val
    for key in (
        "network", "funding_vout", "spending_vin", "funding_hex",
        "spending_hex", "pubkey_a", "pubkey_b", "witness_script",
        "expected_bip143_digest", "expected_prevouts",
        "expected_sequence", "expected_outputs", "funding_txid_raw",
        "spending_txid_raw", "spending_wtxid_raw",
    ):
        if key not in values:
            raise ValueError("missing fixture " + key)
    if values["network"] != "Litecoin":
        raise ValueError("unexpected source network")
    return values


class Cursor:
    def __init__(self, data: bytes):
        if len(data) > MAX_RAW:
            raise ValueError("oversized raw transaction")
        self.data = data
        self.offset = 0

    def take(self, n: int) -> bytes:
        if n < 0 or self.offset + n > len(self.data):
            raise ValueError("truncated transaction")
        result = self.data[self.offset:self.offset + n]
        self.offset += n
        return result

    def integer(self) -> int:
        code = self.take(1)[0]
        if code < 253:
            return code
        if code == 253:
            value = struct.unpack("<H", self.take(2))[0]
            if value < 253:
                raise ValueError("noncanonical CompactSize")
            return value
        if code == 254:
            value = struct.unpack("<I", self.take(4))[0]
            if value <= 0xFFFF:
                raise ValueError("noncanonical CompactSize")
            return value
        value = struct.unpack("<Q", self.take(8))[0]
        if value <= 0xFFFFFFFF:
            raise ValueError("noncanonical CompactSize")
        return value

    def count(self, nonzero: bool = False) -> int:
        value = self.integer()
        if value > MAX_ITEMS or (nonzero and value == 0):
            raise ValueError("invalid transaction count")
        return value

    def blob(self) -> bytes:
        size = self.integer()
        if size > MAX_RAW:
            raise ValueError("oversized transaction blob")
        return self.take(size)


@dataclass(frozen=True)
class Input:
    outpoint: bytes
    script_sig: bytes
    sequence: bytes


@dataclass(frozen=True)
class Output:
    value: int
    script_pubkey: bytes
    serial: bytes


@dataclass(frozen=True)
class Tx:
    version: bytes
    inputs: tuple[Input, ...]
    outputs: tuple[Output, ...]
    witnesses: tuple[tuple[bytes, ...], ...]
    locktime: bytes
    txid: bytes
    wtxid: bytes
    segwit: bool


def parse_tx(raw: bytes) -> Tx:
    cur = Cursor(raw)
    version = cur.take(4)
    segwit = raw[4:5] == b"\x00"
    if segwit and cur.take(2) != b"\x00\x01":
        raise ValueError("unsupported segwit marker/flag")
    inputs_start = cur.offset
    inputs = []
    for _ in range(cur.count(nonzero=True)):
        outpoint = cur.take(36)
        script_sig = cur.blob()
        sequence = cur.take(4)
        inputs.append(Input(outpoint, script_sig, sequence))
    outputs = []
    for _ in range(cur.count(nonzero=True)):
        start = cur.offset
        value = struct.unpack("<Q", cur.take(8))[0]
        script_pubkey = cur.blob()
        outputs.append(Output(value, script_pubkey, raw[start:cur.offset]))
    outputs_end = cur.offset
    witnesses = []
    if segwit:
        for _ in inputs:
            witness = tuple(cur.blob() for _ in range(cur.count()))
            witnesses.append(witness)
        if not any(item for witness in witnesses for item in witness):
            raise ValueError("superfluous witness marker")
    locktime = cur.take(4)
    if cur.offset != len(raw):
        raise ValueError("trailing transaction data")
    stripped = version + raw[inputs_start:outputs_end] + locktime if segwit else raw
    return Tx(
        version, tuple(inputs), tuple(outputs), tuple(witnesses),
        locktime, dsha(stripped), dsha(raw), segwit,
    )


def valid_2of2_script(script: bytes, signer_a: bytes, signer_b: bytes) -> None:
    if len(script) != 71 or script[0:2] != b"\x52\x21":
        raise ValueError("noncanonical 2-of-2 script")
    if script[35:36] != b"\x21" or script[-2:] != b"\x52\xae":
        raise ValueError("noncanonical CHECKMULTISIG template")
    if signer_a == signer_b or any(len(pk) != 33 or pk[0] not in (2, 3)
                                   for pk in (signer_a, signer_b)):
        raise ValueError("invalid signer identities")
    if not ((script[2:35], script[36:69]) == (signer_a, signer_b)
            or (script[2:35], script[36:69]) == (signer_b, signer_a)):
        raise ValueError("witness script pubkeys do not match fixture")


def b66_signed_der(item: bytes) -> bytes:
    if len(item) < 9 or len(item) > 73 or item[-1] != 1:
        raise ValueError("unsupported sighash or DER length")
    der = item[:-1]
    if der[0] != 0x30 or der[1] != len(der) - 2 or der[2] != 0x02:
        raise ValueError("invalid DER")
    r_len = der[3]
    if not r_len or r_len + 5 >= len(der) or der[4 + r_len] != 0x02:
        raise ValueError("invalid DER R")
    s_len = der[5 + r_len]
    if not s_len or r_len + s_len + 6 != len(der):
        raise ValueError("invalid DER S")
    for start, n in ((4, r_len), (6 + r_len, s_len)):
        if der[start] & 0x80 or (n > 1 and der[start] == 0 and not der[start + 1] & 0x80):
            raise ValueError("nonminimal or negative DER integer")
    return der


def bip143(funding: Tx, spend: Tx, vout: int, vin: int, pub_a: bytes, pub_b: bytes):
    if not 0 <= vout < len(funding.outputs) or not 0 <= vin < len(spend.inputs):
        raise ValueError("index out of bounds")
    output = funding.outputs[vout]
    src = spend.inputs[vin]
    if src.outpoint != funding.txid + struct.pack("<I", vout):
        raise ValueError("spender does not reference actual funding output")
    if not spend.segwit or src.script_sig:
        raise ValueError("native P2WSH requires SegWit and empty scriptSig")
    witness = spend.witnesses[vin]
    if len(witness) != 4 or witness[0] != b"":
        raise ValueError("incorrect multisig witness shape")
    script = witness[3]
    valid_2of2_script(script, pub_a, pub_b)
    if output.value == 0:
        raise ValueError("zero value funding output")
    if output.script_pubkey != b"\x00\x20" + hashlib.sha256(script).digest():
        raise ValueError("P2WSH script does not match committed output")
    sigs = [b66_signed_der(witness[1]), b66_signed_der(witness[2])]
    h_prev = dsha(b"".join(inp.outpoint for inp in spend.inputs))
    h_seq = dsha(b"".join(inp.sequence for inp in spend.inputs))
    h_out = dsha(b"".join(out.serial for out in spend.outputs))
    preimage = (
        spend.version + h_prev + h_seq + src.outpoint + compact(len(script))
        + script + struct.pack("<Q", output.value) + src.sequence
        + h_out + spend.locktime + struct.pack("<I", 1)
    )
    return dsha(preimage), (h_prev, h_seq, h_out), sigs


def ec_spki_pem(compressed_pubkey: bytes) -> bytes:
    """DER SubjectPublicKeyInfo secp256k1 EC key, checked by OpenSSL itself."""
    if len(compressed_pubkey) != 33 or compressed_pubkey[0] not in (2, 3):
        raise ValueError("wrong compressed public key")
    # id-ecPublicKey (1.2.840.10045.2.1), secp256k1 (1.3.132.0.10).
    alg = bytes.fromhex("301006072a8648ce3d020106052b8104000a")
    bitstr = b"\x03\x22\x00" + compressed_pubkey
    spki = bytes([0x30, len(alg) + len(bitstr)]) + alg + bitstr
    return (b"-----BEGIN PUBLIC KEY-----\n" +
            base64.encodebytes(spki) +
            b"-----END PUBLIC KEY-----\n")


def verify_signature(openssl: str, pubkey: bytes, signature_der: bytes, digest: bytes) -> bool:
    """Only OpenSSL verifies mathematical ECDSA. Never fake a pass on errors."""
    with tempfile.TemporaryDirectory(prefix="tl-stage25j-") as directory:
        root = Path(directory)
        (root / "key.pem").write_bytes(ec_spki_pem(pubkey))
        (root / "signature.der").write_bytes(signature_der)
        (root / "digest.bin").write_bytes(digest)
        proc = subprocess.run(
            [openssl, "pkeyutl", "-verify", "-pubin",
             "-inkey", str(root / "key.pem"),
             "-sigfile", str(root / "signature.der"),
             "-in", str(root / "digest.bin"),
             "-pkeyopt", "digest:sha256"],
            capture_output=True, timeout=15, check=False,
        )
        return proc.returncode == 0


def main() -> None:
    if sys.argv[1:] not in ([], ["--check"]):
        raise SystemExit("usage: audit_channel_ecdsa_openssl.py [--check]")
    openssl = shutil.which("openssl")
    if openssl is None:
        raise SystemExit("FAIL: preinstalled OpenSSL CLI unavailable; cannot verify signatures")
    fixture = load_fixture()
    fund = parse_tx(bytes.fromhex(fixture["funding_hex"]))
    spend = parse_tx(bytes.fromhex(fixture["spending_hex"]))
    assert fund.txid.hex() == fixture["funding_txid_raw"]
    assert spend.txid.hex() == fixture["spending_txid_raw"]
    assert spend.wtxid.hex() == fixture["spending_wtxid_raw"]
    pub_a = bytes.fromhex(fixture["pubkey_a"])
    pub_b = bytes.fromhex(fixture["pubkey_b"])
    vout = int(fixture["funding_vout"])
    vin = int(fixture["spending_vin"])
    digest, components, sigs = bip143(fund, spend, vout, vin, pub_a, pub_b)
    assert spend.witnesses[vin][3].hex() == fixture["witness_script"]
    assert digest.hex() == fixture["expected_bip143_digest"]
    assert [digest.hex() for digest in components] == [
        fixture["expected_prevouts"],
        fixture["expected_sequence"],
        fixture["expected_outputs"],
    ]
    assert verify_signature(openssl, pub_a, sigs[0], digest), "signer A signature invalid"
    assert verify_signature(openssl, pub_b, sigs[1], digest), "signer B signature invalid"

    # Cross-signer, duplicate-signer and 1-byte forgery must fail independent
    # math verification (even when DER remains syntactically well-formed).
    assert not verify_signature(openssl, pub_a, sigs[1], digest)
    assert not verify_signature(openssl, pub_b, sigs[0], digest)
    assert not verify_signature(openssl, pub_b, sigs[0], digest)
    tampered = bytearray(sigs[0])
    tampered[5] ^= 1  # R value, preserve DER sequence boundaries
    assert not verify_signature(openssl, pub_a, bytes(tampered), digest)

    # SIGHASH_ALL must cryptographically bind output amount, all input
    # sequences, the funded value, and the other input outpoint.
    new_output = list(spend.outputs)
    original = new_output[0].serial
    changed = struct.pack("<Q", spend.outputs[0].value + 1) + original[8:]
    new_output[0] = Output(spend.outputs[0].value + 1, spend.outputs[0].script_pubkey, changed)
    modified = Tx(spend.version, spend.inputs, tuple(new_output), spend.witnesses,
                  spend.locktime, spend.txid, spend.wtxid, spend.segwit)
    changed_digest, _, _ = bip143(fund, modified, vout, vin, pub_a, pub_b)
    assert changed_digest != digest
    assert not verify_signature(openssl, pub_a, sigs[0], changed_digest)
    assert not verify_signature(openssl, pub_b, sigs[1], changed_digest)

    new_funding_output = Output(
        fund.outputs[vout].value + 1, fund.outputs[vout].script_pubkey,
        struct.pack("<Q", fund.outputs[vout].value + 1) + fund.outputs[vout].serial[8:],
    )
    altered_funding = Tx(
        fund.version, fund.inputs, (new_funding_output,),
        fund.witnesses, fund.locktime, dsha(fund.version + b"\x01" +
        b"".join(inp.outpoint + compact(len(inp.script_sig)) + inp.script_sig + inp.sequence
                 for inp in fund.inputs) + b"\x01" + new_funding_output.serial +
        fund.locktime), fund.wtxid, fund.segwit,
    )
    altered_inputs = list(spend.inputs)
    altered_inputs[vin] = Input(altered_funding.txid + struct.pack("<I", vout),
                                spend.inputs[vin].script_sig, spend.inputs[vin].sequence)
    changed_spend = Tx(spend.version, tuple(altered_inputs), spend.outputs,
                       spend.witnesses, spend.locktime, spend.txid, spend.wtxid, True)
    modified_digest, _, _ = bip143(
        altered_funding, changed_spend, vout, vin, pub_a, pub_b,
    )
    assert modified_digest != digest
    assert not verify_signature(openssl, pub_a, sigs[0], modified_digest)
    assert not verify_signature(openssl, pub_b, sigs[1], modified_digest)
    print("PASS: independent OpenSSL secp256k1 ECDSA verifies BOTH synthetic signers")
    print("PASS: swapped/forged signatures and mutated output/funding amount reject")
    print("BOUNDARY: no proven UTXO, inclusion, revocation, channel state or economic authority")


if __name__ == "__main__":
    main()
