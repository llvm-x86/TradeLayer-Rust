#!/usr/bin/env bash
# CI-only preflight: source changes live only inside this disposable checkout.
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
formal="1cee617309af9aa9b2fbcdebc21a21607af4d2c3"
expected_main="40c74283c6f68ec970251fc22c2fc5784a9e2348"
[[ -z "$(git status --porcelain)" ]] || { echo "Dirty checkout" >&2; exit 1; }
git merge-base --is-ancestor "$formal" HEAD || { echo "Unexpected formal ancestry" >&2; exit 1; }
git fetch --no-tags origin +refs/heads/main:refs/remotes/origin/main
main_sha="$(git rev-parse refs/remotes/origin/main)"
[[ "$main_sha" == "$expected_main" ]] || { echo "main advanced to $main_sha" >&2; exit 1; }
if git -c user.name='CI Preview' -c user.email='ci@example.invalid' merge --no-commit --no-ff "$main_sha"; then
    echo 'Merged without conflicts'
else
    git rev-parse -q --verify MERGE_HEAD >/dev/null || { echo "Could not start merge" >&2; exit 1; }
fi
while IFS= read -r path; do
    case "$path" in
        .github/workflows/ci.yml) git checkout --theirs -- "$path" ;;
        src/lib.rs) git checkout --ours -- "$path" ;;
        *) echo "Unexpected source conflict: $path" >&2; exit 1 ;;
    esac
done < <(git diff --name-only --diff-filter=U)
python3 - <<'PY'
from pathlib import Path
lib = Path("src/lib.rs")
s = lib.read_text()
declaration = "pub mod clearing_pnl; // Stage 25C read-only per-address PnL and residual projection."
if declaration not in s:
    anchor = "pub mod clearing_sandbox; // Stage 25A isolated single-ledger derivative margin and funding research."
    if s.count(anchor) != 1:
        raise SystemExit("Cannot place Stage 25C module unambiguously")
    s = s.replace(anchor, declaration + "\n" + anchor)
lib.write_text(s)
for name in (
    "mod atomic_batch;", "mod canonical_columns;", "mod checked_amounts;",
    "mod journal_move;", "mod ordered_columns;", "pub mod clearing_cadence;",
    "pub mod clearing_math;", "pub mod clearing_pnl;", "pub mod clearing_sandbox;",
    "pub mod funding_audit;",
):
    if s.count(name) != 1:
        raise SystemExit(f"Missing or duplicate module: {name}")

p = Path(".github/workflows/ci.yml")
ci = p.read_text()
gate = ("      - name: Native wallet-key Ord and binary-search cross-language oracle\n"
        "        run: python3 scripts/audit_native_wallet_order.py\n")
if gate.strip() not in ci:
    anchor = "      - name: Legacy withdrawal amount mismatch audit\n"
    if ci.count(anchor) != 1:
        raise SystemExit("Missing native wallet-order CI insertion anchor")
    ci = ci.replace(anchor, gate + anchor)
for name in (
    "Native wallet-key Ord and binary-search cross-language oracle",
    "Enforce zero external Rust crates and frozen toolchain",
    "Stage 25C atomic PnL and replay fence tests",
    "Verify Stage 25B single-ledger spot and clearing invariants",
    "Verify Stage 25B collateral floors and fail-closed funding",
    "Reject orphaned clearing replay history in legacy spot checkpoints",
    "Independent Node BigInt cross-position PnL projections",
    "Independent Node BigInt clearing arithmetic vectors",
    "Independent Node BigInt funding direction and balance conservation",
    "Independent Node clearing call cadence and funding bucket vectors",
):
    if ci.count(name) != 1:
        raise SystemExit(f"Missing/duplicate verification gate: {name}")
p.write_text(ci)

for path in (
    "src/clearing_pnl.rs", "tests/clearing_mark_to_market.rs",
    "scripts/audit_clearing_pnl.cjs", "fixtures/clearing_pnl_vectors.tsv",
    "formal/TradeLayerFormal.lean", "tests/native_prefix_checkpoint.rs",
    "src/ordered_columns.rs", "scripts/check_rust_supply_chain.py",
    "scripts/audit_native_wallet_order.py",
):
    if not Path(path).is_file():
        raise SystemExit(f"Missing required merged source or verifier: {path}")
print("Verified Stage 21+25C modules, files and CI gates")
PY
git add src/lib.rs .github/workflows/ci.yml
[[ -z "$(git diff --name-only --diff-filter=U)" ]] || { echo 'Unresolved merge conflict' >&2; exit 1; }
git diff --cached --check
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
python3 scripts/audit_native_wallet_order.py
node scripts/audit_clearing_cadence.cjs --check
node scripts/audit_clearing_math.cjs --check
node scripts/audit_funding_vectors.cjs --check
node scripts/audit_clearing_pnl.cjs --check
cargo test --frozen --test clearing_mark_to_market
cargo test --frozen --test clearing_sandbox
cargo test --frozen --test spot_clearing_one_ledger
cargo test --frozen --test clearing_collateral_policy
cargo test --frozen --test clearing_checkpoint_fence
echo "PASS: Stage 21 proof + Stage 25C main merged-tree preflight"
