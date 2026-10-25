#!/usr/bin/env python3
"""Stage25L public-only legacy P2SH SIGHASH_ALL differential verifier.

Use Python stdlib for serialized transaction/legacy digest, system OpenSSL
pkeyutl for real secp256k1 verification. Never grants channel authority.
"""

import base64
import hashlib
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
from dataclasses import dataclass

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / 'fixtures' / 'channel_p2sh_ecdsa_signed_synthetic.tsv'


def hash256(data):
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()


def compact(value):
    if value < 0:
        raise ValueError('negative CompactSize')
    if value < 253:
        return bytes([value])
    if value <= 65535:
        return b'\xfd' + struct.pack('<H', value)
    if value <= 0xffffffff:
        return b'\xfe' + struct.pack('<I', value)
    return b'\xff' + struct.pack('<Q', value)


class Cursor:
    def __init__(self, data):
        if len(data) > 1_000_000:
            raise ValueError('oversized transaction')
        self.data = data
        self.offset = 0

    def take(self, size):
        if size < 0 or self.offset + size > len(self.data):
            raise ValueError('truncated transaction')
        out = self.data[self.offset:self.offset + size]
        self.offset += size
        return out

    def count(self, require_nonzero=False):
        tag = self.take(1)[0]
        if tag < 253:
            value = tag
        elif tag == 253:
            value = struct.unpack('<H', self.take(2))[0]
            if value < 253:
                raise ValueError('noncanonical CompactSize')
        elif tag == 254:
            value = struct.unpack('<I', self.take(4))[0]
            if value <= 65535:
                raise ValueError('noncanonical CompactSize')
        else:
            value = struct.unpack('<Q', self.take(8))[0]
            if value <= 0xffffffff:
                raise ValueError('noncanonical CompactSize')
        if value > 100_000 or (require_nonzero and value == 0):
            raise ValueError('invalid vector count')
        return value

    def blob(self):
        return self.take(self.count())


@dataclass(frozen=True)
class Input:
    prevout: bytes
    script: bytes
    sequence: bytes
    prevout_offset: int
    sequence_offset: int


@dataclass(frozen=True)
class Output:
    value: int
    script: bytes
    offset: int


@dataclass(frozen=True)
class Tx:
    version: bytes
    inputs: tuple[Input, ...]
    outputs: tuple[Output, ...]
    locktime: bytes
    txid: bytes


def parse_tx(raw):
    c = Cursor(raw)
    version = c.take(4)
    ins = []
    for _ in range(c.count(True)):
        offset = c.offset
        prevout = c.take(36)
        script = c.blob()
        seq_offset = c.offset
        sequence = c.take(4)
        ins.append(Input(prevout, script, sequence, offset, seq_offset))
    outs = []
    for _ in range(c.count(True)):
        offset = c.offset
        value = struct.unpack('<Q', c.take(8))[0]
        outs.append(Output(value, c.blob(), offset))
    locktime = c.take(4)
    if c.offset != len(raw):
        raise ValueError('trailing bytes or unsupported witness')
    return Tx(version, tuple(ins), tuple(outs), locktime, hash256(raw))


def serialized(inputs, outputs, version, locktime):
    return (version + compact(len(inputs)) +
            b''.join(i.prevout + compact(len(i.script)) + i.script + i.sequence
                     for i in inputs) + compact(len(outputs)) +
            b''.join(struct.pack('<Q', o.value) + compact(len(o.script)) + o.script
                     for o in outputs) + locktime)


def der_only(item):
    if len(item) < 9 or len(item) > 73 or item[-1] != 1:
        raise ValueError('invalid sighash type/DER length')
    der = item[:-1]
    if der[0] != 0x30 or der[1] != len(der) - 2 or der[2] != 2:
        raise ValueError('invalid BIP66 DER')
    rlen = der[3]
    if rlen == 0 or 5 + rlen >= len(der) or der[4 + rlen] != 2:
        raise ValueError('invalid R field')
    slen = der[5 + rlen]
    if slen == 0 or rlen + slen + 6 != len(der):
        raise ValueError('invalid S field')
    for start, size in ((4, rlen), (6 + rlen, slen)):
        if der[start] & 0x80 or (size > 1 and der[start] == 0 and not der[start + 1] & 0x80):
            raise ValueError('nonminimal negative DER integer')
    return der


