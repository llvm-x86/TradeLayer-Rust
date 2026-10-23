"""Stage 23A: hermetic Litecoin Core source trace regression fixtures.

The RPC is fake and every transaction here is synthetic. Tests establish
fidelity to source bytes and legacy address selection, never chain consensus.
"""
import importlib.util
from pathlib import Path
import sys
import unittest

SCRIPTS = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPTS))
import collect_litecoin_rpc as collector  # noqa: E402
import litecoin_source_trace as trace  # noqa: E402


def raw_tx(prev_txid, prev_vout, outputs):
    data = bytearray(bytes.fromhex("01000000"))
    data.extend(b"\x01")
    data.extend(bytes.fromhex(prev_txid)[::-1])
    data.extend(prev_vout.to_bytes(4, "little"))
    data.extend(b"\x00")
    data.extend(b"\xff\xff\xff\xff")
    data.append(len(outputs))
    for value, script in outputs:
        data.extend(value.to_bytes(8, "little"))
        data.append(len(script))
        data.extend(script)
    data.extend(b"\x00\x00\x00\x00")
    return data.hex()


def output(n, satoshis, script_hex, address=None, legacy=True):
    spk = {"hex": script_hex, "type": "nulldata" if script_hex.startswith("6a") else "pubkeyhash"}
    if address:
        spk["addresses" if legacy else "address"] = [address] if legacy else address
    return {"n": n, "value": satoshis / 100_000_000, "scriptPubKey": spk}


class FakeHistory:
    """Two linked node-accepted blocks with parent outpoint provenance."""
    def __init__(self):
        self.h0, self.h1, self.old = "10" * 32, "20" * 32, "01" * 32
        self.genesis = trace.PINNED_GENESIS["test"]
        self.origin_script = bytes.fromhex("76a914" + "11" * 20 + "88ac")
        self.ref_script = bytes.fromhex("76a914" + "22" * 20 + "88ac")
        self.ref2_script = bytes.fromhex("76a914" + "33" * 20 + "88ac")
        self.payload = b"tl20;alice;5;2"
        self.opret = bytes([0x6a, len(self.payload)]) + self.payload
        self.parent_raw = raw_tx("ab" * 32, 0, [(500_000_000, self.origin_script)])
        self.prev_id, _ = collector.inspect_raw_transaction(self.parent_raw)
        self.child_raw = raw_tx(self.prev_id, 0, [
            (0, self.opret), (10_000, self.ref_script), (20_000, self.ref2_script)
        ])
        self.child_id, _ = collector.inspect_raw_transaction(self.child_raw)
        self.parent = {
            "txid": self.prev_id, "blockhash": self.h0, "confirmations": 30,
            "vout": [output(0, 500_000_000, self.origin_script.hex(), "Lsender")],
        }
        self.child = {
            "txid": self.child_id, "hex": self.child_raw,
            "vin": [{"txid": self.prev_id, "vout": 0}],
            "vout": [output(0, 0, self.opret.hex()),
                     output(1, 10_000, self.ref_script.hex(), "Lreference"),
                     output(2, 20_000, self.ref2_script.hex(), "Lmodern", legacy=False)],
        }
        self.parent_block = {
            "hash": self.h0, "height": 100, "confirmations": 31,
            "merkleroot": collector.merkle_root([self.prev_id]), "tx": [self.prev_id],
        }
        self.child_block = {
            "hash": self.h1, "height": 101, "confirmations": 30,
            "previousblockhash": self.h0,
            "merkleroot": collector.merkle_root([self.child_id]), "tx": [self.child],
        }
        self.hash_queries = 0
        self.reorg_at = 1_000_000

    def call(self, method, params=None):
        params = params or []
        if method == "getblockchaininfo":
            return {"chain": "test", "pruned": False, "initialblockdownload": False}
        if method == "getblockcount":
            return 130
        if method == "getblockhash":
            if params == [0]:
                return self.genesis
            if params == [99]:
                return self.old
            if params == [100]:
                return self.h0
            if params == [101]:
                self.hash_queries += 1
                return self.h1 if self.hash_queries < self.reorg_at else "30" * 32
        if method == "getblock":
            if params == [self.h1, 2]:
                return self.child_block
            if params == [self.h0, 1]:
                return self.parent_block
        if method == "getrawtransaction":
            if params == [self.child_id, False, self.h1]:
                return self.child_raw
            if params == [self.prev_id, True]:
                return self.parent
            if params == [self.prev_id, False, self.h0]:
                return self.parent_raw
        raise AssertionError((method, params))


