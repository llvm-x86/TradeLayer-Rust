#!/usr/bin/env python3
"""Narrow C++ semantics regression for PR #36629 script history merge.

The original Bitcoin Core RPC is not linked by this test. It checks a small
direct C++ extraction of the map key, try_emplace, append and .at behavior.
The Lean schedule theorem supplies the abstract event interleaving.
"""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory() as tmp:
    binary = Path(tmp) / "cpp_reorg_oracle"
    subprocess.run(
        ["g++", "-std=c++20", "-O2", "-Wall", "-Wextra", "-Werror",
         str(root / "cpp_reorg_oracle.cpp"), "-o", str(binary)],
        check=True,
    )
    lines = subprocess.run([str(binary)], check=True, capture_output=True, text=True).stdout.splitlines()

expected = {
    "first_query_synced": "1",
    "second_query_synced": "1",
    "final_tip_gate_accepts": "1",
    "epoch_gate_accepts": "0",
    "height_offset_groups": "1",
    "height_offset_bounds_ok": "0",
    "verbose_at_throws": "1",
    "fork_aware_groups": "2",
    "fork_aware_bounds_ok": "1",
}
found = dict(line.split("=", 1) for line in lines)
assert found == expected, (found, expected)
print("PASS: controlled C++ cross-fork merge extraction, invalid-index symptom, "
      "block-hash-aware alternative, and endpoint guard example")
print("Scope: extraction only; full Bitcoin Core synchronization interleaving untested")
