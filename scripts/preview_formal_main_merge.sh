#!/usr/bin/env bash
# CI-only merge preview; no source commit or remote push.
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
proof="b6467d893692f58687445488a9a3a0fa0034cee9"
expected_main="83080fedd6bfdbc368871e4c29ad26d4332bb70f"
[[ -z "$(git status --porcelain)" ]] || { echo 'dirty preview worktree' >&2; exit 1; }
git merge-base --is-ancestor "$proof" HEAD || { echo 'proof ancestry changed' >&2; exit 1; }
git fetch --no-tags origin +refs/heads/main:refs/remotes/origin/main
main="$(git rev-parse refs/remotes/origin/main)"
[[ "$main" == "$expected_main" ]] || { echo "main advanced to $main, re-review needed" >&2; exit 1; }
printf 'CI merge preview formal=%s main=%s\n' "$(git rev-parse HEAD)" "$main"
if git -c user.name='CI Preview' -c user.email='ci@example.invalid' merge --no-ff --no-commit "$main"; then
    echo 'Auto-merge clean'
else
    git rev-parse -q --verify MERGE_HEAD >/dev/null || { echo 'cannot start merge' >&2; exit 1; }
fi
while IFS= read -r path; do
    case "$path" in
        src/lib.rs|.github/workflows/ci.yml) git checkout --theirs -- "$path" ;;
        *) echo "Unexpected conflict: $path" >&2; exit 1 ;;
    esac
done < <(git diff --name-only --diff-filter=U)
python3 - <<'PY'
from pathlib import Path
import re

lib_path = Path("src/lib.rs")
lib = lib_path.read_text()
insertions = [
    ("mod atomic_batch; // Clone-validate-commit boundary and Kani rollback harnesses.",
     "pub mod bitcoin_chainwork;"),
    ("mod canonical_columns; // Canonical nonzero map mutations for wallet and channel columns.",
     "pub mod chain;"),
    ("mod checked_amounts; // Bit-precise checked debit/credit arithmetic; Kani proof harnesses.",
     "pub mod clearing_cadence;"),
    ("mod journal_move; // Fail-closed single-use settlement journal movement; Kani harness.",
     "pub mod ledger;"),
    ("mod ordered_columns; // Sorted Vec backend for wallet and channel accounting columns.",
     "pub mod protocol;"),
]
def mod_count(declaration):
    # Treat comments as independent from module identity, including an
    # auto-merged declaration with a changed explanatory comment.
    prefix = declaration.split(";", 1)[0] + ";"
    return len(re.findall(r"(?m)^" + re.escape(prefix) + r"(?:[ \t]*//[^\n]*)?$", lib))

for declaration, anchor in insertions:
    count = mod_count(declaration)
    if count > 1:
        raise SystemExit(f"Duplicate declaration: {declaration}")
    if count == 0:
        if lib.count(anchor) != 1:
            raise SystemExit(f"Missing/ambiguous module anchor {anchor}. Review current main manually.")
        lib = lib.replace(anchor, declaration + "\n" + anchor, 1)
for declaration in (
    "pub mod clearing_cadence;", "pub mod clearing_math;", "pub mod funding_audit;", *[row[0] for row in insertions]
):
    if mod_count(declaration) != 1:
        raise SystemExit(f"Missing/duplicate required Rust module: {declaration}")
lib_path.write_text(lib)

ci_path = Path(".github/workflows/ci.yml")
ci = ci_path.read_text()
step = (
    "      - name: Native wallet-key Ord and binary-search cross-language oracle\n"
    "        run: python3 scripts/audit_native_wallet_order.py\n"
)
if ci.count("Native wallet-key Ord and binary-search cross-language oracle") > 1:
    raise SystemExit("Duplicate native wallet CI audit")
