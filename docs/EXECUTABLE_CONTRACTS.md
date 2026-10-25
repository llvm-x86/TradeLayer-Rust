# Executable contracts for autonomous code changes

## First contract: TXINDEX read-only v1

`contracts/txindex-readonly.v1.json` gives five domain-language scenarios. The dependency-free Python verifier `scripts/verify_executable_contract.py` binds them to a hardcoded allowlist of existing Rust integration suites and independent SHA256d and dependency-wave oracles. The manifest is descriptive; commands are pinned in the protected verifier source.

Covered behavior includes transaction position preservation across worker configurations, deterministic first-error selection, fused parser equality, TXID/wTXID reference vectors, bounded read-only dependency waves, and explicit denial of authenticated-consensus authority. Tests run on synthetic inputs and exercise read-only processing. They establish neither Bitcoin consensus validation nor live-funds safety.

Run locally from the repository root:

```bash
python3 scripts/verify_executable_contract.py --validate-only
python3 -m unittest discover -s tests -p test_executable_contract.py -v
python3 scripts/verify_executable_contract.py
bash scripts/ci-preflight.sh --full  # required for Rust-source changes
```

## Two verification planes

**Implementation CI** runs the candidate PR's Rust tests and release build with read-only credentials. The new `executable_contract` job rechecks all pinned acceptance suites and tests the harness against synthetic mutations. The longstanding `check` aggregator must find `tests`, `release` and `executable_contract` successful on the candidate commit.

**Specification authority** runs independently from the trusted *default branch*. The `Executable Contract Authority / contract-authority` GitHub Actions workflow uses `pull_request_target`, read-only GitHub API access, and the base-branch `scripts/guard_contract_authority.py` to inspect PR filenames and rename origins. It rejects edits to the contract manifest, pinned oracles, acceptance targets, verifier scripts, CODEOWNERS, skill instructions and CI workflows. It inspects candidate data without executing candidate code or obtaining secrets.

### Bootstrap and administration

The new base-branch workflow can begin running on PRs only after the bootstrap PR is merged. The initial change requires full manual review. To make the separation enforceable, repository administrators must protect `main`, require both `check` and `contract-authority` status checks, require code-owner review and prevent implementation agents from bypassing checks. A ruleset read returned no repository rulesets, while the branch-protection endpoint was inaccessible to the GitHub App. Configuration is therefore unverified; this PR does not claim immutability.

The authority guard deliberately blocks changes to protected surfaces in ordinary PRs. Changes to the contract need a separately reviewed owner-controlled policy revision with a documented temporary exception or an external authority repository. Avoid PR-controlled waiver labels or self-approved changes. If GitHub restricts `pull_request_target` for public repositories, a repository administrator should explicitly approve the read-only policy or move this guard to an independent trusted runner. Never run PR-head code inside a privileged workflow.

## Agent and Dot handoff

A task coordinator can select a scenario, record the base SHA and assign implementation to Codex while reserving spec changes for a separate human approval flow. A verifier checks the resulting commit and records each scenario's pass/fail result and the actual head SHA. Dot can coordinate that queue when connected; this repository only provides the contract interface and CI runner, not a deployed Dot service.

## Red-state and mutation evidence

For a new scenario, write a failing acceptance test *before* implementation; preserve the red output with a commit reference, then make the same test pass. Existing baseline suites start green. `tests/test_executable_contract.py` tests harness integrity by removing or substituting checks, deleting a scenario, simulating a failing subprocess, and renaming a protected test path. It is a harness mutation exercise rather than a measured mutation score for Rust. Future mutant probes should target ordering, unverified-prevout assumptions and unauthorized state promotion.
