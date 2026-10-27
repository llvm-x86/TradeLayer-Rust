"""Independent raw-block TXID/Merkle provenance oracle for SQLite candidates."""
from pathlib import Path
import hashlib
import sys
import unittest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
from candidate_block_provenance import (
    InvalidBlock, read_block, check_candidate, tl_payload, sha256d,
)


def fixture_bytes():
    return bytes.fromhex((ROOT / "fixtures/raw_block_ingest_synthetic.hex").read_text().strip())


class ProvenanceTests(unittest.TestCase):
    def test_frozen_block_has_four_full_transactions_and_two_tl_outputs(self):
        source = fixture_bytes()
        blockhash = sha256d(source[:80])[::-1].hex()
        records = read_block(source, expected_hash=blockhash, expected_count=4)
        self.assertEqual(len(records), 4)
        self.assertEqual(source[36:68], self._merkle([bytes.fromhex(x.txid)[::-1] for x in records]))
        observations = []
        for pos, body in enumerate(records):
            for vout, script in enumerate(body.output_scripts):
                payload = tl_payload(script)
                if payload is not None:
                    fmt = "legacy-tl" if payload.startswith(b"tl") else "proposed-TLR"
                    observations.append((pos, vout, fmt))
                    check_candidate(body, pos, body.txid, body.raw, vout, fmt)
        self.assertEqual(len(observations), 2)

    @staticmethod
    def _merkle(ids):
        while len(ids) > 1:
            if len(ids) % 2:
                ids.append(ids[-1])
            ids = [sha256d(ids[i] + ids[i + 1]) for i in range(0, len(ids), 2)]
        return ids[0]

    def test_untouched_block_header_cannot_hide_mutated_transaction_bytes(self):
        source = bytearray(fixture_bytes())
        # Flip a value inside a transaction, preserving the original block header.
        source[81] ^= 1
        with self.assertRaises(InvalidBlock):
            read_block(bytes(source))

    def test_wrong_count_and_trailing_bytes_rejected(self):
        source = fixture_bytes()
        with self.assertRaises(InvalidBlock):
            read_block(source, expected_count=5)
        with self.assertRaises(InvalidBlock):
            read_block(source + b"\x00")

    def test_wrong_tx_identity_or_vout_rejected(self):
        blocks = read_block(fixture_bytes())
        body = next(b for b in blocks if any(tl_payload(s) is not None for s in b.output_scripts))
        vout = next(i for i, script in enumerate(body.output_scripts) if tl_payload(script) is not None)
        fmt = "legacy-tl" if tl_payload(body.output_scripts[vout]).startswith(b"tl") else "proposed-TLR"
        with self.assertRaises(InvalidBlock):
            check_candidate(body, 1, "00" * 32, body.raw, vout, fmt)
        with self.assertRaises(InvalidBlock):
            check_candidate(body, 1, body.txid, body.raw + b"\x00", vout, fmt)
        with self.assertRaises(InvalidBlock):
            check_candidate(body, 1, body.txid, body.raw, len(body.output_scripts), fmt)
        with self.assertRaises(InvalidBlock):
            check_candidate(body, 1, body.txid, body.raw, vout, "other")

    def test_noncanonical_push_and_wrong_marker_rejected(self):
        self.assertIsNone(tl_payload(b"\x6a"))
        self.assertIsNone(tl_payload(b"\x6a\x4c\x02tl"))  # Non-minimal PUSHDATA1
        self.assertIsNone(tl_payload(b"\x6a\x03tlx\x00"))  # Trailing script byte
        self.assertIsNone(tl_payload(b"\x6a\x02xx"))
        self.assertEqual(tl_payload(b"\x6a\x02tl"), b"tl")


if __name__ == "__main__":
    unittest.main()
