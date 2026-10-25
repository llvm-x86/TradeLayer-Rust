"""Offline CI for read-only Bitcoin Core bulk/per-TX benchmark harness.

Uses independently frozen Stage8C transaction digests and a fake RPC node.
No network call, cookie, wallet or real Bitcoin node is needed in CI.
"""
from __future__ import annotations

import copy
import hashlib
from pathlib import Path
import tempfile
import unittest

from scripts import bench_bitcoin_core_rpc_ingest as bench

ROOT = Path(__file__).resolve().parents[1]
RAW = bytes.fromhex((ROOT / "fixtures/raw_block_ingest_synthetic.hex").read_text().strip())

# Independent SHA256d values from scripts/audit_raw_block_ingest_vectors.py,
# stored in the native Bitcoin/Litecoin little-endian hash byte order.
KNOWN_TXIDS_RAW = [
    "8647c745029df431acac0ff4f8a91ff00d6194ef87a705946f784af9f213bffa",
    "49f3b25cef04ddad2b71d1dbfa5cf1b5b2f0bc39ae73eaa395de79e0a17c4af3",
    "f878d360c2a1b812ef5aa5b2706f0f56aa34dba00b4f40fe07000571f00208b1",
    "4f61a8c818de93316dc5b69b57d8fdae42c774056908109397d8381e7e8dcfac",
]
KNOWN_TXIDS = [bytes.fromhex(x)[::-1].hex() for x in KNOWN_TXIDS_RAW]
BLOCK_HASH = hashlib.sha256(hashlib.sha256(RAW[:80]).digest()).digest()[::-1].hex()
HEIGHT = 850_000


class FakeBitcoinCore:
    def __init__(self):
        self.calls = []
        self.network = "main"
        self.ibd = False
        self.txids = KNOWN_TXIDS[:]
        self.block_hex = RAW.hex()
        self.tx_hex = [tx[0].hex() for tx in bench.parse_raw_block(RAW)[1]]
        self.confirmations = 1
        self.actual_hash = BLOCK_HASH
        self.height = HEIGHT
        self.version = 290100

    def __call__(self, method, *args):
        self.calls.append((method, args))
        if method == "getblockchaininfo":
            return {
                "chain": self.network, "initialblockdownload": self.ibd,
                "blocks": 900000, "headers": 900000,
            }
        if method == "getnetworkinfo":
            return {"version": self.version}
        if method == "getblockhash":
            assert args == (HEIGHT,)
            return self.actual_hash
        if method == "getblock":
            assert args[0] == BLOCK_HASH
            if args[1] == 1:
                return {
                    "hash": BLOCK_HASH, "height": self.height,
                    "confirmations": self.confirmations, "tx": self.txids,
                }
            assert args[1] == 0
            return self.block_hex
        if method == "getrawtransaction":
            txid, verbose, blockhash = args
            assert verbose is False and blockhash == BLOCK_HASH
            return self.tx_hex[self.txids.index(txid)]
        raise AssertionError("unexpected method: " + method)


