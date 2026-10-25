# Parallel parity migration: Codex worktree lanes and CI critical path

Status: scheduling and ownership contract for the remaining source-parity files. It grants NO protocol, financial, wallet or channel authorization. Baseline main 973cca97 after source subset 7/49 (txEncoder.js), legacy JS pinned to tradelayer.js@dca0888b.

## Empirical CI baseline, not a promised speedup

Main successful GitHub Actions run 40182647219 on 2026-10-25 lasted approximately 84 seconds (09:55:29–09:56:53 UTC). Its single serial runner included a **42-second frozen Cargo release build**, 18-second full Rust test, 6-second Clippy, and 8-second pinned toolchain install. Most other independent Node/Python or source-vector steps needed 0–1 second each. Release + test is the avoidable critical-path dependency.

This PR splits Rust CI into:
- tests: every PREEXISTING check EXCEPT the release build, unchanged.
- release: separate Ubuntu 24.04 runner, pinned checkout, Rust 1.90, zero third-party crate policy, and EXACTLY the original frozen Cargo release build.
- check: a strict aggregator that needs BOTH tests and release. Its original job ID is retained for branch protection compatibility. It uses always() so a failed, skipped or cancelled prerequisite cannot count as successful. The entire workflow must be green on the exact PR head and after merging main.

Parallel runners consume more total billable minutes; queue or quota effects may offset benefit. Estimate ~50–60s critical path but measure five post-merge runs before claiming a win.

## More important: eliminate remote CI as iterative formatter

Previous one-file PRs required repeated Actions rounds to fix Rustfmt, Clippy and occasional independent Node fixture regressions. The LOCAL Codex orchestrator should run the pinned formatter, Clippy, complete Rust tests, original-source Node oracle and supply-chain policy BEFORE its first feature PR push. Push once for a validated logical slice, instead of one remote commit per source file. Preserve GitHub concurrency cancellation for superseded PR revisions.

The local build/orchestration agent is Codex alone. This document stages disjoint work for Codex worktrees/subagents if available; creating a branch, doc or GitHub issue does not start an agent.

## Three independent owner lanes plus one integrator

| Lane | Work | Exclusively owned paths | Non-dependency |
|---|---|---|---|
| A: ingestion | File8 txIndex.js: two OP_RETURN scanners, 3-to-9 decoder argument mismatch, block/tx positions, error-object propagation, mock RPC, checked index persistence; no active chain assertion | src/legacy_txindex_audit.rs, tests/legacy_txindex_audit.rs, scripts/audit_legacy_txindex_source.cjs, fixtures/legacy_txindex_*, docs/LEGACY_TXINDEX_JS_PARITY.md | No live reliance on lane B |
| B: first input | File9 txUtils.js: first-vin parent output provenance, address/script candidate, reference outputs, missing parent and fork fail-close, BTC only unless separately proved LTC/DOGE | src/legacy_txutils_audit.rs, tests/legacy_txutils_audit.rs, scripts/audit_legacy_txutils_source.cjs, fixtures/legacy_txutils_*, docs/LEGACY_TXUTILS_JS_PARITY.md | Existing Stage25O research, not lane A |
| C: codecs | Source Base94/Base256 exact codec parity, shadowed Type23 settlement and malformed amounts; no final economic state | src/legacy_base94_audit.rs, src/legacy_base256_audit.rs, tests/legacy_base94_audit.rs, tests/legacy_base256_audit.rs, scripts/audit_legacy_base94_256_source.cjs, fixtures/legacy_base94_*, fixtures/legacy_base256_*, docs/LEGACY_BASE94_256_PARITY.md | Pin original base94.js/base256.js, not lane A/B |
| I: coordinator | Cross-workstream review, common exports, workflow steps, strict integration CI and one guarded merge at a time | ONLY owner of src/lib.rs, .github/workflows/ci.yml, Cargo.toml, Cargo.lock, integration branch and final merge | Completed, tested lane artifacts |

Run a MAXIMUM of three implementation workers and one integrator until throughput and merge collisions have been measured. Workers may internally divide research, code and adversarial fixtures, but each file must have a single writer. A source-subset parity PR is NOT a full production port.

## Delivery contract and merge train

1. Freeze main SHA and historical JS blob hashes for each lane. Write those exact pins into issue/PR.
2. Freeze a source fixture contract before Rust implementation. All new artifacts go exclusively into that lane's owned paths. Do not modify another worker's branch, source files or shared exports.
3. LOCAL PRE-PUSH CHECKS: run cargo fmt --all -- --check; cargo clippy --all-targets --frozen -- -D warnings; cargo test --frozen; cargo build --all-targets --release --frozen; original-source Node/Python oracle; python3 scripts/check_rust_supply_chain.py. Workers can temporarily install uncommitted module exports to compile; restore shared src/lib.rs BEFORE publishing. If local compilation is unavailable, mark the lane UNVERIFIED and hand its code to the integrator, not a falsely green PR.
4. Push one source-formatted logical set of commits. Code Mode connector should publish one atomic Git tree when running without local git, not trigger dozens of superseding GitHub Actions runs.
5. Integrator alone installs module exports and common workflow gates. If main moved, reconcile and rerun on the EXACT combined source. No stale-green cherry-picked assumptions.
6. Require GitHub WHOLE-WORKFLOW SUCCESS on exact PR head: tests, release and the preserved aggregator check ALL green. An incomplete source handler, missing provenance, unverified sender/UTXO, list state, source code-hash or economic revision never grants authority.
7. Merge one verified integration PR at a time; require post-merge main full CI green. Keep source limitations and consensus blockers in Issue #80. Do not force-push over other worker worktrees.
8. No trusted Ledger/clearing/type3/23/31, liquidation or offchain two-signer revision API. Future consensus change requires ratified versioning, active-chain and full-script evidence, actual list/admin replay, canonical both-signed state, supersession and dispute.

## Monitor actual throughput

Per completed lane record: baseline SHA, elapsed wall time, verified source function/vector count, coder/editor time, commit count, rustfmt/Clippy reruns, CI test/release/check durations, PR conflicts, and regression count. Optimize merged-and-verified functionality/hour instead of PR count or generated code volume. If third lane increases queueing/merge conflicts, reduce workers to two. Review five fully green main runs to evaluate CI split; revert if more queue delay than savings.

Do not imply these issues are executing autonomously. They are ready-to-assign work packets for Codex. No method here authenticates the historical source or turns a source-compatible parser into a spend authority.
