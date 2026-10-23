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
import export_ltc_evidence as exporter  # noqa: E402
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


class ExportCompareTests(unittest.TestCase):
    def fixture(self):
        with tempfile.TemporaryDirectory() as tmp:
            synthetic.write_fixture(Path(tmp), synthetic.DEFAULT_PAYLOADS)
            return (json.loads((Path(tmp) / "manifest.json").read_text()),
                    (Path(tmp) / "corpus.tsv").read_text())

    def test_export_lines_carry_raw_bytes_only(self):
        manifest, corpus = self.fixture()
        lines = exporter.export(manifest, corpus).splitlines()
        self.assertTrue(lines[0].startswith("# litecoin-evidence-export-v1\ttest\t3082500\t"))
        self.assertEqual([l.split("\t")[0] for l in lines[1:]], ["H", "C", "C", "C"])
        self.assertNotIn("tltc1", "\n".join(lines))  # no node-decoded addresses
        with self.assertRaises(base.EvidenceError):
            exporter.export(dict(manifest, network="litecoin-regtest"), corpus)

    def test_compare_classifies_each_outcome(self):
        manifest, _ = self.fixture()
        txids = [c["txid"] for c in manifest["transactions"]]
        ok_refs = f"1:{synthetic.RECIPIENT}"
        rows = [
            "# header",
            "\t".join([txids[0], "h", "1", "m", f"single:{synthetic.ADMIN}", "true", ok_refs, ok_refs]),
            "\t".join([txids[1], "h", "2", "m", "ambiguous", "false", ok_refs, ok_refs]),
            "REJECTED\t3082500\tNotInAttestedChain",
        ]
        result = exporter.compare("\n".join(rows), manifest)
        self.assertEqual(result["counts"], {"MATCH": 1, "MISMATCH": 1, "NOT_AUTHENTICATED": 1})
        self.assertEqual(len(result["rejected_blocks"]), 1)
        self.assertFalse(result["records"][1]["fields"]["sender"])
        self.assertEqual(exporter.expected_sender({"status": "schema-conflict"}), "conflict:schema-conflict")
        self.assertEqual(exporter.expected_sender({"status": "unresolved"}), "none")


if __name__ == "__main__":
    unittest.main()
