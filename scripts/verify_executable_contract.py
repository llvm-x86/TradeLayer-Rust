#!/usr/bin/env python3
"""Fail-closed, stdlib-only executor for pinned TX-index behavioral contracts."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
CONTRACT = ROOT / "contracts" / "txindex-readonly.v1.json"
# Fixed commands prevent a prose spec from substituting a trivial green check.
CHECKS = {
    "hash-oracle": ["python3", "scripts/audit_txindex_hash_vectors.py"],
    "dependency-oracle": ["python3", "scripts/audit_stage8d_dependency_waves.py"],
    "parallel-index": ["cargo", "test", "--frozen", "--test", "parallel_txindex"],
    "fused-dependencies": ["cargo", "test", "--frozen", "--test", "stage8d_fused_dependency"],
    "one-team": ["cargo", "test", "--frozen", "--test", "stage8d_one_team"],
    "raw-block": ["cargo", "test", "--frozen", "--test", "raw_block_ingest"],
    "hash-fastpath": ["cargo", "test", "--frozen", "--test", "txindex_hash_fastpath"],
}
REQUIRED = {
    "TX-ORDER-001": {"parallel-index", "one-team", "hash-oracle"},
    "TX-FAIL-002": {"parallel-index", "fused-dependencies"},
    "TX-WAVE-003": {"fused-dependencies", "dependency-oracle", "raw-block"},
    "TX-BOUNDARY-004": {"parallel-index", "fused-dependencies", "one-team"},
    "TX-HASH-005": {"hash-oracle", "hash-fastpath"},
}


def validate(data):
    if not isinstance(data, dict) or data.get("schema") != 1 or data.get("id") != "TL-TXINDEX-READONLY-V1":
        raise ValueError("unrecognized contract schema/id")
    scenarios = data.get("scenarios")
    if not isinstance(scenarios, list) or len(scenarios) != len(REQUIRED):
        raise ValueError("required scenario count changed")
    seen = set()
    union = set()
    for item in scenarios:
        if not isinstance(item, dict):
            raise ValueError("invalid scenario")
        ident = item.get("id")
        if ident not in REQUIRED or ident in seen:
            raise ValueError(f"unknown or duplicate scenario: {ident}")
        seen.add(ident)
        for field in ("given", "when", "then"):
            if not isinstance(item.get(field), str) or not item[field].strip():
                raise ValueError(f"{ident}: missing {field}")
        checks = item.get("checks")
        if not isinstance(checks, list) or any(not isinstance(c, str) for c in checks):
            raise ValueError(f"{ident}: invalid checks")
        listed = set(checks)
        if len(listed) != len(checks) or not REQUIRED[ident].issubset(listed) or not listed.issubset(CHECKS):
            raise ValueError(f"{ident}: missing, duplicate or unknown check")
        union.update(listed)
    if seen != set(REQUIRED) or union != set(CHECKS):
        raise ValueError("contract lost mandatory coverage")
    if not isinstance(data.get("limits"), list) or not data["limits"]:
        raise ValueError("trust limitations missing")
    return True


def run_checks(runner=subprocess.run):
    env = dict(os.environ, CARGO_NET_OFFLINE="true")
    for ident, command in CHECKS.items():
        print(f'RUN {ident}: {" ".join(command)}', flush=True)
        try:
            runner(command, cwd=ROOT, env=env, check=True, timeout=600)
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError) as exc:
            print(f"FAIL {ident}: {exc}", file=sys.stderr, flush=True)
            return False
        print(f"PASS {ident}", flush=True)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validate-only", action="store_true")
    args = parser.parse_args()
    try:
        validate(json.loads(CONTRACT.read_text(encoding="utf-8")))
    except (ValueError, OSError) as exc:
        print(f"FAIL contract manifest: {exc}", file=sys.stderr)
        return 1
    print("PASS contract shape and pinned check registry", flush=True)
    return 0 if args.validate_only or run_checks() else 1


if __name__ == "__main__":
    sys.exit(main())
