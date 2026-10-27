"""Independent bounded Bitcoin transaction-to-block provenance for candidate storage.

Research-grade structural parsing only. No proof of PoW, scripts, UTXO state,
active chain, witness commitment or TradeLayer transaction authorization.
Source is independent of the existing Rust scanner.
"""
from dataclasses import dataclass
import hashlib


class InvalidBlock(ValueError):
    pass


MAX_TX_BYTES = 1_000_000
MAX_BLOCK_BYTES = 4_000_000
MAX_IO_COUNT = 100_000
MAX_BLOCK_TX_COUNT = 1_000_000


def sha256d(blob):
    return hashlib.sha256(hashlib.sha256(blob).digest()).digest()


class Reader:
    def __init__(self, raw):
        self.raw = memoryview(raw)
        self.pos = 0

    def take(self, n):
        if n < 0 or n > len(self.raw) - self.pos:
            raise InvalidBlock("truncated transaction or excessive script/witness length")
        previous = self.pos
        self.pos += n
        return self.raw[previous:self.pos]

    def compact(self):
        tag = self.take(1)[0]
        if tag < 253:
            return tag
        width = {253: 2, 254: 4, 255: 8}[tag]
        n = int.from_bytes(self.take(width), "little")
        if n < 253 or (tag == 254 and n <= 65535) or (
            tag == 255 and n <= 2**32 - 1
        ):
            raise InvalidBlock("noncanonical CompactSize")
        return n

    def vector(self):
        n = self.compact()
        if n > MAX_TX_BYTES:
            raise InvalidBlock("oversized script or witness element")
        return self.take(n)


@dataclass(frozen=True)
class TxBody:
    txid: str
    raw: bytes
    output_scripts: tuple


def read_transaction(reader):
    beginning = reader.pos
    version = reader.take(4)
    has_witness = False
    if reader.pos < len(reader.raw) and reader.raw[reader.pos] == 0:
        if reader.pos + 1 >= len(reader.raw) or reader.raw[reader.pos + 1] != 1:
            raise InvalidBlock("invalid SegWit marker/flag")
        reader.take(2)
        has_witness = True

    vin_start = reader.pos
    n_inputs = reader.compact()
    if not 1 <= n_inputs <= MAX_IO_COUNT:
        raise InvalidBlock("invalid transaction input count")
    for _ in range(n_inputs):
        reader.take(36)
        reader.vector()
        reader.take(4)

    n_outputs = reader.compact()
    if not 1 <= n_outputs <= MAX_IO_COUNT:
        raise InvalidBlock("invalid transaction output count")
    scripts = []
    for _ in range(n_outputs):
        reader.take(8)
        scripts.append(bytes(reader.vector()))

    stripped_end = reader.pos
    if has_witness:
        any_nonempty = False
        for _ in range(n_inputs):
            n_items = reader.compact()
            if n_items > MAX_IO_COUNT:
                raise InvalidBlock("excessive witness stack")
            if n_items:
                any_nonempty = True
            for _ in range(n_items):
                reader.vector()
        if not any_nonempty:
            raise InvalidBlock("superfluous empty witness marker")
    locktime = reader.take(4)
    if reader.pos - beginning > MAX_TX_BYTES:
        raise InvalidBlock("transaction exceeds research size limit")

    tx = bytes(reader.raw[beginning:reader.pos])
    if has_witness:
        serialized_without_witness = (
            bytes(version) + bytes(reader.raw[vin_start:stripped_end]) + bytes(locktime)
        )
        digest = sha256d(serialized_without_witness)
    else:
        digest = sha256d(tx)
    return TxBody(digest[::-1].hex(), tx, tuple(scripts)), digest


def read_block(raw, *, expected_hash=None, expected_count=None):
    if not 81 <= len(raw) <= MAX_BLOCK_BYTES:
        raise InvalidBlock("raw block outside supported byte bounds")
    header = raw[:80]
    actual_hash = sha256d(header)[::-1].hex()
    if expected_hash is not None and actual_hash != expected_hash:
        raise InvalidBlock("block header hash mismatch")
    reader = Reader(raw)
    reader.take(80)
    count = reader.compact()
    if count == 0 or count > MAX_BLOCK_TX_COUNT:
        raise InvalidBlock("invalid block transaction count")
    if expected_count is not None and count != expected_count:
        raise InvalidBlock("raw transaction count mismatches announced block count")
    bodies = []
    digests = []
    for _ in range(count):
        body, digest = read_transaction(reader)
        bodies.append(body)
        digests.append(digest)
    if reader.pos != len(raw):
        raise InvalidBlock("trailing bytes beyond declared transactions")
    level = digests
    while len(level) > 1:
        if len(level) % 2:
            level = level + [level[-1]]
        level = [sha256d(level[i] + level[i + 1]) for i in range(0, len(level), 2)]
    if level[0] != header[36:68]:
        raise InvalidBlock("Bitcoin transaction Merkle root mismatch")
    return bodies


def tl_payload(script):
    """Mirror Rust's canonical one-push OP_RETURN envelope, not TL validity."""
    if len(script) < 3 or script[0] != 0x6A:
        return None
    tag = script[1]
    if 1 <= tag <= 75:
        length, offset = tag, 2
    elif tag == 0x4C and len(script) >= 3:
        length, offset = script[2], 3
        if length <= 75:
            return None
    elif tag == 0x4D and len(script) >= 4:
        length, offset = int.from_bytes(script[2:4], "little"), 4
        if length <= 255:
            return None
    elif tag == 0x4E and len(script) >= 6:
        length, offset = int.from_bytes(script[2:6], "little"), 6
        if length <= 65535:
            return None
    else:
        return None
    if len(script) != offset + length:
        return None
    payload = script[offset:]
    return payload if payload.startswith((b"tl", b"TLR")) else None


def check_candidate(body, pos, txid, raw, vout, fmt):
    if txid != body.txid or raw != body.raw:
        raise InvalidBlock(f"candidate transaction mismatches raw block position {pos}")
    if vout >= len(body.output_scripts):
        raise InvalidBlock("candidate output index beyond transaction vout")
    payload = tl_payload(body.output_scripts[vout])
    if payload is None:
        raise InvalidBlock("candidate output lacks a canonical TL OP_RETURN push")
    expected_fmt = "legacy-tl" if payload.startswith(b"tl") else "proposed-TLR"
    if fmt != expected_fmt:
        raise InvalidBlock("candidate TL format mismatches its source output")
