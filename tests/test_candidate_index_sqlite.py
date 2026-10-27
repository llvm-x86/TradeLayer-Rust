"""Offline adversarial transactions for the read-only SQLite candidate store."""
import hashlib
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts"))
import candidate_index_sqlite as index


def block(height, parent, nonce=1, count=2):
    header = (1).to_bytes(4, "little") + bytes.fromhex(parent)[::-1] + b"\0" * 32
    header += (1700000000).to_bytes(4, "little")
    header += (0x207fffff).to_bytes(4, "little")
    header += nonce.to_bytes(4, "little")
    assert len(header) == 80
    value = hashlib.sha256(hashlib.sha256(header).digest()).digest()[::-1].hex()
    return index.Block(height, value, parent, count), header.hex() + f"{count:02x}" + "00" * 10


def inputs(blocks, rows=(), completion=True):
    b = "".join(f"B\t{item.height}\t{item.hash}\t{raw}\n" for item, raw in blocks)
    b += f"E\tBLOCKS\t{len(blocks)}\n"
    c = "".join(
        f"C\tBLOCK\t{item.height}\t{item.hash}\t{pos}\t{txid}\t{vout}\tTLR\t2\tSyntaxOnly\t{raw}\n"
        for item, pos, vout, txid, raw in rows
    )
    if completion:
        c += f"END\tCOMPLETE_UNVERIFIED\tBLOCKS\t{len(blocks)}\t{sum(item.tx_count for item,_ in blocks)}\t{len(rows)}\n"
    return b, c


class StorageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name)
        self.db = self.dir / "state.sqlite"
        self.anchor = "11" * 32
        index.init(self.db, "regtest", 4, self.anchor)

    def apply(self, blocks, rows=(), completion=True, c_rewrite=None, b_rewrite=None, network="regtest"):
        b, c = inputs(blocks, rows, completion)
        if c_rewrite:
            c = c_rewrite(c)
        if b_rewrite:
            b = b_rewrite(b)
        bf, cf = self.dir / "blocks.tsv", self.dir / "rows.tsv"
        bf.write_text(b)
        cf.write_text(c)
        return index.ingest(self.db, network, bf, cf)

    def test_init_empty_and_status(self):
        state = index.stats(self.db)
        self.assertEqual(state["height"], 4)
        self.assertEqual(state["hash"], self.anchor)
        self.assertEqual(state["trust"], "UNVERIFIED")
        self.assertEqual(state["blocks"], 0)
        with self.assertRaises(index.Incomplete):
            index.init(self.db, "regtest", 4, self.anchor)

    def test_canonical_append_including_empty_carrier_block(self):
        a = block(5, self.anchor)
        b = block(6, a[0].hash)
        txid = "cc" * 32
        outcome = self.apply([a, b], [(a[0], 1, 0, txid, "01020304")])
        self.assertEqual(outcome[:3], (2, 4, 1))
        self.assertEqual(index.stats(self.db)["height"], 6)
        self.assertEqual(index.stats(self.db)["blocks"], 2)
        self.assertEqual(index.stats(self.db)["transactions"], 1)
        self.assertEqual(index.stats(self.db)["carriers"], 1)
        self.apply([a, b], [(a[0], 1, 0, txid, "01020304")])
        self.assertEqual(index.stats(self.db)["carriers"], 1)

    def test_reorg_at_first_block_deletes_all_descendants_and_raw_bytes(self):
        a = block(5, self.anchor)
        b = block(6, a[0].hash)
        self.apply([a,b], [(a[0],1,0,"aa"*32,"01020304"),(b[0],0,1,"bb"*32,"112233")])
        replacement = block(5, self.anchor, nonce=99)
        self.apply([replacement], [(replacement[0],0,0,"ee"*32,"aabbcc")])
        self.assertEqual(index.stats(self.db)["height"], 5)
        self.assertEqual(index.stats(self.db)["blocks"], 1)
        self.assertEqual(index.stats(self.db)["carriers"], 1)
        with index.open_database(self.db) as db:
            self.assertEqual(db.execute("SELECT txid FROM transactions").fetchone()[0], "ee"*32)

    def test_reorg_suffix_rewrites_only_requested_height(self):
        a = block(5, self.anchor)
        b = block(6, a[0].hash)
        self.apply([a,b], [(a[0],0,1,"aa"*32,"0102"),(b[0],0,0,"bb"*32,"1122")])
        other = block(6, a[0].hash, nonce=31)
        self.apply([other], [(other[0],1,0,"cc"*32,"1234")])
        with index.open_database(self.db) as db:
            self.assertEqual(db.execute("SELECT txid FROM transactions ORDER BY height").fetchall(), [("aa"*32,),("cc"*32,)])

    def test_crashed_stream_after_reorg_delete_rolls_back(self):
        a = block(5, self.anchor)
        self.apply([a], [(a[0],0,0,"aa"*32,"010203")])
        before = index.stats(self.db)
        other = block(5, self.anchor, nonce=2)
        with self.assertRaises(index.Incomplete):
            self.apply([other], [(other[0],0,0,"bb"*32,"332211")], completion=False)
        self.assertEqual(index.stats(self.db), before)
        with index.open_database(self.db) as db:
            self.assertEqual(db.execute("SELECT hash FROM blocks WHERE height=5").fetchone()[0], a[0].hash)

    def test_wrong_parent_and_gap_fail_without_mutation(self):
        a = block(7, self.anchor)
        with self.assertRaises(index.Incomplete):
            self.apply([a])
        self.assertEqual(index.stats(self.db)["blocks"], 0)
        a = block(5, "00"*32)
        with self.assertRaises(index.Incomplete):
            self.apply([a])
        self.assertEqual(index.stats(self.db)["blocks"], 0)

    def test_wrong_block_header_and_bad_block_complete(self):
        a = block(5, self.anchor)
        with self.assertRaises(index.Incomplete):
            self.apply([a], b_rewrite=lambda s: s.replace(a[0].hash,"aa"*32))
        with self.assertRaises(index.Incomplete):
            self.apply([a], b_rewrite=lambda s: s.replace("E\tBLOCKS\t1", "E\tBLOCKS\t2"))
        self.assertEqual(index.stats(self.db)["blocks"], 0)

    def test_candidate_wrong_block_reference_and_count_fail(self):
        a = block(5, self.anchor)
        row = (a[0], 0, 0, "bb"*32, "0102")
        before = index.stats(self.db)
        with self.assertRaises(index.Incomplete):
            self.apply([a], [row], c_rewrite=lambda s: s.replace(a[0].hash, "00"*32))
        with self.assertRaises(index.Incomplete):
            self.apply([a], [row], c_rewrite=lambda s: s.replace("BLOCKS\t1\t2\t1", "BLOCKS\t1\t2\t2"))
        self.assertEqual(index.stats(self.db), before)

    def test_duplicate_out_of_order_and_conflicting_raw_tx(self):
        a = block(5, self.anchor)
        rows = [(a[0],0,0,"bb"*32,"0102"),(a[0],0,0,"bb"*32,"0102")]
        with self.assertRaises(index.Incomplete):
            self.apply([a], rows)
        rows = [(a[0],0,0,"bb"*32,"0102"),(a[0],0,1,"bb"*32,"ffff")]
        with self.assertRaises(index.Incomplete):
            self.apply([a], rows)
        self.assertEqual(index.stats(self.db)["carriers"], 0)

    def test_multiple_vouts_share_one_transaction_body(self):
        a = block(5, self.anchor)
        rows = [(a[0],0,0,"bb"*32,"0102"),(a[0],0,1,"bb"*32,"0102")]
        self.apply([a], rows)
        self.assertEqual(index.stats(self.db)["transactions"], 1)
        self.assertEqual(index.stats(self.db)["carriers"], 2)

    def test_unsupported_script_and_trailing_data(self):
        a = block(5, self.anchor)
        with self.assertRaises(index.Incomplete):
            self.apply([a], c_rewrite=lambda s: "C\tSCRIPT\t0\n" + s)
        with self.assertRaises(index.Incomplete):
            self.apply([a], c_rewrite=lambda s: s + "C\tBLOCK\tanything\n")
        self.assertEqual(index.stats(self.db)["blocks"], 0)

    def test_wrong_namespace(self):
        a = block(5, self.anchor)
        with self.assertRaises(index.Incomplete):
            self.apply([a], network="main")
        self.assertEqual(index.stats(self.db)["blocks"], 0)

    def test_anchor_audit_detects_mutated_parent_without_rewriting_data(self):
        a = block(5, self.anchor)
        b = block(6, a[0].hash)
        self.apply([a,b])
        count, height, tip = index.audit(self.db)
        self.assertEqual((count, height, tip), (2, 6, b[0].hash))
        with index.open_database(self.db) as db:
            db.execute("UPDATE blocks SET parent=? WHERE height=6", ("22"*32,))
        with self.assertRaises(index.Incomplete):
            index.audit(self.db)

    def test_pre_genesis_anchor_rules(self):
        db = self.dir / "genesis.sqlite"
        with self.assertRaises(index.Incomplete):
            index.init(db, "regtest", -1, "12"*32)
        index.init(db, "regtest", -1, "00"*32)
        a = block(0, "00"*32)
        b,c = inputs([a])
        (self.dir / "blocks.tsv").write_text(b)
        (self.dir / "rows.tsv").write_text(c)
        index.ingest(db, "regtest", self.dir / "blocks.tsv", self.dir / "rows.tsv")
        self.assertEqual(index.stats(db)["height"], 0)


if __name__ == "__main__":
    unittest.main()