class SourceTraceTests(unittest.TestCase):
    def scan(self, node):
        return collector.scan_blocks(node, "test", 101, 101, confirmations=6)

    def test_first_input_raw_prevout_and_two_reference_rules(self):
        node = FakeHistory()
        corpus, manifest = self.scan(node)
        result = trace.augment_with_sources(node, corpus, manifest)
        self.assertEqual(result["source_trace_format"], "litecoin-legacy-js-source-trace-v1")
        self.assertFalse(result["joint_replay_authorized"])
        self.assertEqual(result["transactions_selected"], 1)
        item = result["transactions"][0]["source_trace"]
        source = item["sender_prevout"]
        self.assertEqual(source["txid"], node.prev_id)
        self.assertEqual(source["satoshis"], 500_000_000)
        self.assertEqual(source["address"]["address"], "Lsender")
        self.assertEqual(source["address"]["status"], "legacy-exact")
        self.assertEqual([r["vout"] for r in item["indexed_reference_outputs_0_1"]], [1])
        self.assertEqual(item["last_non_nulldata_reference"]["vout"], 2)
        self.assertEqual(item["last_non_nulldata_reference"]["address"]["status"], "modern-only")
        self.assertEqual(result["corpus_sha256"],
                         __import__("hashlib").sha256(corpus.encode()).hexdigest())

    def test_raw_outpoint_is_checked_against_decoded_vin(self):
        node = FakeHistory()
        corpus, manifest = self.scan(node)
        node.child["vin"][0]["vout"] = 1
        with self.assertRaisesRegex(collector.EvidenceError, "decoded input disagrees"):
            trace.augment_with_sources(node, corpus, manifest)

    def test_parent_output_amount_and_script_must_match_raw_bytes(self):
        node = FakeHistory()
        corpus, manifest = self.scan(node)
        node.parent["vout"][0]["value"] = 2
        with self.assertRaisesRegex(collector.EvidenceError, "value differs"):
            trace.augment_with_sources(node, corpus, manifest)
        node.parent["vout"][0]["value"] = 5
        node.parent["vout"][0]["scriptPubKey"]["hex"] = "6a"
        with self.assertRaisesRegex(collector.EvidenceError, "script differs"):
            trace.augment_with_sources(node, corpus, manifest)

    def test_parent_missing_from_checked_block_rejected(self):
        node = FakeHistory()
        corpus, manifest = self.scan(node)
        node.parent_block["tx"] = ["cd" * 32]
        node.parent_block["merkleroot"] = "cd" * 32
        with self.assertRaisesRegex(collector.EvidenceError, "missing or later"):
            trace.augment_with_sources(node, corpus, manifest)

    def test_reorg_while_reading_source_is_rejected(self):
        node = FakeHistory()
        corpus, manifest = self.scan(node)
        node.reorg_at = node.hash_queries + 2
        with self.assertRaisesRegex(collector.EvidenceError, "reorganized"):
            trace.augment_with_sources(node, corpus, manifest)

    def test_wrong_bitcoin_family_node_is_rejected_by_genesis(self):
        node = FakeHistory()
        corpus, manifest = self.scan(node)
        node.genesis = "00" * 32
        with self.assertRaisesRegex(collector.EvidenceError, "wrong chain genesis"):
            trace.augment_with_sources(node, corpus, manifest)
        node.genesis = trace.PINNED_GENESIS["test"]
        accepted = trace.augment_with_sources(node, corpus, manifest)
        self.assertEqual(accepted["pinned_litecoin_genesis"], node.genesis)

    def test_corpus_mismatch_is_rejected(self):
        node = FakeHistory()
        corpus, manifest = self.scan(node)
        with self.assertRaisesRegex(collector.EvidenceError, "corpus"):
            trace.augment_with_sources(node, corpus.replace(node.child_id, "00" * 32), manifest)

    def test_coinbase_input_and_address_schema_are_fail_closed(self):
        node = FakeHistory()
        _, manifest = self.scan(node)
        node.parent["vout"][0]["scriptPubKey"]["addresses"] = ["a", "b"]
        self.assertEqual(trace.script_address(node.parent["vout"][0]["scriptPubKey"])["status"], "ambiguous")
        node.parent["vout"][0]["scriptPubKey"]["addresses"] = ["a"]
        node.parent["vout"][0]["scriptPubKey"]["address"] = "b"
        self.assertEqual(trace.script_address(node.parent["vout"][0]["scriptPubKey"])["status"], "schema-conflict")
        coinbase = raw_tx("00" * 32, 0xffffffff, [(0, node.opret)])
        node.child_raw = coinbase
        node.child_id, _ = collector.inspect_raw_transaction(coinbase)
        node.child["txid"], node.child["hex"] = node.child_id, coinbase
        node.child["vin"] = [{"coinbase": "00"}]
        node.child["vout"] = [output(0, 0, node.opret.hex())]
        node.child_block["merkleroot"] = collector.merkle_root([node.child_id])
        corpus, manifest = self.scan(node)
        with self.assertRaises(collector.EvidenceError):
            trace.augment_with_sources(node, corpus, manifest)

    def test_decimal_output_strictness(self):
        self.assertEqual(trace.require_satoshis("0.00000001"), 1)
        with self.assertRaises(collector.EvidenceError):
            trace.require_satoshis("0.000000001")
        with self.assertRaises(collector.EvidenceError):
            trace.require_satoshis(-1)


if __name__ == "__main__":
    unittest.main()
