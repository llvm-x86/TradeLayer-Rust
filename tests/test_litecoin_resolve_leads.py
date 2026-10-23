"""Stage 23B lead resolution: fake RPC and a loopback-only HTTP stub.

Every transaction and block here is synthetic. The committed leads file is
parsed for shape only; no lead is resolved against a real chain in CI.
"""
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
from pathlib import Path
import socket
import sys
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import collect_litecoin_rpc as base  # noqa: E402
import litecoin_resolve_leads as leads  # noqa: E402
import litecoin_source_trace as trace  # noqa: E402

LEADS = ROOT / "fixtures" / "litecoin_historical_leads.tsv"


def raw_tx(scripts):
    data = bytearray(bytes.fromhex("0100000001") + b"\x33" * 32 + bytes(4) + b"\x00" + b"\xff" * 4)
    data.append(len(scripts))
    for script in scripts:
        data += (5000).to_bytes(8, "little") + bytes([len(script)]) + script
    return (bytes(data) + bytes(4)).hex()


class FakeNode:
    def __init__(self, chain="test", tip=3_500_000):
        self.chain, self.tip = chain, tip
        self.opret = bytes.fromhex("6a04746c3141")
        self.raw = raw_tx([self.opret, bytes.fromhex("76a914" + "44" * 20 + "88ac")])
        self.txid = base.parse_transaction(self.raw)["txid"]
        self.height, self.block_hash = 3_432_676, "55" * 32
        self.reorg = False

    def call(self, method, params=None):
        params = params or []
        if method == "getblockchaininfo":
            return {"chain": self.chain, "pruned": False, "initialblockdownload": False,
                    "blocks": self.tip, "headers": self.tip}
        if method == "getnetworkinfo":
            return {"version": 1, "subversion": "/synthetic/", "protocolversion": 1}
        if method == "getindexinfo":
            return {"txindex": {"synced": True, "best_block_height": self.tip}}
        if method == "getblockcount":
            return self.tip
        if method == "getblockhash":
            if params == [0]:
                return trace.PINNED_GENESIS[self.chain]
            if params == [self.height]:
                return "66" * 32 if self.reorg else self.block_hash
        if method == "getrawtransaction":
            if params == [self.txid, True]:
                return {"txid": self.txid, "blockhash": self.block_hash, "vout": [
                    {"n": 0, "value": 0.00005, "scriptPubKey": {"hex": self.opret.hex(), "type": "nulldata"}},
                    {"n": 1, "value": 0.00005, "scriptPubKey": {
                        "hex": "76a914" + "44" * 20 + "88ac", "type": "pubkeyhash", "addresses": ["Lx"]}}]}
            if params == [self.txid, False, self.block_hash]:
                return self.raw
            if len(params) == 2:
                raise base.RPCResponseError("no such tx", -5)
        if method == "getblock" and params[0] == self.block_hash:
            return {"hash": self.block_hash, "height": self.height, "confirmations": 100,
                    "merkleroot": base.merkle_root([self.txid]), "tx": [self.txid]}
        raise AssertionError((method, params))


def lead(lead_id, txid, network="test", height="-"):
    return {"lead_id": lead_id, "claimed_network": network, "network_basis": "synthetic",
            "txid": txid, "claimed_height": None if height == "-" else int(height),
            "source": "synthetic", "role": "synthetic"}


class LeadFileTests(unittest.TestCase):
    def test_committed_leads_are_well_formed_and_unverified(self):
        rows = leads.read_leads(LEADS)
        self.assertEqual(len(rows), 16)
        self.assertEqual(len({r["txid"] for r in rows}), 16)
        header = LEADS.read_text().splitlines()[0]
        self.assertIn("dca0888b827c5bc28fa9189bc9e1d507f5c5670c", header)
        self.assertIn("UNVERIFIED", LEADS.read_text().splitlines()[1])
        by_id = {r["lead_id"]: r for r in rows}
        self.assertEqual(by_id["T1"]["claimed_height"], 3432676)
        self.assertEqual(by_id["T2"]["claimed_height"], 3496378)
        self.assertEqual({r["claimed_network"] for r in rows}, {"test", "main", "unknown"})

    def test_malformed_lead_rows_fail_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "bad_leads.tsv"
            for row in ["X\ttest\tb\tzz\t-\ts\tr", "X\tdoge\tb\t" + "00" * 32 + "\t-\ts\tr",
                        "X\ttest\tb\t" + "00" * 32 + "\t12a\ts\tr"]:
                bad.write_text(row + "\n")
                with self.assertRaises(base.EvidenceError):
                    leads.read_leads(bad)


