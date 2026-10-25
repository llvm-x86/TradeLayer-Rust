#!/usr/bin/env bash
# Read-only merged-tree validation. Never create/push a repository commit.
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
expected_formal_source="48057fc6e9735415fa93d00ef2d53ef4a8c1088d"
expected_main="7d9154c4b3c7de0bca8c1980e4cc056da62d295a"
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
main_ci = show(".github/workflows/ci.yml", "7d9154c4b3c7de0bca8c1980e4cc056da62d295a")
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
main_lib = show("src/lib.rs", "7d9154c4b3c7de0bca8c1980e4cc056da62d295a")
# Stage25M inserted bitcoin_witness_commitment between bitcoin_inclusion
# and that older module's trailing documentation comment. This exactly
# reviewed source-only normalization restores the comment to its module.
# Any other divergence still fails the full nonmodule comparison below.
old_decl = (
    "pub mod bitcoin_inclusion;\n"
    "pub mod bitcoin_witness_commitment; // Stage25M unverified Bitcoin BIP141 full-tx witness evidence.\n"
    "                                    // Bitcoin SHA256d header and tx Merkle checks; no full consensus.\n"
)
new_decl = (
    "pub mod bitcoin_inclusion; // Bitcoin SHA256d header and tx Merkle checks; no full consensus.\n"
    "pub mod bitcoin_witness_commitment; // Stage25M unverified Bitcoin BIP141 full-tx witness evidence.\n"
)
if main_lib.count(old_decl) != 1 or new_decl in main_lib:
    raise SystemExit("Unexpected Stage25M bitcoin_inclusion comment source; refuse source merge")
main_lib = main_lib.replace(old_decl, new_decl)
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
if len(all_modules) != 50 or len(get_names(merged_ci)) != 76:
    raise SystemExit("Unexpected validity-helper formal merge module/gate count")
required_main_gates = {
    "Stage 25F bounded synthetic bilateral closeout and TLCC rollback",
    "Stage 25F pinned AMM priority and weighted-average accounting diagnostics",
    "Stage 25F source-only exact rational bankruptcy threshold",
    "Check included synthetic bilateral partial reduction formatting",
    "Stage 25G unverified mark and two-of-two admission gate",
    "Stage 25H native P2WSH funding and witness structure (not signatures)",
    "Stage 25I independent hashlib BIP143 and published vector",
    "Stage 25J independent OpenSSL ECDSA two-signer test vectors",
    "Stage 25J public-only signed 2-of-2 fixture Rust parity and always-deny",
    "Stage 25K independent RIPEMD160 HASH160 Python provider reference",
    "Stage 25K legacy P2SH two-of-two script and outpoint, no ECDSA",
    "Stage 25L independent legacy P2SH SIGHASH_ALL and OpenSSL ECDSA",
    "Stage 25L Rust read-only P2SH signed-fixture parity and permanent denial",
    "Stage 25L independent Bitcoin P2WSH Merkle and witness non-auth oracle",
    "Stage 25L dual Bitcoin Merkle inclusion and always-deny tests",
    "Stage 25M independent Python BIP141 coinbase witness vectors",
    "Stage 25M BIP141 witness block Merkle and permanent denial tests",
    "Stage 25M as-of direct clearlist revocation/fork and hard channel-auth refusal",
    "Stage 25M independent Node type4/type19 pinned source-predicate subset",
    "Stage 25N independent Python synthetic type9 txid-Merkle vectors",
    "Stage 25N Bitcoin raw type9 source origin and always-deny evidence",
    "Stage 25O pinned source type9 direct-vs-generic encoding regression",
    "Stage 25O independent OpenSSL ECDSA and P2WPKH Bech32 BIP143 fixture",
    "Stage 25O native P2WPKH first-input research and permanent authorization denial",
    "Pinned legacy channel A/B object guard and missing assignment authority",
    "Legacy activation.js source-registry name and non-genesis parity oracle",
    "Legacy activation.js 36-type/33-threshold/Rust permanent-denial parity",
    "Pinned Validity activation helper versus registry height parity",
}
if not required_main_gates.issubset(get_names(merged_ci)):
    raise SystemExit("Stage25F CI gate omitted")
for required_module in ("atomic_batch", "canonical_columns", "checked_amounts", "journal_move", "ordered_columns", "bankruptcy_source_audit", "liquidation_source_characterization", "liquidation_input_gate", "channel_witness_audit", "channel_bip143_audit", "channel_inclusion_audit", "channel_p2sh_audit", "channel_p2sh_sighash_audit", "hash160", "bitcoin_witness_commitment", "channel_admission_audit", "channel_type9_inclusion_audit", "channel_type9_sender_audit", "legacy_activation_audit"):
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
python3 scripts/audit_channel_ecdsa_openssl.py --check
cargo test --frozen --test channel_signed_evidence
python3 scripts/audit_p2sh_hash160.py
cargo test --frozen --test channel_p2sh_audit
python3 scripts/audit_p2sh_legacy_ecdsa_openssl.py --check
cargo test --frozen --test channel_p2sh_signed_evidence
python3 scripts/audit_bitcoin_channel_merkle_fixture.py
cargo test --frozen --test channel_inclusion_audit
python3 scripts/audit_bip141_witness_fixture.py
cargo test --frozen --test bitcoin_witness_commitment
cargo test --frozen --test channel_admission_audit
node scripts/audit_historical_channel_lists.cjs
python3 scripts/audit_type9_merkle_vectors.py
cargo test --frozen --test channel_type9_inclusion_audit
node scripts/audit_type9_source_paths.cjs
python3 scripts/audit_type9_p2wpkh_openssl.py --check
cargo test --frozen --test channel_type9_sender_audit
node scripts/audit_legacy_channel_column_guard.cjs
node scripts/audit_legacy_activation_parity.cjs
cargo test --frozen --test legacy_activation_audit
node scripts/audit_legacy_validity_activation.cjs
node scripts/audit_liquidation_loss_budget.cjs --check
node scripts/audit_clearing_pnl.cjs --check
node scripts/audit_clearing_math.cjs --check
node scripts/audit_funding_vectors.cjs --check
echo "PASS: pinned activation formal + main validity helper Rust/Lean/Kani preview; NO source push"
