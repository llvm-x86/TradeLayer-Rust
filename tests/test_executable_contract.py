"""Adversarial self-tests of the contract runner and protected-path guard."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import guard_contract_authority as guard
import verify_executable_contract as verifier


class ContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads(verifier.CONTRACT.read_text(encoding="utf-8"))

    def test_baseline_coverage(self):
        self.assertTrue(verifier.validate(self.data))

    def test_mutated_check_rejected(self):
        data = copy.deepcopy(self.data)
        data["scenarios"][0]["checks"] = ["parallel-index", "echo green"]
        with self.assertRaises(ValueError):
            verifier.validate(data)
        data = copy.deepcopy(self.data)
        data["scenarios"][1]["checks"] = ["parallel-index"]
        with self.assertRaises(ValueError):
            verifier.validate(data)

    def test_removed_scenario_rejected(self):
        data = copy.deepcopy(self.data)
        data["scenarios"].pop()
        with self.assertRaises(ValueError):
            verifier.validate(data)

    def test_failing_command_stops_suite(self):
        seen = []

        def failing(cmd, **kwargs):
            seen.append(cmd)
            raise subprocess.CalledProcessError(9, cmd)

        self.assertFalse(verifier.run_checks(runner=failing))
        self.assertEqual(len(seen), 1)

    def test_success_executes_all_pinned_commands(self):
        seen = []

        def success(cmd, **kwargs):
            seen.append((cmd, kwargs))

        self.assertTrue(verifier.run_checks(runner=success))
        self.assertEqual([v[0] for v in seen], list(verifier.CHECKS.values()))
        self.assertTrue(all(v[1]["env"]["CARGO_NET_OFFLINE"] == "true" for v in seen))
        self.assertTrue(all(v[1]["check"] is True for v in seen))

    def test_protected_rename_and_deletion_rejected(self):
        changed = [
            {"filename": "src/parallel_txindex.rs"},
            {"filename": "new-file.txt", "previous_filename": "contracts/txindex-readonly.v1.json"},
            {"filename": "tests/parallel_txindex.rs", "status": "removed"},
        ]
        self.assertEqual(guard.violations(changed), [
            "contracts/txindex-readonly.v1.json", "tests/parallel_txindex.rs"
        ])

    def test_implementation_change_permitted(self):
        self.assertEqual(guard.violations([
            {"filename": "src/parallel_txindex.rs"},
            {"filename": "docs/PARALLEL_TXINDEX_RUNTIME.md"},
        ]), [])


if __name__ == "__main__":
    unittest.main()
