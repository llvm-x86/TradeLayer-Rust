---
name: executable-contract
description: Apply protected executable acceptance contracts to TradeLayer-Rust changes, especially TXID scanning, dependency waves, consensus boundaries and parallel throughput refactors.
---

# Executable contract workflow

Follow `AGENTS.md` and `docs/EXECUTABLE_CONTRACTS.md` first. The human-approved behavioral contract lives at `contracts/txindex-readonly.v1.json`. Read its scenario IDs and trust limitations before implementation.

1. Choose a scenario ID and state its observable given/when/then behavior. Make the consensus trust boundary explicit.
2. For new behavior, propose a separate reviewed contract addition with independent oracle and a demonstrated failing test on the old implementation. Protected contract paths, referenced acceptance tests, audits and CI scripts belong to the spec authority, and ordinary feature PRs must treat them as read-only.
3. Run `python3 scripts/verify_executable_contract.py --validate-only` before editing. Record the base SHA and any existing red tests.
4. Implement in small chunks. Keep source-order equivalence, deterministic first errors, read-only authority denials and stable dependency waves.
5. Before every Rust-source push, run `bash scripts/ci-preflight.sh --full` under `AGENTS.md`. Run `python3 -m unittest discover -s tests -p test_executable_contract.py -v` and `python3 scripts/verify_executable_contract.py` before proposing a PR.
6. Publish one fully tested branch update when practical. Wait for the required Rust CI `check` aggregator and separate default-branch `contract-authority` gate. Configure the latter as a required branch check after the initial bootstrap merge.
7. Report the exact commit SHA, scenario IDs, red-to-green evidence for new scenarios, verifier output, independent mutants caught, known proof gaps and benchmark methodology.

When a change demands weakening a contract, report the blocked change for human review. Treat the green contract as evidence for tested synthetic read-only behavior, never as authentication of Bitcoin consensus or permission to spend funds.
