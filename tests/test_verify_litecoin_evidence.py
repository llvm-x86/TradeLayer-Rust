"""Stage 23B offline evidence verifier tests.

The end-to-end cases reuse the synthetic fake-RPC history; only the two
Litecoin genesis headers (pinned chainparams constants) are real data.
"""
import copy
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import collect_litecoin_rpc as base  # noqa: E402
import litecoin_source_trace as trace  # noqa: E402
import verify_litecoin_evidence as verifier  # noqa: E402
from test_litecoin_source_trace import FakeHistory  # noqa: E402

GENESIS = ROOT / "fixtures" / "litecoin_genesis_headers.tsv"
COMMIT = trace.LEGACY_JS_COMMIT


def traced(legacy_last_reference=True):
    node = FakeHistory()
    if legacy_last_reference:
        node.child["vout"][2]["scriptPubKey"].pop("address")
        node.child["vout"][2]["scriptPubKey"]["addresses"] = ["Lmodern"]
    corpus, manifest = base.scan_blocks(node, "test", 101, 101, confirmations=6)
    return node, corpus, trace.augment_with_sources(node, corpus, manifest)


def run(corpus, manifest, **kw):
    return verifier.verify([(corpus, json.dumps(manifest))], COMMIT, False, kw.get("rpc"))


class GenesisVectorTests(unittest.TestCase):
    def test_pinned_genesis_headers_hash_and_meet_scrypt_target(self):
        rows = [line.split("\t") for line in GENESIS.read_text().splitlines() if not line.startswith("#")]
        self.assertEqual([r[0] for r in rows], ["main", "test"])
        for network, header_hex, expected in rows:
            self.assertEqual(base.parse_header(header_hex)["hash"], expected)
            self.assertEqual(expected, trace.PINNED_GENESIS[network])
            self.assertTrue(verifier.scrypt_meets_bits(header_hex))
            tampered = header_hex[:-8] + (int.from_bytes(bytes.fromhex(header_hex[-8:]), "little") + 1).to_bytes(4, "little").hex()
            self.assertFalse(verifier.scrypt_meets_bits(tampered))

    def test_is_push_only_matches_core_rules(self):
        self.assertTrue(verifier.is_nulldata("6a"))
        self.assertTrue(verifier.is_nulldata("6a02746c"))
        self.assertTrue(verifier.is_nulldata("6a4c02746c51"))
        self.assertTrue(verifier.is_nulldata("6a0052"))      # OP_0 OP_2
        self.assertFalse(verifier.is_nulldata("6a0002"))     # truncated push
        self.assertFalse(verifier.is_nulldata("6a746c"))      # 0x74 is OP_DEPTH
        self.assertFalse(verifier.is_nulldata("6a05746c"))    # truncated push
        self.assertFalse(verifier.is_nulldata("76a914" + "11" * 20 + "88ac"))


