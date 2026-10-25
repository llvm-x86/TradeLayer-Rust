#!/usr/bin/env bash
# Read-only merged-tree validation. Never create/push a repository commit.
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
expected_formal_source="8feeec52e2c7a7b49b9e4dbcae3da5438714bceb"
expected_main="dbe9bafb69493bff11c421a0be8f0982e27863c3"
[[ -z "$(git status --porcelain)" ]] || { echo "Dirty candidate checkout" >&2; exit 1; }
git merge-base --is-ancestor "$expected_formal_source" HEAD || {
    echo "Unexpected Stage22 source ancestry" >&2; exit 1;
}
git fetch --no-tags origin +refs/heads/main:refs/remotes/origin/main
actual_main="$(git rev-parse refs/remotes/origin/main)"
git merge-base --is-ancestor "$expected_main" "$actual_main" || {
    echo "Pinned main is no longer ancestor of current main: $actual_main; fail closed" >&2; exit 1;
}
echo "CURRENT_MAIN=$actual_main"
echo "MERGE_PREFLIGHT_FORMAL=$expected_formal_source"
echo "MERGE_PREFLIGHT_MAIN=$expected_main"
if git -c user.name='Formal CI' -c user.email='ci@example.invalid' merge --no-commit --no-ff "$expected_main"; then
    echo 'Automatic source merge'
else
    git rev-parse -q --verify MERGE_HEAD >/dev/null || {
        echo "Unexpected failure before merge conflict stage" >&2; exit 1;
    }
fi
while IFS= read -r path; do
    case "$path" in
       .github/workflows/ci.yml|src/lib.rs)
           git checkout --theirs -- "$path" ;;
       *)
           echo "Unreviewed source conflict: $path" >&2; exit 1 ;;
    esac
done < <(git diff --name-only --diff-filter=U)
python3 - <<'PY'
from pathlib import Path
import re
import subprocess

def show(path, revision):
    return subprocess.check_output(["git", "show", f"{revision}:{path}"], text=True)

formal_ci = show(".github/workflows/ci.yml", "HEAD")
main_ci = show(".github/workflows/ci.yml", "dbe9bafb69493bff11c421a0be8f0982e27863c3")
get_names = lambda text: set(re.findall(r"^      - name: (.+)$", text, re.MULTILINE))
additional = get_names(formal_ci) - get_names(main_ci)
required = {"Native wallet-key Ord and binary-search cross-language oracle"}
if additional != required:
    raise SystemExit(f"Unreviewed formal-only CI gates: {additional!r}")
gate = ("      - name: Native wallet-key Ord and binary-search cross-language oracle\n"
        "        run: python3 scripts/audit_native_wallet_order.py\n")
anchor = "      - name: Legacy withdrawal amount mismatch audit\n"
if main_ci.count(anchor) != 1 or gate in main_ci:
    raise SystemExit("Cannot uniquely restore native wallet oracle")
merged_ci = main_ci.replace(anchor, gate + anchor)
for name in get_names(formal_ci) | get_names(main_ci):
    if merged_ci.count("- name: " + name) != 1:
        raise SystemExit(f"Missing or duplicate CI gate: {name}")
Path(".github/workflows/ci.yml").write_text(merged_ci)

formal_lib = show("src/lib.rs", "HEAD")
main_lib = show("src/lib.rs", "dbe9bafb69493bff11c421a0be8f0982e27863c3")
module_pattern = re.compile(r"^(?:(?:pub|pub\(crate\)) )?mod (\w+);[^\n]*$", re.MULTILINE)
def modules(source):
    matches = list(module_pattern.finditer(source))
    items = {x.group(1): x.group(0) for x in matches}
    if len(items) != len(matches):
        raise SystemExit("Duplicate modules in input source")
    return items
m, f = modules(main_lib), modules(formal_lib)
def without_modules(source):
    # Remove entire declaration lines, not just their text: one branch
    # legitimately has more modules/newline separators than the other.
    return "".join(line for line in source.splitlines(keepends=True)
                   if not module_pattern.fullmatch(line.strip()))
