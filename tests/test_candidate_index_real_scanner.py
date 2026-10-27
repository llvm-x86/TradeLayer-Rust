"""Full offline Rust scanner -> SQLite candidate evidence integration."""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'scripts'))
import candidate_index_sqlite as store


class EndToEnd(unittest.TestCase):
    def test_real_frozen_block_fixture(self):
        rawhex = (ROOT / 'fixtures/raw_block_ingest_synthetic.hex').read_text().strip()
        raw = bytes.fromhex(rawhex)
        block_hash = hashlib.sha256(hashlib.sha256(raw[:80]).digest()).digest()[::-1].hex()
        parent_hash = raw[4:36][::-1].hex()
        block_frames = f'B\t10\t{block_hash}\t{rawhex}\nE\tBLOCKS\t1\n'
        binary = Path(os.environ.get('TRADELAYER_SCANNER_BIN', str(ROOT / 'target/debug/tradelayer-core-index-ingest')))
        if not binary.is_file():
            self.fail(f'build the pinned Rust scanner before integration test: {binary}')
        scanned = subprocess.run([str(binary)], input=block_frames, text=True,
                                 capture_output=True, check=True)
        self.assertIn('END\tCOMPLETE_UNVERIFIED\tBLOCKS\t1\t4\t2', scanned.stdout)
        self.assertIn('source=UNVERIFIED', scanned.stderr)
        with tempfile.TemporaryDirectory() as temp:
            p = Path(temp)
            db_path = p / 'evidence.sqlite'
            blocks = p / 'blocks.tsv'
            candidates = p / 'candidates.tsv'
            blocks.write_text(block_frames)
            candidates.write_text(scanned.stdout)
            store.init(db_path, 'regtest', 9, parent_hash)
            result = store.ingest(db_path, 'regtest', blocks, candidates)
            self.assertEqual(result[:3], (1, 4, 2))
            self.assertEqual(store.stats(db_path)['carriers'], 2)
            with store.open_database(db_path) as db:
                stored = db.execute('SELECT txid,raw FROM transactions ORDER BY tx_position').fetchall()
                self.assertEqual(len(stored), 2)
                self.assertTrue(all(len(txid) == 64 and raw for txid, raw in stored))
            # The same fixture is an idempotent replay, even after process restart.
            store.ingest(db_path, 'regtest', blocks, candidates)
            self.assertEqual(store.stats(db_path)['carriers'], 2)
            before = store.stats(db_path)
            candidates.write_text(scanned.stdout.rsplit('END\t', 1)[0])
            with self.assertRaises(store.Incomplete):
                store.ingest(db_path, 'regtest', blocks, candidates)
            self.assertEqual(store.stats(db_path), before)


if __name__ == '__main__':
    unittest.main()