class ResolutionTests(unittest.TestCase):
    def test_resolves_match_mismatch_not_found_and_skips_other_network(self):
        node = FakeNode()
        report = leads.resolve_leads(node, "test", [
            lead("A", node.txid, height="3432676"),
            lead("B", node.txid, height="3496378"),
            lead("C", "77" * 32),
            lead("D", "88" * 32, network="main"),
            lead("E", "99" * 32, network="unknown"),
        ], 6, include_unknown=False)
        status = {item["lead_id"]: item for item in report["leads"]}
        self.assertEqual(status["A"]["status"], "RESOLVED")
        self.assertEqual(status["A"]["claimed_height_check"], "match")
        self.assertEqual(status["A"]["carrier_class"], "strict-and-legacy-js")
        self.assertEqual(status["B"]["claimed_height_check"], "MISMATCH")
        self.assertEqual(status["C"]["status"], "NOT_FOUND_ON_THIS_CHAIN")
        self.assertEqual(status["D"]["status"], "SKIPPED_OTHER_NETWORK")
        self.assertEqual(status["E"]["status"], "SKIPPED_OTHER_NETWORK")
        self.assertEqual(report["suggested_windows"],
                         [{"start_height": 3432676, "end_height": 3433175, "lead_ids": ["A", "B"]}])
        self.assertIn("no TradeLayer acceptance", report["evidence_status"])

    def test_window_clamped_to_confirmed_tip_and_reorg_rejected(self):
        node = FakeNode(tip=3_432_700)
        report = leads.resolve_leads(node, "test", [lead("A", node.txid)], 6, False)
        self.assertEqual(report["suggested_windows"][0]["end_height"], 3_432_695)
        node.reorg = True
        with self.assertRaisesRegex(base.EvidenceError, "no longer on the node's active chain|reorganized"):
            leads.resolve_leads(node, "test", [lead("A", node.txid)], 6, False)

    def test_testnet_leads_never_queried_on_mainnet_and_wrong_genesis_refused(self):
        node = FakeNode(chain="main")
        report = leads.resolve_leads(node, "main", [lead("T", node.txid)], 6, False)
        self.assertEqual(report["leads"][0]["status"], "SKIPPED_OTHER_NETWORK")
        self.assertEqual(report["suggested_windows"], [])
        wrong = FakeNode()
        wrong.chain = "test"
        original = wrong.call
        wrong.call = lambda m, p=None: ("00" * 32 if m == "getblockhash" and p == [0] else original(m, p))
        with self.assertRaisesRegex(base.EvidenceError, "genesis"):
            leads.resolve_leads(wrong, "test", [], 6, False)

    def test_other_rpc_errors_are_not_treated_as_absence(self):
        node = FakeNode()
        original = node.call

        def failing(method, params=None):
            if method == "getrawtransaction":
                raise base.RPCResponseError("work queue depth exceeded", -1)
            return original(method, params)
        node.call = failing
        with self.assertRaises(base.RPCResponseError):
            leads.resolve_leads(node, "test", [lead("A", node.txid)], 6, False)


class LoopbackTransportTests(unittest.TestCase):
    """Real LitecoinRPC over a loopback socket: error classes, not node data."""

    @classmethod
    def setUpClass(cls):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                method = body["method"]
                status, reply = {
                    "getblockcount": (200, {"result": 7, "error": None}),
                    "getrawtransaction": (500, {"result": None, "error": {"code": -5, "message": "x"}}),
                    "getindexinfo": (404, {"result": None, "error": {"code": -32601, "message": "x"}}),
                    "getblockhash": (401, None),
                    "getnetworkinfo": (500, {"result": 1, "error": None}),
                }[method]
                data = b"" if reply is None else json.dumps(reply).encode()
                self.send_response(status)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        cls.server = HTTPServer(("127.0.0.1", 0), Handler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.rpc = base.LitecoinRPC(f"http://127.0.0.1:{cls.server.server_port}", "u:p")

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def test_error_classes(self):
        self.assertEqual(self.rpc.call("getblockcount"), 7)
        with self.assertRaises(base.RPCResponseError) as ctx:
            self.rpc.call("getrawtransaction", ["00" * 32, True])
        self.assertEqual(ctx.exception.code, -5)
        with self.assertRaises(base.RPCMethodNotFound):
            self.rpc.call("getindexinfo")
        with self.assertRaises(base.EvidenceError) as ctx:
            self.rpc.call("getblockhash", [0])
        self.assertNotIsInstance(ctx.exception, base.RPCResponseError)
        with self.assertRaisesRegex(base.EvidenceError, "without RPC error object"):
            self.rpc.call("getnetworkinfo")

    def test_lost_connection_is_transport_failure(self):
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        rpc = base.LitecoinRPC(f"http://127.0.0.1:{port}", "u:p")
        with self.assertRaises(base.EvidenceError) as ctx:
            rpc.call("getblockcount")
        self.assertNotIsInstance(ctx.exception, base.RPCResponseError)


if __name__ == "__main__":
    unittest.main()