if "Native wallet-key Ord and binary-search cross-language oracle" not in ci:
    anchor = "      - name: Legacy withdrawal amount mismatch audit\n"
    if ci.count(anchor) != 1:
        raise SystemExit("Missing/ambiguous CI insertion anchor")
    ci = ci.replace(anchor, step + anchor, 1)
for check in (
    "Native wallet-key Ord and binary-search cross-language oracle",
    "Independent Node clearing call cadence and funding bucket vectors",
    "Independent Node BigInt funding direction and balance conservation",
    "Independent Node BigInt clearing arithmetic vectors",
    "Optional legacy funding runner parses without local checkout",
    "run: python3 scripts/audit_native_wallet_order.py",
    "run: node scripts/audit_clearing_cadence.cjs --check",
    "run: node scripts/audit_clearing_math.cjs --check",
    "run: node scripts/audit_funding_vectors.cjs --check",
    "run: node --check scripts/check_legacy_funding_runtime.cjs",
):
    if ci.count(check) != 1:
        raise SystemExit(f"Missing/duplicate expected CI gate: {check}")
for gate in (
    "Enforce zero external Rust crates and frozen toolchain",
    "Dependency gate adversarial mutation tests",
    "Release build without network or lockfile changes",
    "run: python3 scripts/check_rust_supply_chain.py",
    "run: python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v",
    "run: cargo build --all-targets --release --frozen",
):
    if ci.count(gate) != 1:
        raise SystemExit(f"Missing/duplicate supply-chain CI gate: {gate}")
ci_path.write_text(ci)

for module in ("atomic_batch", "canonical_columns", "checked_amounts", "journal_move", "ordered_columns"):
    if not Path(f"src/{module}.rs").exists():
        raise SystemExit(f"Missing proof-source module src/{module}.rs")
if not Path("formal/TradeLayerFormal.lean").exists():
    raise SystemExit("Formal proofs missing")
if not Path(".cargo/config.toml").exists() or not Path("security/cargo-dependency-policy.toml").exists():
    raise SystemExit("Missing frozen supply-chain policy files")
print("Verified formal/Stage24A-C/security merged module and gate inventory")
PY
# Main's frozen Cargo security policy bans extra manifest sections. The
# formal branch's [lints.rust] existed solely to declare Kani's cfg name.
# Restore the approved main manifest and scope the cfg lint suppression
# narrowly to the five Kani-only proof modules.
git checkout "$main" -- Cargo.toml
python3 - <<'PY'
from pathlib import Path
proof_modules = (
    "atomic_batch", "canonical_columns", "checked_amounts",
    "journal_move", "ordered_columns",
)
for module in proof_modules:
    path = Path(f"src/{module}.rs")
    text = path.read_text()
    marker = "#[cfg(kani)]"
    if text.count(marker) != 1 or "#![allow(unexpected_cfgs)]" in text:
        raise SystemExit(f"Unexpected Kani configuration in {path}")
    if not text.startswith("//!") or "\n\n" not in text:
        raise SystemExit(f"Unexpected Rust module header in {path}")
    before, after = text.split("\n\n", 1)
    text = before + "\n\n#![allow(unexpected_cfgs)]\n\n" + after
    path.write_text(text)
print("Manifest remains approved; Kani-only modules suppress their own cfg lint")
PY
git add src/lib.rs .github/workflows/ci.yml Cargo.toml
git add src/atomic_batch.rs src/canonical_columns.rs src/checked_amounts.rs src/journal_move.rs src/ordered_columns.rs
[[ -z "$(git diff --name-only --diff-filter=U)" ]] || { echo 'Unresolved conflict' >&2; exit 1; }
git diff --cached --check
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
python3 scripts/audit_native_wallet_order.py
node scripts/audit_clearing_cadence.cjs --check
node scripts/audit_clearing_math.cjs --check
node scripts/audit_funding_vectors.cjs --check
node --check scripts/check_legacy_funding_runtime.cjs
echo 'PASS: complete Rust merge preflight and Stage 24 + supply-chain audits; no source pushed.'
