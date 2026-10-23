"""Stage 23B replay inputs: validateaddress recorder and the SYNTHETIC chain.

Both run against fake RPC only; neither produces chain evidence.
"""
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "tests"))
import collect_litecoin_rpc as base  # noqa: E402
import litecoin_record_rpc as recorder  # noqa: E402
import synthetic_ltc_chain as synthetic  # noqa: E402


class RecorderTests(unittest.TestCase):
    def test_only_validateaddress_is_recordable(self):
        addresses, other = recorder.parse_requests([
            'validateAddress ["tltc1a"]', 'validateAddress ["tltc1a"]',
            'getrawtransaction ff', 'dumpprivkey ["x"]', 'validateAddress ["a","b"]',
            'validateAddress not-json'])
        self.assertEqual(addresses, ["tltc1a"])
        self.assertEqual(len(other), 4)

    def test_record_keeps_address_fields_and_binds_network(self):
        chain = synthetic.SyntheticChain(synthetic.DEFAULT_PAYLOADS)
        out = recorder.record(chain, "test", [f'validateAddress ["{synthetic.RECIPIENT}"]', "getblockcount"])
        self.assertEqual(out["network"], "litecoin-testnet")
        self.assertEqual(out["unrecordable_requests"], ["getblockcount"])
        answer = out["validateaddress"][synthetic.RECIPIENT]
        self.assertEqual(answer, {"isvalid": True, "address": synthetic.RECIPIENT, "iswitness": True})
        with self.assertRaisesRegex(base.EvidenceError, "unsuitable"):
            recorder.record(chain, "main", [])
        with self.assertRaisesRegex(base.EvidenceError, "too many"):
            recorder.record(chain, "test", ["x"] * (recorder.MAX_REQUESTS + 1))


class SyntheticChainTests(unittest.TestCase):
    def test_bech32_roundtrip_and_admin_constant(self):
        program = synthetic.segwit_v0_program(synthetic.ADMIN)
        self.assertEqual(len(program), 20)
        self.assertEqual(synthetic.segwit_v0_address("tltc", program), synthetic.ADMIN)
        with self.assertRaises(ValueError):
            synthetic.segwit_v0_program(synthetic.ADMIN[:-1] + "q")

    def test_fixture_flows_through_real_pipeline_and_is_marked_synthetic(self):
        with tempfile.TemporaryDirectory() as tmp:
            report = synthetic.write_fixture(Path(tmp), synthetic.DEFAULT_PAYLOADS)
            manifest = json.loads((Path(tmp) / "manifest.json").read_text())
        self.assertEqual(report["counts"], {"ACCEPTED_FOR_PARITY": 3, "INCONCLUSIVE": 0, "REJECTED": 0})
        self.assertEqual(report["coverage"]["prior_state"], "COMPLETE_FROM_LEGACY_JS_GENESIS")
        self.assertIn("never chain evidence", manifest["SYNTHETIC"])
        self.assertEqual(report["scrypt_header_check"], "skipped")
        self.assertEqual(manifest["node_inspection"]["subversion"], "/synthetic-fake-rpc/")


if __name__ == "__main__":
    unittest.main()
