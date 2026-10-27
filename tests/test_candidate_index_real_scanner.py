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

            # The original Rust-produced rows are valid. Manipulate one
            # field at a time while leaving both completion markers intact.
            base_lines = scanned.stdout.splitlines(keepends=True)
            first_index = next(i for i, line in enumerate(base_lines) if line.startswith('C\t'))
            original_fields = base_lines[first_index].rstrip('\n').split('\t')
            for field, forged in (
                (4, '0'),                    # Another TX position in the same block
                (5, '00' * 32),            # Forged TXID
                (6, '99999'),              # Vout outside parsed TX
                (7, 'proposed-TLR' if original_fields[7] == 'legacy-tl' else 'legacy-tl'),
                (10, original_fields[10] + '00'),  # Forged raw TX bytes
            ):
                with self.subTest(field=field):
                    modified = original_fields.copy()
                    modified[field] = forged
                    changed_lines = base_lines.copy()
                    changed_lines[first_index] = '\t'.join(modified) + '\n'
                    candidates.write_text(''.join(changed_lines))
                    with self.assertRaisesRegex(store.Incomplete,
                                                'candidate failed block/TX/vout binding'):
                        store.ingest(db_path, 'regtest', blocks, candidates)
                    self.assertEqual(store.stats(db_path), before)

            # Alter a serialized TX version while retaining the original
            # block header/hash. The independent Merkle calculation must fail.
            candidates.write_text(scanned.stdout)
            change = list(block_frames.rstrip('\n').split('\n'))
            columns = change[0].split('\t')
            rawbytes = bytearray.fromhex(columns[3])
            rawbytes[81] ^= 1
            columns[3] = rawbytes.hex()
            change[0] = '\t'.join(columns)
            blocks.write_text('\n'.join(change) + '\n')
            with self.assertRaisesRegex(store.Incomplete, 'Merkle root mismatch'):
                store.ingest(db_path, 'regtest', blocks, candidates)
            self.assertEqual(store.stats(db_path), before)


if __name__ == '__main__':
    unittest.main()
