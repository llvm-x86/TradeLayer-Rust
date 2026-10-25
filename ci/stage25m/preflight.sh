#!/usr/bin/env bash
# Prepublish Stage25M Rust in an ephemeral GitHub runner. The staged PR owns
# only TEXT and CI harness files; publishing Rust requires full green preflight.
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
expected_base="3625646263fd00cb56d53cc1a2a7e11e9791fb41"
[[ -z "$(git status --porcelain)" ]] || { echo "Dirty staged checkout" >&2; exit 1; }
git merge-base --is-ancestor "$expected_base" HEAD || {
    echo "Stage25M wrong main source base" >&2; exit 1;
}
git fetch --no-tags origin +refs/heads/main:refs/remotes/origin/main
expected_main="12d458e04df58bff5a0f295642b3a04774be988c"
git merge-base --is-ancestor "$expected_main" refs/remotes/origin/main || {
    echo "Pinned Stage25M admission source no longer in main ancestry" >&2; exit 1;
}
while IFS= read -r path; do
    case "$path" in
        ci/stage25m/*|.github/workflows/stage25m-bip141-preflight.yml) ;;
        *) echo "Unexpected prepublication production source edit: $path" >&2; exit 1 ;;
    esac
done < <(git diff --name-only "$expected_base" HEAD)
# Compose BOTH Stage25M research efforts on this ephemeral checkout.
# The admitted source is pinned, even if newer unrelated main commits appear.
git -c user.name="Stage25M CI" -c user.email="ci@example.invalid" merge --no-commit --no-ff "$expected_main" || {
    echo "Unexpected Stage25M admission merge conflict" >&2; exit 1;
}
[[ -z "$(git diff --name-only --diff-filter=U)" ]] || {
    echo "Unresolved staged admission merge conflict" >&2; exit 1;
}
[[ "$(git hash-object src/lib.rs)" == "aa018961ab02375d6265d3f006517210d2d79e2e" ]] || {
    echo "Changed source module baseline" >&2; exit 1;
}
[[ "$(git hash-object .github/workflows/ci.yml)" == "3c24923fe38ead476dadeff997111bde1401ac69" ]] || {
    echo "Changed Rust CI baseline" >&2; exit 1;
}

cp ci/stage25m/bitcoin_witness_commitment.rs.txt src/bitcoin_witness_commitment.rs
cp ci/stage25m/bitcoin_witness_commitment_tests.rs.txt tests/bitcoin_witness_commitment.rs
cp ci/stage25m/stage25m_bip141_vectors.tsv.txt fixtures/stage25m_bip141_vectors.tsv
cp ci/stage25m/audit_bip141_witness_fixture.py.txt scripts/audit_bip141_witness_fixture.py

python3 - <<'PY'
from pathlib import Path
lib = Path("src/lib.rs")
source = lib.read_text()
anchor = "pub mod bitcoin_inclusion;"
module = (
    "pub mod bitcoin_witness_commitment; "
    "// Stage25M unverified Bitcoin BIP141 full-tx witness evidence.\n"
)
if source.count(anchor) != 1 or "mod bitcoin_witness_commitment" in source:
    raise SystemExit("Stage25M Rust module anchor changed")
lib.write_text(source.replace(anchor, anchor + "\n" + module))

workflow = Path(".github/workflows/ci.yml")
source = workflow.read_text()
anchor = "      - name: Stage 25L independent Bitcoin P2WSH Merkle and witness non-auth oracle\n"
extra = (
    "      - name: Stage 25M independent Python BIP141 coinbase witness vectors\n"
    "        run: python3 scripts/audit_bip141_witness_fixture.py\n"
    "      - name: Stage 25M BIP141 witness block Merkle and permanent denial tests\n"
    "        run: cargo test --frozen --test bitcoin_witness_commitment\n"
)
if source.count(anchor) != 1 or "Stage 25M BIP141 witness block" in source:
    raise SystemExit("Stage25M Rust CI anchor changed")
workflow.write_text(source.replace(anchor, extra + anchor))
names = [line for line in workflow.read_text().splitlines()
         if line.startswith("      - name: ")]
if len(names) != 66 or len(set(names)) != 66:
    raise SystemExit(f"Expected exactly 64 distinct CI steps; found {len(names)}")
print(f"STAGE25M_CI_GATES={len(names)}")
PY

# Only the checked staged source is formatted in the disposable runner.
cargo fmt --all
for modified in $(git diff --name-only); do
    case "$modified" in
        src/lib.rs|.github/workflows/ci.yml) ;;
        *) echo "Unexpected source modification: $modified" >&2; exit 1 ;;
    esac
done
git diff --check
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
python3 scripts/audit_bip141_witness_fixture.py
bash scripts/ci-preflight.sh --full
cargo test --frozen --test bitcoin_witness_commitment
cargo test --frozen --test bitcoin_inclusion --test channel_inclusion_audit --test channel_signed_evidence
cargo build --all-targets --release --frozen

echo "STAGE25M_SRC_SHA=$(git hash-object src/bitcoin_witness_commitment.rs)"
echo "STAGE25M_TEST_SHA=$(git hash-object tests/bitcoin_witness_commitment.rs)"
echo "STAGE25M_PY_SHA=$(git hash-object scripts/audit_bip141_witness_fixture.py)"
echo "STAGE25M_VECTORS_SHA=$(git hash-object fixtures/stage25m_bip141_vectors.tsv)"
echo "STAGE25M_LIB_SHA=$(git hash-object src/lib.rs)"
echo "STAGE25M_CI_SHA=$(git hash-object .github/workflows/ci.yml)"

python3 - <<'PY'
import base64
from pathlib import Path
for path in (
    "src/bitcoin_witness_commitment.rs",
    "tests/bitcoin_witness_commitment.rs",
    "scripts/audit_bip141_witness_fixture.py",
    "fixtures/stage25m_bip141_vectors.tsv",
    "src/lib.rs",
    ".github/workflows/ci.yml",
):
    data = Path(path).read_bytes()
    print("STAGE25M_BASE64 " + path + " " + base64.b64encode(data).decode("ascii"))
PY
echo "PASS: Stage25M BIP141 full Rust prepublication checks; no Rust source published"