class CoreRpcBenchmarkTests(unittest.TestCase):
    def test_frozen_raw_block_and_independent_sha256d_vectors(self):
        blockhash, transactions = bench.parse_raw_block(RAW)
        self.assertEqual(blockhash, BLOCK_HASH)
        self.assertEqual([tx[1] for tx in transactions], KNOWN_TXIDS)
        self.assertEqual(len(transactions), 4)
        self.assertEqual([tx[1] == tx[2] for tx in transactions],
                         [True, True, False, False])
        self.assertEqual(bench.hash_display(bench.merkle_root(KNOWN_TXIDS)),
                         RAW[36:68][::-1].hex())
        self.assertEqual(len(RAW), 370)

    def test_fake_same_node_per_tx_vs_bulk_byte_equality(self):
        rpc = FakeBitcoinCore()
        report = bench.benchmark(rpc, height=HEIGHT, repeats=2, warmups=1)
        self.assertEqual(report["status"], "bitcoin_core_rpc_ingestion_benchmark_unverified")
        self.assertEqual(report["transaction_count"], 4)
        self.assertEqual(report["segwit_transaction_count"], 2)
        self.assertEqual(report["raw_block_bytes"], 370)
        self.assertTrue(report["individual_byte_equal_to_bulk"])
        self.assertTrue(report["preflight_makes_cache_cold_claim_invalid"])
        self.assertEqual(report["benchmark_repeats"], 2)
        self.assertEqual(report["benchmark_warmups"], 1)
        self.assertEqual(report["individual_rpc_calls_per_sample"], 4)
        self.assertEqual(report["bulk_rpc_calls_per_sample"], 1)
        self.assertEqual(report["bitcoin_script_utxo_consensus"], "UNVERIFIED")
        self.assertEqual(report["tradelayer_state"], "UNVERIFIED")
        # Five metadata/preflight RPCs, three bulk reads and 12 individual.
        self.assertEqual(len(rpc.calls), 20)
        self.assertEqual(sum(method == "getrawtransaction" for method, _ in rpc.calls), 12)
        self.assertEqual(sum(method == "getblock" for method, _ in rpc.calls), 5)
        self.assertEqual(sum(method == "getblockhash" for method, _ in rpc.calls), 1)

    def test_aggregate_transaction_reorder_fails_local_merkle(self):
        original_hash, txs = bench.parse_raw_block(RAW)
        bad = RAW[:81] + txs[0][0] + txs[2][0] + txs[1][0] + txs[3][0]
        self.assertNotEqual(bad, RAW)
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "Merkle root differs"):
            bench.parse_raw_block(bad)
        self.assertEqual(original_hash, BLOCK_HASH)

    def test_rpc_wrong_individual_tx_with_unchanged_block_fails(self):
        rpc = FakeBitcoinCore()
        tx = bytearray.fromhex(rpc.tx_hex[2])
        tx[-5] ^= 0x04
        rpc.tx_hex[2] = tx.hex()
        with self.assertRaisesRegex(bench.IngestBenchmarkError,
                                    "individual transaction 2 differs"):
            bench.benchmark(rpc, height=HEIGHT, repeats=1)

    def test_rpc_changed_bulk_response_fails(self):
        rpc = FakeBitcoinCore()
        class ModifiedBulk:
            def __init__(self, wrapped):
                self.wrapped = wrapped
                self.bulk_calls = 0
            def __call__(self, method, *args):
                result = self.wrapped(method, *args)
                if method == "getblock" and args[-1] == 0:
                    self.bulk_calls += 1
                    if self.bulk_calls == 2:
                        return result[:-2] + "01"
                return result
        with self.assertRaisesRegex(bench.IngestBenchmarkError,
                                    "bulk response bytes changed"):
            bench.benchmark(ModifiedBulk(rpc), height=HEIGHT, repeats=1)

    def test_wrong_bitcoin_chain_and_initial_download_rejected(self):
        for mode in ("test", "regtest", "signet"):
            rpc = FakeBitcoinCore()
            rpc.network = mode
            with self.assertRaisesRegex(bench.IngestBenchmarkError, "synced Bitcoin mainnet"):
                bench.benchmark(rpc, height=HEIGHT, repeats=1)
        rpc = FakeBitcoinCore()
        rpc.ibd = True
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "synced Bitcoin mainnet"):
            bench.benchmark(rpc, height=HEIGHT, repeats=1)

    def test_wrong_rpc_height_and_bad_blockhash_rejected(self):
        rpc = FakeBitcoinCore()
        rpc.height -= 1
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "metadata"):
            bench.benchmark(rpc, height=HEIGHT, repeats=1)
        rpc = FakeBitcoinCore()
        rpc.actual_hash = "ff" * 32
        with self.assertRaises(AssertionError):
            bench.benchmark(rpc, height=HEIGHT, repeats=1)

    def test_rpc_source_txid_forgery_and_duplicate_rejected(self):
        rpc = FakeBitcoinCore()
        rpc.txids[2] = "ff" * 32
        with self.assertRaisesRegex(bench.IngestBenchmarkError,
                                    "verbose transaction IDs disagree"):
            bench.benchmark(rpc, height=HEIGHT, repeats=1)
        rpc = FakeBitcoinCore()
        rpc.txids[1] = rpc.txids[0]
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "duplicate verbose"):
            bench.benchmark(rpc, height=HEIGHT, repeats=1)

    def test_reject_noncanonical_compactsize_transaction_count(self):
        # Four TXs encoded as fd 04 00 is nonminimal under Bitcoin Core.
        modified = RAW[:80] + b"\xfd\x04\x00" + RAW[81:]
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "noncanonical CompactSize"):
            bench.parse_raw_block(modified)
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "trailing"):
            bench.parse_raw_block(RAW + b"\x00")

    def test_segwit_witness_mutation_preserves_txid_only(self):
        _, entries = bench.parse_raw_block(RAW)
        witness_tx = bytearray(entries[2][0])
        witness_tx[-5] ^= 0x01
        read = bench.Reader(bytes(witness_tx))
        actual, txid, wtxid = bench.read_tx(read)
        self.assertEqual(actual, bytes(witness_tx))
        self.assertEqual(txid, entries[2][1])
        self.assertNotEqual(wtxid, entries[2][2])

    def test_invalid_benchmark_configuration(self):
        rpc = FakeBitcoinCore()
        for kw in (
            {"height": True}, {"height": -1}, {"height": "850000"},
            {"height": HEIGHT, "repeats": 0},
            {"height": HEIGHT, "repeats": 8},
            {"height": HEIGHT, "warmups": -1},
            {"height": HEIGHT, "warmups": 4},
            {"height": HEIGHT, "max_transactions": True},
            {"height": HEIGHT, "max_transactions": 0},
        ):
            with self.subTest(kw=kw), self.assertRaises(bench.IngestBenchmarkError):
                bench.benchmark(rpc, **kw)
        self.assertEqual(rpc.calls, [])

    def test_rpc_data_size_and_malformed_values_fail_closed(self):
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "bounded raw hexadecimal"):
            bench.raw_hex("gg", bench.MAX_BLOCK_BYTES, "bad")
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "bounded raw hexadecimal"):
            bench.raw_hex("aa" * (bench.MAX_BLOCK_BYTES + 1), bench.MAX_BLOCK_BYTES, "too big")
        with self.assertRaises(bench.IngestBenchmarkError):
            bench.parse_raw_block(bytearray(RAW))
        with self.assertRaises(bench.IngestBenchmarkError):
            bench.parse_raw_block(b"")

    def test_loopback_client_rejects_write_rpc_before_transport(self):
        with tempfile.TemporaryDirectory() as tmp:
            cookie = Path(tmp) / ".cookie"
            cookie.write_text("__cookie__:this-is-a-fake-test-token")
            with bench.BitcoinCoreRpc(cookie) as client:
                with self.assertRaisesRegex(bench.IngestBenchmarkError, "non-read-only"):
                    client("sendrawtransaction", "00")
                with self.assertRaisesRegex(bench.IngestBenchmarkError, "non-read-only"):
                    client("createwallet", "unsafe")
                self.assertEqual(client.calls, 0)
            with self.assertRaisesRegex(bench.IngestBenchmarkError, "cookie"):
                bad = Path(tmp) / "bad"
                bad.write_text("user:plain-secret")
                bench.BitcoinCoreRpc(bad)

    def test_merkle_mutation_and_source_input_bounds(self):
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "equal real siblings"):
            bench.merkle_root([KNOWN_TXIDS[0], KNOWN_TXIDS[0]])
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "Merkle leaf"):
            bench.merkle_root(["not a txid"])
        with self.assertRaisesRegex(bench.IngestBenchmarkError, "truncated"):
            bench.read_tx(bench.Reader(bytes.fromhex("01000000")))
        self.assertEqual(len(bench.merkle_root(KNOWN_TXIDS)), 32)


if __name__ == "__main__":
    unittest.main()
