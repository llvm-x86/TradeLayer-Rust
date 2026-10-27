"""A -> B -> A fork replacement using the actual Stage8C Rust scanner."""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import candidate_index_sqlite as store


class ForkTests(unittest.TestCase):
    def test_reorg_a_b_a_keeps_exact_current_fork_candidates(self):
        original = bytes.fromhex(
            (ROOT / "fixtures/raw_block_ingest_synthetic.hex").read_text().strip()
        )
        parent = original[4:36]
        scanner = Path(os.environ.get(
            "TRADELAYER_SCANNER_BIN",
            str(ROOT / "target/debug/tradelayer-core-index-ingest")
        ))

        def fork_header(parent_digest, nonce):
            header = bytearray(original[:80])
            header[4:36] = parent_digest
            header[76:80] = nonce.to_bytes(4, "little")
            block = bytes(header) + original[80:]
            digest = hashlib.sha256(hashlib.sha256(header).digest()).digest()
            return block, digest, digest[::-1].hex()

        block_a, hash_a_raw, hash_a = fork_header(parent, 1)
        block_a_child, _, hash_a_child = fork_header(hash_a_raw, 2)
        block_b, _, hash_b = fork_header(parent, 19)

        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            db, blocks, candidates = (
                root / "index.sqlite", root / "blocks.tsv", root / "candidates.tsv"
            )
            store.init(db, "regtest", 9, parent[::-1].hex())

            def commit(sequence):
                frames = "".join(
                    f"B\t{height}\t{display}\t{raw.hex()}\n"
                    for height, display, raw in sequence
                ) + f"E\tBLOCKS\t{len(sequence)}\n"
                completed = subprocess.run(
                    [str(scanner)], input=frames, text=True,
                    check=True, capture_output=True, timeout=30,
                )
                blocks.write_text(frames)
                candidates.write_text(completed.stdout)
                store.ingest(db, "regtest", blocks, candidates)

            commit([(10, hash_a, block_a), (11, hash_a_child, block_a_child)])
            self.assertEqual(store.stats(db)["carriers"], 4)
            self.assertEqual(store.audit(db), (2, 11, hash_a_child))

            commit([(10, hash_b, block_b)])
            self.assertEqual(store.stats(db)["carriers"], 2)
            self.assertEqual(store.audit(db), (1, 10, hash_b))
            with store.open_database(db) as connection:
                self.assertEqual(
                    connection.execute("SELECT hash FROM blocks").fetchall(),
                    [(hash_b,)],
                )
                self.assertEqual(
                    connection.execute("SELECT COUNT(*) FROM transactions").fetchone(),
                    (2,),
                )

            commit([(10, hash_a, block_a), (11, hash_a_child, block_a_child)])
            self.assertEqual(store.stats(db)["carriers"], 4)
            self.assertEqual(store.audit(db), (2, 11, hash_a_child))
            with store.open_database(db) as connection:
                self.assertEqual(
                    connection.execute(
                        "SELECT hash FROM blocks ORDER BY height"
                    ).fetchall(),
                    [(hash_a,), (hash_a_child,)],
                )


if __name__ == "__main__":
    unittest.main()
