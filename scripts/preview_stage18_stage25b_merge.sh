#!/usr/bin/env bash
# Read-only-in-GitHub merged-tree preflight; never commits or pushes source.
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
expected_formal="464a5a2fdd4bb180779e9b12e4b64a841225f605"
expected_main="c005c92e35c5a4ffe543abf10309c3d20d23c449"
git merge-base --is-ancestor "$expected_formal" HEAD || { echo 'Unexpected formal ancestry' >&2; exit 1; }
[[ -z "$(git status --porcelain)" ]] || { echo 'Dirty checkout' >&2; exit 1; }
git fetch --no-tags origin +refs/heads/main:refs/remotes/origin/main
main_sha="$(git rev-parse refs/remotes/origin/main)"
[[ "$main_sha" == "$expected_main" ]] || { echo "main advanced to $main_sha; rerun review" >&2; exit 1; }
if git -c user.name='Formal CI' -c user.email='ci@example.invalid' merge --no-commit --no-ff "$main_sha"; then
  echo 'Merged without conflicts'
else
  git rev-parse -q --verify MERGE_HEAD >/dev/null || { echo 'Cannot start preview merge' >&2; exit 1; }
fi
while IFS= read -r path; do
  case "$path" in
    .github/workflows/ci.yml) git checkout --theirs -- "$path" ;;
    *) echo "Unreviewed source conflict: $path" >&2; exit 1 ;;
  esac
done < <(git diff --name-only --diff-filter=U)
python3 - <<'PY'
from pathlib import Path
ci_file = Path(".github/workflows/ci.yml")
ci = ci_file.read_text()
gate = ("      - name: Native wallet-key Ord and binary-search cross-language oracle\n"
        "        run: python3 scripts/audit_native_wallet_order.py\n")
if ci.count("Native wallet-key Ord and binary-search cross-language oracle") == 0:
    anchor = "      - name: Legacy withdrawal amount mismatch audit\n"
    if ci.count(anchor) != 1:
        raise SystemExit("Cannot locate wallet oracle CI insertion anchor")
    ci = ci.replace(anchor, gate + anchor)
for name in (
    "Native wallet-key Ord and binary-search cross-language oracle",
    "Enforce zero external Rust crates and frozen toolchain",
    "Verify Stage 25A isolated margin and funding atomicity",
    "Verify Stage 25B single-ledger spot and clearing invariants",
    "Verify Stage 25B collateral floors and fail-closed funding",
    "Reject orphaned clearing replay history in legacy spot checkpoints",
    "Independent Node BigInt clearing arithmetic vectors",
    "Independent Node BigInt funding direction and balance conservation",
    "Independent Node clearing call cadence and funding bucket vectors",
):
    if ci.count(name) != 1:
        raise SystemExit(f"Missing/duplicate merged CI gate: {name}")
ci_file.write_text(ci)
for path in (
    "tests/native_prefix_checkpoint.rs",
    "tests/spot_clearing_one_ledger.rs",
    "tests/clearing_collateral_policy.rs",
    "tests/clearing_checkpoint_fence.rs",
    "formal/TradeLayerFormal.lean",
    "src/ordered_columns.rs",
    "src/clearing_sandbox.rs",
):
    if not Path(path).is_file():
        raise SystemExit(f"Missing merged source or test: {path}")
print("Preserved native storage proof and Stage 25A/B clearing checks")
PY
git add .github/workflows/ci.yml
[[ -z "$(git diff --name-only --diff-filter=U)" ]] || { echo 'Unresolved conflicts' >&2; exit 1; }
git diff --cached --check
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
python3 scripts/audit_native_wallet_order.py
node scripts/audit_clearing_cadence.cjs --check
node scripts/audit_clearing_math.cjs --check
node scripts/audit_funding_vectors.cjs --check
cargo test --frozen --test clearing_sandbox
cargo test --frozen --test spot_clearing_one_ledger
cargo test --frozen --test clearing_collateral_policy
cargo test --frozen --test clearing_checkpoint_fence
echo 'PASS: merged Stage18 + Stage25B Rust preflight; no GitHub source push'