def parse_sigscript(script):
    c = Cursor(script)
    if c.take(1) != b'\x00':
        raise ValueError('CHECKMULTISIG dummy must be OP_0')
    sigs = []
    for _ in range(2):
        size = c.take(1)[0]
        if not 9 <= size <= 73:
            raise ValueError('noncanonical direct signature push')
        sigs.append(der_only(c.take(size)))
    if c.take(1) != b'\x47':
        raise ValueError('noncanonical redeem-script push')
    redeem = c.take(71)
    if c.offset != len(script):
        raise ValueError('trailing scriptSig')
    if redeem[:2] != b'\x52\x21' or redeem[35:36] != b'\x21' or redeem[-2:] != b'\x52\xae':
        raise ValueError('unexpected 2-of-2 redeemScript')
    keys = (redeem[2:35], redeem[36:69])
    if keys[0] == keys[1] or any(len(key) != 33 or key[0] not in (2, 3) for key in keys):
        raise ValueError('bad script signer keys')
    return redeem, keys, sigs


def legacy_digest(funding, spending, vout, vin, expected_keys):
    if not 0 <= vout < len(funding.outputs) or not 0 <= vin < len(spending.inputs):
        raise ValueError('out-of-range vin/vout')
    source = funding.outputs[vout]
    target = spending.inputs[vin]
    if target.prevout != funding.txid + struct.pack('<I', vout):
        raise ValueError('funding outpoint mismatch')
    redeem, keys, sigs = parse_sigscript(target.script)
    if set(keys) != set(expected_keys) or len(set(expected_keys)) != 2:
        raise ValueError('unexpected signer identity')
    hash160 = hashlib.new('ripemd160', hashlib.sha256(redeem).digest()).digest()
    if source.value <= 0 or source.script != b'\xa9\x14' + hash160 + b'\x87':
        raise ValueError('invalid P2SH funding commitment')
    replacements = [Input(i.prevout, redeem if k == vin else b'', i.sequence,
                          i.prevout_offset, i.sequence_offset)
                    for k, i in enumerate(spending.inputs)]
    # Canonical 71-byte script contains no executable OP_CODESEPARATOR and
    # cannot contain a pushed signature to FindAndDelete.
    preimage = serialized(replacements, spending.outputs, spending.version,
                          spending.locktime) + struct.pack('<I', 1)
    return hash256(preimage), preimage, keys, sigs


def verify(openssl, key, der, digest):
    algorithm = bytes.fromhex('301006072a8648ce3d020106052b8104000a')
    spki = bytes([0x30, len(algorithm) + 36]) + algorithm + b'\x03\x22\x00' + key
    pem = b'-----BEGIN PUBLIC KEY-----\n' + base64.encodebytes(spki) + b'-----END PUBLIC KEY-----\n'
    with tempfile.TemporaryDirectory(prefix='tl-stage25l-') as temp:
        root = Path(temp)
        (root / 'public.pem').write_bytes(pem)
        (root / 'der.sig').write_bytes(der)
        (root / 'digest.bin').write_bytes(digest)
        call = subprocess.run([openssl, 'pkeyutl', '-verify', '-pubin', '-inkey',
                               str(root / 'public.pem'), '-sigfile', str(root / 'der.sig'),
                               '-in', str(root / 'digest.bin'), '-pkeyopt', 'digest:sha256'],
                              capture_output=True, check=False, timeout=15)
        return call.returncode == 0


def load():
    fields = {}
    for line in FIXTURE.read_text(encoding='utf-8').splitlines():
        if not line or line.startswith('#'):
            continue
        name, data = line.split('\t')
        if name in fields:
            raise ValueError('duplicate fixture field')
        fields[name] = data
    expected = ('funding_hex', 'spending_hex', 'funding_vout', 'spending_vin',
                'pubkey_a', 'pubkey_b', 'redeem_script', 'signature_a',
                'signature_b', 'script_hash160', 'expected_legacy_digest',
                'expected_legacy_preimage', 'funding_txid_raw', 'spending_txid_raw')
    if any(field not in fields for field in expected) or fields.get('network') != 'Litecoin':
        raise ValueError('missing or unexpected fixture field')
    return fields