class EndToEndTests(unittest.TestCase):
    def test_authentic_synthetic_trace_is_accepted_for_parity_only(self):
        _, corpus, manifest = traced()
        report = run(corpus, manifest)
        self.assertEqual(report["counts"], {"ACCEPTED_FOR_PARITY": 1, "INCONCLUSIVE": 0, "REJECTED": 0})
        cand = report["manifests"][0]["candidates"][0]
        self.assertEqual(cand["parent_chain_membership"], "node-attested-only")
        self.assertEqual(cand["reference_rules"], {"vout_0_1": [1], "last_non_nulldata": 2, "agree": False})
        self.assertEqual(report["coverage"]["prior_state"], "INCOMPLETE")
        self.assertIn("no TradeLayer acceptance", report["authority"])

    def test_manifest_level_tampering_rejects_everything(self):
        _, corpus, manifest = traced()
        for mutate, reason in [
            (lambda m, c: (m, c + "#\n"), "SHA-256"),
            (lambda m, c: (dict(m, legacy_js_commit="00" * 20), c), "commit"),
            (lambda m, c: (dict(m, joint_replay_authorized=True), c), "replay authority"),
            (lambda m, c: (dict(m, pinned_litecoin_genesis=trace.PINNED_GENESIS["main"]), c), "genesis"),
            (lambda m, c: (dict(m, block_headers={}), c), "cover exactly"),
            (lambda m, c: (dict(m, previous_block_hash="ee" * 32), c), "header chain broken"),
            (lambda m, c: ({k: v for k, v in m.items() if k != "block_headers"}, c), "offline-verifiable"),
        ]:
            m, c = mutate(copy.deepcopy(manifest), corpus)
            with self.assertRaisesRegex(verifier.Rejected, reason):
                run(c, m)

    def test_candidate_level_tampering_is_rejected(self):
        node, corpus, manifest = traced()
        cases = {
            "parent raw bytes do not hash": lambda st, c: st["sender_prevout"].update(
                raw_hex=st["sender_prevout"]["raw_hex"][:-2] + "01"),
            "nulldata classification": lambda st, c: st["decoded_outputs"][1].update(script_type="nulldata"),
            "differs from raw bytes": lambda st, c: st["decoded_outputs"][1].update(satoshis=1),
            "Merkle": lambda st, c: c.update(merkle_branch=["ab" * 32]),
            "carrier class": lambda st, c: c.update(carrier_class="legacy-js-only"),
            "vout 0/1": lambda st, c: st["indexed_reference_outputs_0_1"].clear(),
            "economic authorization": lambda st, c: st.update(actor_economic_authorization=True),
        }
        for reason, mutate in cases.items():
            m = copy.deepcopy(manifest)
            mutate(m["transactions"][0]["source_trace"], m["transactions"][0])
            report = run(corpus, m)
            cand = report["manifests"][0]["candidates"][0]
            self.assertEqual(cand["parity_input"], "REJECTED", reason)
            self.assertRegex(cand["reasons"][0], reason)

    def test_unresolved_actor_or_reference_is_inconclusive(self):
        _, corpus, manifest = traced(legacy_last_reference=False)
        cand = run(corpus, manifest)["manifests"][0]["candidates"][0]
        self.assertEqual(cand["parity_input"], "INCONCLUSIVE")
        self.assertEqual(cand["reasons"], ["last non-nulldata reference address modern-only"])
        _, corpus, manifest = traced()
        m = copy.deepcopy(manifest)
        m["transactions"][0]["source_trace"]["sender_prevout"]["address"]["status"] = "modern-only"
        cand = run(corpus, m)["manifests"][0]["candidates"][0]
        self.assertEqual(cand["parity_input"], "INCONCLUSIVE")
        self.assertIn("sender address modern-only", cand["reasons"])
        # The committed fake's last non-nulldata output only has a modern address.
        m = copy.deepcopy(manifest)
        last = m["transactions"][0]["source_trace"]["last_non_nulldata_reference"]
        for out in m["transactions"][0]["source_trace"]["decoded_outputs"]:
            if out["vout"] == last["vout"]:
                out["address"]["status"] = "ambiguous"
        last["address"]["status"] = "ambiguous"
        cand = run(corpus, m)["manifests"][0]["candidates"][0]
        self.assertEqual(cand["parity_input"], "INCONCLUSIVE")

    def test_recheck_rpc_detects_reorg(self):
        node, corpus, manifest = traced()
        self.assertIn("rechecked 1 heights", run(corpus, manifest, rpc=node)["active_chain"])
        node.reorg_at = 0
        report = run(corpus, manifest, rpc=node)
        self.assertEqual(report["counts"]["REJECTED"], 1)

    def test_coverage_continuity_and_prior_state(self):
        a = {"network": "test", "start_height": 3_082_500, "end_height": 3_082_999,
             "previous_block_hash": "aa", "last_block_hash": "bb"}
        b = dict(a, start_height=3_083_000, end_height=3_083_499, previous_block_hash="bb", last_block_hash="cc")
        gap = dict(b, start_height=3_083_001)
        fork = dict(b, previous_block_hash="zz")
        late = dict(a, start_height=3_082_501)
        self.assertEqual(verifier.coverage([a, b])["prior_state"], "COMPLETE_FROM_LEGACY_JS_GENESIS")
        self.assertEqual(verifier.coverage([late])["prior_state"], "INCOMPLETE")
        before = dict(a, start_height=100, end_height=101)
        self.assertEqual(verifier.coverage([before])["prior_state"], "INCOMPLETE")
        self.assertEqual(verifier.coverage([dict(a, network="regtest")])["prior_state"], "INCOMPLETE")
        self.assertFalse(verifier.coverage([a, gap])["contiguous_hash_linked"])
        self.assertEqual(verifier.coverage([a, fork])["prior_state"], "INCOMPLETE")
        with self.assertRaises(verifier.Rejected):
            verifier.coverage([a, dict(b, network="main")])

if __name__ == "__main__":
    unittest.main()
