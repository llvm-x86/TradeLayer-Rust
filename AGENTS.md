# Agent development instructions

TradeLayer's Rust CI runs after each pull-request update. Frequent pushes while formatting or Clippy is failing generate a separate failure notification for each intermediate commit.

Before **each GitHub push that changes Rust source**, run `bash scripts/ci-preflight.sh --full` in the repository root. This checks the pinned Rust formatting, the included implementation files that Cargo fmt does not cover, Clippy with warnings denied, and Rust tests. Fix the local errors and amend or update the commit before publishing the branch. For local iterations, `--fast` omits the Rust test run; the full command remains required before pushing.

When altering Lean or Kani work, run its relevant local verifier as well. The preflight script covers only the first Rust CI gates. Leave GitHub's broader Bitcoin, Litecoin, Kani, Lean, and replay audits active, and keep proof claims scoped to checks that completed successfully.

Prefer one tested branch update over a sequence of knowingly failing intermediate pushes. Preserve failing evidence and diagnose regressions instead of weakening CI checks.

## Executable acceptance contracts

Read [Executable Contracts](docs/EXECUTABLE_CONTRACTS.md) and use the repo-local `executable-contract` skill when changing TXID, raw-block ingestion, scheduling or read-only dependency waves. The contract manifest, acceptance tests, independent oracles, verifier and CI workflows are owned by a separate specification authority. Implementation PRs must not alter them. Run `python3 scripts/verify_executable_contract.py` and the harness self-tests before proposing a PR. A newly needed contract scenario requires a separately reviewed failing test before implementation.