def main():
    openssl = shutil.which('openssl')
    if not openssl:
        raise RuntimeError('system OpenSSL unavailable: verification fails closed')
    f = load()
    fund_raw = bytes.fromhex(f['funding_hex'])
    spend_raw = bytes.fromhex(f['spending_hex'])
    fund, spend = parse_tx(fund_raw), parse_tx(spend_raw)
    assert fund.txid.hex() == f['funding_txid_raw']
    assert spend.txid.hex() == f['spending_txid_raw']
    expected_keys = (bytes.fromhex(f['pubkey_a']), bytes.fromhex(f['pubkey_b']))
    vout, vin = int(f['funding_vout']), int(f['spending_vin'])
    digest, preimage, keys, sigs = legacy_digest(fund, spend, vout, vin, expected_keys)
    assert digest.hex() == f['expected_legacy_digest']
    assert preimage.hex() == f['expected_legacy_preimage']
    assert bytes.fromhex(f['redeem_script']) == parse_sigscript(spend.inputs[vin].script)[0]
    assert hashlib.new('ripemd160', hashlib.sha256(bytes.fromhex(f['redeem_script'])).digest()).hexdigest() == f['script_hash160']
    assert sigs == [bytes.fromhex(f['signature_a'])[:-1], bytes.fromhex(f['signature_b'])[:-1]]
    assert all(verify(openssl, key, der, digest) for key, der in zip(keys, sigs))
    assert not verify(openssl, keys[0], sigs[1], digest)
    assert not verify(openssl, keys[1], sigs[0], digest)
    assert not verify(openssl, keys[1], sigs[0], digest)
    altered_sig = bytearray(sigs[0])
    altered_sig[9] ^= 1
    assert not verify(openssl, keys[0], bytes(altered_sig), digest)

    # Signatures are excluded from the legacy digest, but INCLUDED in txid.
    first_sig = bytes.fromhex(f['signature_a'])
    changed = spend_raw.replace(first_sig, first_sig[:9] + bytes([first_sig[9] ^ 1]) + first_sig[10:])
    assert len(changed) == len(spend_raw) and changed != spend_raw
    forged = parse_tx(changed)
    assert forged.txid != spend.txid
    forged_digest, _, _, _ = legacy_digest(fund, forged, vout, vin, expected_keys)
    assert forged_digest == digest

    for offset in (spend.outputs[0].offset, spend.inputs[0].prevout_offset,
                   spend.inputs[0].sequence_offset, 0, len(spend_raw) - 1):
        changed = bytearray(spend_raw)
        changed[offset] ^= 1
        d, _, _, _ = legacy_digest(fund, parse_tx(bytes(changed)), vout, vin, expected_keys)
        assert d != digest
        assert all(not verify(openssl, key, sig, d) for key, sig in zip(keys, sigs))

    # The changed funding amount is committed INDIRECTLY by its TXID.
    altered_funding = bytearray(fund_raw)
    altered_funding[fund.outputs[vout].offset] ^= 1
    changed_fund = parse_tx(bytes(altered_funding))
    try:
        legacy_digest(changed_fund, spend, vout, vin, expected_keys)
        raise AssertionError('stale funding outpoint accepted')
    except ValueError as error:
        assert 'outpoint' in str(error)
    updated_spend = bytearray(spend_raw)
    src_offset = spend.inputs[vin].prevout_offset
    updated_spend[src_offset:src_offset + 32] = changed_fund.txid
    changed_digest, _, _, _ = legacy_digest(
        changed_fund, parse_tx(bytes(updated_spend)), vout, vin, expected_keys)
    assert changed_digest != digest
    assert all(not verify(openssl, key, sig, changed_digest) for key, sig in zip(keys, sigs))

    print('PASS: historical legacy SIGHASH_ALL SHA256d and public-only P2SH fixture')
    print('PASS: OpenSSL validates both signers, rejects swaps, forgery and spent-output changes')
    print('BOUNDARY: no chain inclusion, spent status, TradeLayer state revision or settlement')


if __name__ == '__main__':
    import sys
    if sys.argv[1:] not in ([], ['--check']):
        raise SystemExit('usage: audit_p2sh_legacy_ecdsa_openssl.py [--check]')
    main()
