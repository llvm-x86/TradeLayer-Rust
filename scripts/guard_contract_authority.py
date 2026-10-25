#!/usr/bin/env python3
"""Default-branch-only PR metadata guard. Never execute candidate PR code."""
import json
import os
import sys
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

LOCKED_PREFIXES = (".github/", ".agents/skills/executable-contract/", "contracts/")
LOCKED_FILES = frozenset({
    "AGENTS.md",
    "docs/EXECUTABLE_CONTRACTS.md",
    "scripts/verify_executable_contract.py",
    "scripts/guard_contract_authority.py",
    "tests/test_executable_contract.py",
    "tests/parallel_txindex.rs",
    "tests/stage8d_fused_dependency.rs",
    "tests/stage8d_one_team.rs",
    "tests/raw_block_ingest.rs",
    "tests/txindex_hash_fastpath.rs",
    "scripts/audit_txindex_hash_vectors.py",
    "scripts/audit_stage8d_dependency_waves.py",
})


def protected(filename):
    return isinstance(filename, str) and (
        filename in LOCKED_FILES or any(filename.startswith(prefix) for prefix in LOCKED_PREFIXES)
    )


def violations(files):
    touched = set()
    for item in files:
        for key in ("filename", "previous_filename"):
            name = item.get(key)
            if protected(name):
                touched.add(name)
    return sorted(touched)


def get_files(repository, number, token):
    if not repository or "/" not in repository or not token or number < 1:
        raise ValueError("missing repository, token or PR number")
    collected = []
    for page in range(1, 32):
        url = f"https://api.github.com/repos/{repository}/pulls/{number}/files?per_page=100&page={page}"
        request = Request(url, headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "TradeLayer-contract-authority/1",
        })
        with urlopen(request, timeout=25) as response:
            batch = json.load(response)
        if not isinstance(batch, list):
            raise ValueError("malformed GitHub PR-files response")
        collected.extend(batch)
        if len(batch) < 100:
            if not collected:
                raise ValueError("empty PR changed-file list")
            return collected
    raise ValueError("PR file count exceeds bounded authority scan")


def main():
    try:
        files = get_files(
            os.environ["GITHUB_REPOSITORY"],
            int(os.environ["PULL_REQUEST_NUMBER"]),
            os.environ["GITHUB_TOKEN"],
        )
        bad = violations(files)
    except (ValueError, KeyError, HTTPError, URLError, TimeoutError, OSError) as exc:
        print(f"FAIL contract authority API: {exc}", file=sys.stderr)
        return 1
    if bad:
        print("FAIL protected contract changes require separately approved policy update:", file=sys.stderr)
        for path in bad:
            print(f"  {path}", file=sys.stderr)
        return 1
    print(f"PASS contract authority: inspected {len(files)} PR files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