if without_modules(main_lib) != without_modules(formal_lib):
    raise SystemExit("lib.rs nonmodule code diverged; refuse automated union")
all_modules = dict(m)
for name, source in f.items():
    if name in all_modules and all_modules[name] != source:
        raise SystemExit(f"Unreviewed module declaration conflict for {name}")
    all_modules[name] = source
lines = main_lib.splitlines(keepends=True)
indices = [i for i, line in enumerate(lines)
           if module_pattern.fullmatch(line.rstrip("\n"))]
if not indices or indices != list(range(min(indices), max(indices)+1)):
    raise SystemExit("Module section is not contiguous")
lo, hi = min(indices), max(indices)
new_lib = ("".join(lines[:lo]) +
           "".join(all_modules[name] + "\n" for name in sorted(all_modules)) +
           "".join(lines[hi+1:]))
if modules(new_lib) != all_modules:
    raise SystemExit("Module union mismatch")
if len(all_modules) != 41 or len(get_names(merged_ci)) != 55:
    raise SystemExit("Unexpected Stage25I merge module/gate count")
required_main_gates = {
    "Stage 25F bounded synthetic bilateral closeout and TLCC rollback",
    "Stage 25F pinned AMM priority and weighted-average accounting diagnostics",
    "Stage 25F source-only exact rational bankruptcy threshold",
    "Check included synthetic bilateral partial reduction formatting",
    "Stage 25G unverified mark and two-of-two admission gate",
    "Stage 25H native P2WSH funding and witness structure (not signatures)",
    "Stage 25I independent hashlib BIP143 and published vector",
}
if not required_main_gates.issubset(get_names(merged_ci)):
    raise SystemExit("Stage25F CI gate omitted")
for required_module in ("atomic_batch", "canonical_columns", "checked_amounts", "journal_move", "ordered_columns", "bankruptcy_source_audit", "liquidation_source_characterization", "liquidation_input_gate", "channel_witness_audit", "channel_bip143_audit"):
    if required_module not in all_modules:
        raise SystemExit(f"Required formal/Stage25F module missing: {required_module}")
Path("src/lib.rs").write_text(new_lib)
print(f"PASS: merged CI union {len(get_names(merged_ci))} gates; "
      f"all {len(all_modules)} module declarations preserved")
PY
git add .github/workflows/ci.yml src/lib.rs
[[ -z "$(git diff --name-only --diff-filter=U)" ]] || {
    echo 'Unresolved merge conflicts' >&2; exit 1;
}
git diff --cached --check
echo "MERGED_CI_BLOB=$(git hash-object .github/workflows/ci.yml)"
echo "MERGED_LIB_BLOB=$(git hash-object src/lib.rs)"
echo "MERGED_SOURCE_TREE_SHA256=$(git diff --cached --name-only | sha256sum)"
python3 scripts/check_rust_supply_chain.py
python3 scripts/audit_native_wallet_order.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
rustfmt --check --edition 2021 src/clearing_snapshot_impl.rs src/clearing_distress_impl.rs src/clearing_partial_impl.rs
cargo build --all-targets --release --frozen
cargo test --frozen --test clearing_mark_to_market --test clearing_snapshot --test clearing_chain \
  --test insurance_ledger_v2 --test insured_distressed_closeout --test liquidation_preflight \
  --test clearing_checkpoint_fence --test clearing_collateral_policy --test spot_clearing_one_ledger \
  --test clearing_partial_closeout --test liquidation_source_characterization --test bankruptcy_source_audit \
  --test liquidation_input_gate --test channel_witness_audit
python3 scripts/audit_bip143_channel_fixture.py
node scripts/audit_liquidation_loss_budget.cjs --check
node scripts/audit_clearing_pnl.cjs --check
node scripts/audit_clearing_math.cjs --check
node scripts/audit_funding_vectors.cjs --check
echo "PASS: pinned Stage25H formal + main Stage25I Rust/Lean/Kani preview; NO source push"
