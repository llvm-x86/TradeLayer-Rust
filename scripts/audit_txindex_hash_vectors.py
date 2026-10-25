#!/usr/bin/env python3
"""Independent stdlib SHA256d oracle for synthetic legacy/SegWit preindex.

This fixture does NOT authenticate Bitcoin consensus, chain ancestry, script
execution, witness signatures, previous outputs or TradeLayer admission.
"""
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "fixtures" / "txindex_segwit_hash_vectors.tsv"


def sha256d(value):
    return hashlib.sha256(hashlib.sha256(value).digest()).digest().hex()


def main():
    count = 0
    for raw_line in FIXTURE.read_text(encoding="utf-8").splitlines():
        if not raw_line or raw_line.startswith("#") or raw_line.startswith("label\t"):
            continue
        label, raw_hex, stripped_hex, txid_raw, wtxid_raw, witness, carriers = raw_line.split("\t")
        raw = bytes.fromhex(raw_hex)
        stripped = bytes.fromhex(stripped_hex)
        has_witness = witness == "1"
        assert raw[:4] == stripped[:4], label
        assert raw[-4:] == stripped[-4:], label
        assert (raw[4:6] == b"\x00\x01") == has_witness, label
        assert (stripped == raw) == (not has_witness), label
        assert sha256d(stripped) == txid_raw, label
        assert sha256d(raw) == wtxid_raw, label
        assert (txid_raw == wtxid_raw) == (not has_witness), label
        if has_witness:
            assert raw.startswith(stripped[:4] + b"\x00\x01"), label
            assert int(carriers) in (0, 1), label
        count += 1
    assert count == 3
    print("PASS: 3 independent hashlib legacy/SegWit txid/wtxid digests")
    print("PASS: legacy txid==wtxid; witness changes wtxid but not stripped txid")
    print("BOUNDARY: fabricated UTXOs, no active-chain/Script/consensus authentication")


if __name__ == "__main__":
    main()
