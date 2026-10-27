"""Black-box tests for the standalone Rust immutable candidate segment store.

The first fixtures are synthetic Bitcoin-looking headers with explicit
UNVERIFIED classification. The integration test uses the repository's actual
Rust transaction scanner and frozen raw block test fixture.
"""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
BINARY = Path(os.environ.get("TLCAND_RUST_STORE_BIN", "/tmp/tradelayer-candidate-kv-std"))
SCANNER = Path(os.environ.get("TLCAND_RUST_SCANNER_BIN", str(ROOT / "target/debug/tradelayer-core-index-ingest")))
ANCHOR = "11" * 32


def run(*argv, env=None):
    if not BINARY.is_file():
        raise AssertionError(f"compile candidate store before running: {BINARY}")
    return subprocess.run([str(BINARY), *map(str, argv)], text=True,
                          capture_output=True, env=env)


def block(height, parent, nonce, txcount=2):
    header = (
        (1).to_bytes(4, "little")
        + bytes.fromhex(parent)[::-1]
        + bytes(32)
        + (1700000000).to_bytes(4, "little")
        + (0x207fffff).to_bytes(4, "little")
        + nonce.to_bytes(4, "little")
    )
    assert len(header) == 80
    digest = hashlib.sha256(hashlib.sha256(header).digest()).digest()[::-1].hex()
    return dict(height=height, hash=digest, parent=parent,
                txcount=txcount, raw=header.hex()+f"{txcount:02x}"+"00"*8)


def frame_text(blocks, rows=(), complete=True):
    b = "".join(
        f"B\t{x['height']}\t{x['hash']}\t{x['raw']}\n" for x in blocks
    ) + f"E\tBLOCKS\t{len(blocks)}\n"
    c = "".join(
        f"C\tBLOCK\t{x['height']}\t{x['hash']}\t{pos}\t{txid}\t{vout}\tlegacy-tl\t2\ttyped-syntax\t{raw}\n"
        for x, pos, vout, txid, raw in rows
    )
    if complete:
        c += f"END\tCOMPLETE_UNVERIFIED\tBLOCKS\t{len(blocks)}\t{sum(x['txcount'] for x in blocks)}\t{len(rows)}\n"
    return b, c


class StoreTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.data = self.root / "store"
        r = run("init", "--dir", self.data, "--network", "regtest",
                "--anchor-height", "4", "--anchor-hash", ANCHOR)
        self.assertEqual(r.returncode, 0, r.stderr)

    def operate(self, blocks, rows=(), complete=True, modify_c=None, modify_b=None,
                network="regtest", env=None):
        b, c = frame_text(blocks, rows, complete)
        if modify_b is not None:
            b = modify_b(b)
        if modify_c is not None:
            c = modify_c(c)
        bs = self.root / "blocks.tsv"
        cs = self.root / "carriers.tsv"
        bs.write_text(b)
        cs.write_text(c)
        return run("ingest", "--dir", self.data, "--network", network,
                   "--blocks", bs, "--candidates", cs, env=env)

    def test_empty_store_and_explicit_init_cannot_overwrite(self):
        r = run("verify", "--dir", self.data)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("blocks=0", r.stdout)
        self.assertIn("trust=UNVERIFIED", r.stdout)
        r = run("init", "--dir", self.data, "--network", "regtest",
                "--anchor-height", "4", "--anchor-hash", ANCHOR)
        self.assertNotEqual(r.returncode, 0)

    def test_append_empty_candidate_block_query_and_idempotent_replay(self):
        a = block(5, ANCHOR, 1)
        b = block(6, a["hash"], 2)
        rows = [(a, 0, 0, "bb"*32, "010203")]
        r = self.operate([a,b], rows)
        self.assertEqual(r.returncode, 0, r.stderr)
        s = run("verify", "--dir", self.data)
        self.assertEqual(s.returncode, 0, s.stderr)
        self.assertIn("blocks=2", s.stdout)
        self.assertIn("height=6", s.stdout)
        self.assertIn("segment_integrity=CHECKED", s.stdout)
        a_rows = run("get", "--dir", self.data, "--height", "5")
        self.assertEqual(a_rows.returncode, 0, a_rows.stderr)
        self.assertIn("C\tBLOCK\t5\t", a_rows.stdout)
        b_rows = run("get", "--dir", self.data, "--height", "6")
        self.assertEqual(b_rows.returncode, 0, b_rows.stderr)
        self.assertNotIn("C\tBLOCK\t", b_rows.stdout)
        self.assertIn("END\tUNVERIFIED\t6", b_rows.stdout)
        seg_before = sorted(p.name for p in (self.data / "segments").iterdir())
        self.assertEqual(len(seg_before), 2)
        r = self.operate([a,b], rows)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(seg_before, sorted(p.name for p in (self.data / "segments").iterdir()))

    def test_reorg_rollback_orphan_retention_and_wrong_parent(self):
        a = block(5, ANCHOR, 3)
        b = block(6, a["hash"], 4)
        old = [(a, 0, 0, "aa"*32, "0123"),(b,0,1,"bb"*32,"2345")]
        self.assertEqual(self.operate([a,b], old).returncode, 0)
        new_b = block(6, a["hash"], 99)
        r = self.operate([new_b], [(new_b,1,0,"cc"*32,"abcd")])
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(new_b["hash"], run("status", "--dir", self.data).stdout)
        self.assertIn("cc"*32, run("get", "--dir", self.data, "--height", "6").stdout)
        self.assertNotIn("bb"*32, run("get", "--dir", self.data, "--height", "6").stdout)
        self.assertEqual(run("verify", "--dir", self.data).returncode, 0)
        bad = block(7, "00"*32, 123)
        denied = self.operate([bad])
        self.assertNotEqual(denied.returncode, 0)
        self.assertIn(new_b["hash"], run("status", "--dir", self.data).stdout)
        # Old immutable fork segments remain as recoverable orphans, without a live manifest reference.
        self.assertEqual(len(list((self.data / "segments").iterdir())), 3)

    def test_missing_marker_after_fork_replace_keeps_previous_generation(self):
        a = block(5, ANCHOR, 5)
        self.assertEqual(self.operate([a],[(a,0,0,"aa"*32,"0102")]).returncode,0)
        before = (self.data / "CURRENT").read_bytes()
        replacement = block(5, ANCHOR, 6)
        denied = self.operate([replacement], [(replacement,0,0,"bb"*32,"1234")],
                              complete=False)
        self.assertNotEqual(denied.returncode, 0)
        self.assertEqual((self.data / "CURRENT").read_bytes(), before)
        self.assertIn("aa"*32, run("get","--dir",self.data,"--height","5").stdout)

    def test_conflicting_candidate_raw_and_order_are_denied(self):
        a = block(5, ANCHOR, 7)
        rows = [(a,0,0,"aa"*32,"0102"),(a,0,0,"aa"*32,"0102")]
        self.assertNotEqual(self.operate([a],rows).returncode,0)
        rows = [(a,0,0,"aa"*32,"0102"),(a,0,1,"aa"*32,"0103")]
        self.assertNotEqual(self.operate([a],rows).returncode,0)
        self.assertIn("blocks=0", run("status","--dir",self.data).stdout)

    def test_wrong_header_gap_namespace_and_completion_counts(self):
        a = block(5, ANCHOR, 8)
        self.assertNotEqual(self.operate([a],network="main").returncode,0)
        self.assertNotEqual(self.operate([a],modify_b=lambda s:s.replace(a["hash"],"ff"*32)).returncode,0)
        gap = block(8, ANCHOR, 9)
        self.assertNotEqual(self.operate([gap]).returncode,0)
        self.assertNotEqual(self.operate([a],modify_c=lambda s:s.replace("BLOCKS\t1\t2\t0","BLOCKS\t1\t2\t1")).returncode,0)
        self.assertIn("blocks=0",run("status","--dir",self.data).stdout)

    def test_crash_before_manifest_recovery_and_lock_fail_closed(self):
        a = block(5, ANCHOR, 10)
        self.assertEqual(self.operate([a]).returncode,0)
        old = (self.data / "CURRENT").read_bytes()
        b = block(6, a["hash"], 11)
        injected = dict(os.environ, TLCAND_TEST_FAILPOINT="after_segments")
        r = self.operate([b],env=injected)
        self.assertNotEqual(r.returncode,0)
        self.assertEqual((self.data / "CURRENT").read_bytes(), old)
        self.assertIn("blocks=1",run("verify","--dir",self.data).stdout)
        denied = self.operate([b])
        self.assertNotEqual(denied.returncode,0)
        self.assertIn("LOCK",denied.stderr)
        # Operator inspects process termination and removes stale lock explicitly.
        (self.data / "LOCK").unlink()
        recovered = self.operate([b])
        self.assertEqual(recovered.returncode,0,recovered.stderr)
        self.assertIn("blocks=2",run("verify","--dir",self.data).stdout)

    def test_corrupt_segment_fails_integrity_checks(self):
        a = block(5, ANCHOR, 12)
        self.assertEqual(self.operate([a]).returncode,0)
        path = next((self.data / "segments").iterdir())
        path.write_bytes(path.read_bytes()+b"CORRUPT")
        self.assertNotEqual(run("verify","--dir",self.data).returncode,0)
        self.assertNotEqual(run("get","--dir",self.data,"--height","5").returncode,0)

    def test_live_rust_scanner_fixture_roundtrip(self):
        if not SCANNER.is_file():
            self.fail(f"compile Rust scanner before test: {SCANNER}")
        rawhex = (ROOT / "fixtures/raw_block_ingest_synthetic.hex").read_text().strip()
        raw = bytes.fromhex(rawhex)
        blockhash = hashlib.sha256(hashlib.sha256(raw[:80]).digest()).digest()[::-1].hex()
        parent = raw[4:36][::-1].hex()
        original = self.root / "real"
        result = run("init","--dir",original,"--network","regtest",
                     "--anchor-height","9","--anchor-hash",parent)
        self.assertEqual(result.returncode,0,result.stderr)
        bf = self.root / "real_blocks.tsv"
        cf = self.root / "real_carriers.tsv"
        bf.write_text(f"B\t10\t{blockhash}\t{rawhex}\nE\tBLOCKS\t1\n")
        scanner = subprocess.run([str(SCANNER)],input=bf.read_text(),text=True,
                                 capture_output=True,check=True)
        self.assertIn("END\tCOMPLETE_UNVERIFIED\tBLOCKS\t1\t4\t2",scanner.stdout)
        cf.write_text(scanner.stdout)
        receipt = run("ingest","--dir",original,"--network","regtest",
                      "--blocks",bf,"--candidates",cf)
        self.assertEqual(receipt.returncode,0,receipt.stderr)
        self.assertIn("carriers=2",receipt.stdout)
        self.assertEqual(run("verify","--dir",original).returncode,0)
        self.assertIn("C\tBLOCK\t10\t",run("get","--dir",original,"--height","10").stdout)


if __name__ == "__main__":
    unittest.main()
