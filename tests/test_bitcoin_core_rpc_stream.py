"""Offline contract tests for the proposed Bitcoin Core RPC streaming bridge."""
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

PATH = Path(__file__).resolve().parents[1] / "scripts" / "bitcoin_core_rpc_stream.py"
spec = importlib.util.spec_from_file_location("bitcoin_core_rpc_stream", PATH)
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)

A = "a" * 64
B = "b" * 64
C = "c" * 64
TX = "d" * 64


class FakeRPC:
    def __init__(self, *, tip=20, reorg=False, limit=False, pruned=False):
        self.tip = tip
        self.reorg = reorg
        self.limit = limit
        self.pruned = pruned
        self.calls = []
        self.rechecks = 0

    def call(self, method, *params):
        self.calls.append((method, list(params)))
        if method == "getblockchaininfo":
            return {"chain": "regtest", "blocks": self.tip}
        if method == "getblockhash":
            return self._hash(params[0])
        if method == "getscripthistory":
            options = params[2]
            left, right = options["start_height"], options["end_height"]
            if self.limit and right > left:
                raise bridge.RPCError(-1, "Script history index lookup limit exceeded")
            if self.pruned:
                return {"transactions": [], "pruned_blocks": [A]}
            return {"transactions": [{"txid": TX, "hex": "0200", "blockhash": A,
                                       "vout": [{"n": 0, "spent": False}]}]}
        raise AssertionError(method)

    def _hash(self, height):
        self.rechecks += 1
        if self.reorg and self.rechecks > 2:
            return B
        return A if height == 0 else B

    def batch(self, requests):
        self.calls.extend(requests)
        if all(method == "getblockhash" for method, _ in requests):
            self.rechecks += 1
            if self.reorg and self.rechecks >= 3:
                return [C for _ in requests]
            return [A if params[0] == 0 else B for _, params in requests]
        if all(method == "getblock" for method, _ in requests):
            return ["00" * 100 for _ in requests]
        raise AssertionError(requests)


class BridgeTests(unittest.TestCase):
    def test_bulk_rpc_one_block_request_per_height(self):
        fake = FakeRPC(tip=2)
        out = io.StringIO()
        count = bridge.stream_blocks(fake, "regtest", 0, 2, 2, out)
        self.assertEqual(count, 3)
        self.assertEqual(out.getvalue().count("\n"), 4)
        self.assertTrue(out.getvalue().endswith("E\tBLOCKS\t3\n"))
        block_calls = [params for method, params in fake.calls if method == "getblock"]
        self.assertEqual(len(block_calls), 3)
        self.assertTrue(all(params[1] == 0 for params in block_calls))

    def test_reorg_fails_before_emitting_affected_batch(self):
        out = io.StringIO()
        with self.assertRaisesRegex(ValueError, "active chain changed"):
            bridge.stream_blocks(FakeRPC(tip=2, reorg=True), "regtest", 0, 1, 2, out)
        self.assertEqual(out.getvalue(), "")

    def test_bounded_script_lookup_bisects_exactly(self):
        fake = FakeRPC(tip=5, limit=True)
        out = io.StringIO()
        self.assertEqual(bridge.stream_script(fake, "regtest", "addr(...)", 1, 3,
                                              [0, 2], out), 3)
        self.assertEqual(out.getvalue().count("\n"), 4)
        self.assertIn("E\tSCRIPT\t3\n", out.getvalue())
        lookups = [params for method, params in fake.calls if method == "getscripthistory"]
        self.assertEqual([ (p[2]["start_height"], p[2]["end_height"]) for p in lookups],
                         [(1, 3), (1, 2), (1, 1), (2, 2), (3, 3)])
        self.assertTrue(all(p[2]["include_mempool"] is False and
                            p[2]["include_spending_transactions"] is False for p in lookups))

    def test_pruned_blocks_fail_without_end_sentinel(self):
        out = io.StringIO()
        with self.assertRaises(bridge.IncompleteHistory):
            bridge.stream_script(FakeRPC(pruned=True), "regtest", A, 0, 0, None, out)
        self.assertEqual(out.getvalue(), "")

    def test_single_height_budget_is_hard_failure(self):
        class AlwaysLimit(FakeRPC):
            def call(self, method, *params):
                if method == "getscripthistory":
                    raise bridge.RPCError(-1, "Script history exceeds the 52428800 byte budget")
                return super().call(method, *params)
        with self.assertRaises(bridge.IncompleteHistory):
            list(bridge.fetch_script_pages(AlwaysLimit(), A, 4, 4))

    def test_rejects_invalid_history_vout_order(self):
        with self.assertRaisesRegex(ValueError, "duplicate or unsorted"):
            list(bridge.history_rows([{"txid": TX, "hex": "0200", "blockhash": A,
                                       "vout": [{"n": 1}, {"n": 1}]}]))

    def test_wrong_network_fails_before_fetch(self):
        with self.assertRaisesRegex(ValueError, "wrong Bitcoin network"):
            bridge.stream_blocks(FakeRPC(), "main", 0, 0, 1, io.StringIO())

    def test_cookie_loopback_constraint(self):
        with tempfile.TemporaryDirectory() as directory:
            cookie = Path(directory) / ".cookie"
            cookie.write_text("__cookie__:example_secret", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "loopback"):
                bridge.BitcoinCoreRPC("http://remote.example:8332", str(cookie))
            rpc = bridge.BitcoinCoreRPC("http://127.0.0.1:8332", str(cookie))
            self.assertIn("Basic ", rpc.headers["Authorization"])
            self.assertNotIn("example_secret", str(rpc.headers))
            rpc.connection.close()

    def test_http_500_rpc_error_is_parsed_for_budget_bisection(self):
        class Response:
            status = 500
            def read(self, limit):
                return (b'[{' + b'"id":1,"result":null,"error":{' +
                        b'"code":-1,"message":"Script history index lookup limit exceeded"}}]')
        class Connection:
            def request(self, *args, **kwargs):
                pass
            def getresponse(self):
                return Response()
        with tempfile.TemporaryDirectory() as directory:
            cookie = Path(directory) / ".cookie"
            cookie.write_text("u:p", encoding="utf-8")
            rpc = bridge.BitcoinCoreRPC("http://127.0.0.1:8332", str(cookie))
            rpc.connection = Connection()
            with self.assertRaises(bridge.RPCError) as caught:
                rpc.call("getscripthistory", A, 0, {"start_height": 0, "end_height": 1})
            self.assertTrue(bridge.budget_failure(caught.exception))

    def test_batch_reorders_by_request_id_and_rejects_duplicate(self):
        class Response:
            status = 200
            def __init__(self, body):
                self.body = body
            def read(self, limit):
                return self.body
        class Connection:
            def __init__(self, content):
                self.content = content
            def request(self, *args, **kwargs):
                pass
            def getresponse(self):
                return Response(self.content)
        with tempfile.TemporaryDirectory() as directory:
            cookie = Path(directory) / ".cookie"
            cookie.write_text("u:p", encoding="utf-8")
            rpc = bridge.BitcoinCoreRPC("http://localhost:8332", str(cookie))
            rpc.connection = Connection(b'[{"id":2,"result":"second","error":null},{"id":1,"result":"first","error":null}]')
            self.assertEqual(rpc.batch([("one", []), ("two", [])]), ["first", "second"])
            rpc.connection = Connection(b'[{"id":3,"result":0,"error":null},{"id":3,"result":1,"error":null}]')
            with self.assertRaisesRegex(ValueError, "duplicate"):
                rpc.batch([("one", []), ("two", [])])


if __name__ == "__main__":
    unittest.main()
