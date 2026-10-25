#!/usr/bin/env bash
# Stage25L: validate staged Rust source BEFORE it is published as Rust.
# This runs only in a disposable checkout. It never commits or pushes.
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
expected_main="1ad1e3791d1fa105fc20f2acb79bba59b6e82c87"
[[ -z "$(git status --porcelain)" ]] || { echo "Dirty checkout" >&2; exit 1; }
git merge-base --is-ancestor "$expected_main" HEAD || { echo "Wrong base" >&2; exit 1; }
git fetch --no-tags origin +refs/heads/main:refs/remotes/origin/main
git merge-base --is-ancestor "$expected_main" refs/remotes/origin/main || {
    echo "Pinned Stage25K is no longer main ancestry" >&2; exit 1;
}
# A PR preview must add staging-only files. It may not silently overwrite
# any production Rust, Cargo, workflows or existing tests before checking.
while IFS= read -r path; do
    case "$path" in
      ci/stage25l/*|.github/workflows/stage25l-inclusion-preview.yml) ;;
      *) echo "Unexpected staged production edit: $path" >&2; exit 1 ;;
    esac
done < <(git diff --name-only "$expected_main" HEAD)
[[ "$(git hash-object src/lib.rs)" == "d334cfa9aba97ba2b64d118038dac802cd84de25" ]] || {
    echo "Unexpected baseline src/lib.rs" >&2; exit 1;
}
[[ "$(git hash-object .github/workflows/ci.yml)" == "3df8099cccafbc536be6e4af7683fdbc6f976ee5" ]] || {
    echo "Unexpected baseline Rust CI" >&2; exit 1;
}
cp ci/stage25l/channel_inclusion_audit.rs.txt src/channel_inclusion_audit.rs
cp ci/stage25l/channel_inclusion_audit_tests.rs.txt tests/channel_inclusion_audit.rs
cp ci/stage25l/audit_bitcoin_channel_merkle_fixture.py.txt scripts/audit_bitcoin_channel_merkle_fixture.py
python3 - <<'PY'
from pathlib import Path
lib = Path("src/lib.rs")
text = lib.read_text()
anchor = "pub mod channel_p2sh_audit;"
insert = (
    "pub mod channel_inclusion_audit; "
    "// Stage25L relative Bitcoin Merkle/witness non-authority.\n"
)
if text.count(anchor) != 1 or "mod channel_inclusion_audit" in text:
    raise SystemExit("Invalid module insertion anchor")
lib.write_text(text.replace(anchor, insert + anchor))

ci = Path(".github/workflows/ci.yml")
text = ci.read_text()
anchor = "      - name: Stage 25K independent RIPEMD160 HASH160 Python provider reference\n"
new = (
    "      - name: Stage 25L independent Bitcoin P2WSH Merkle and witness non-auth oracle\n"
    "        run: python3 scripts/audit_bitcoin_channel_merkle_fixture.py\n"
    "      - name: Stage 25L dual Bitcoin Merkle inclusion and always-deny tests\n"
    "        run: cargo test --frozen --test channel_inclusion_audit\n"
)
if text.count(anchor) != 1 or "Stage 25L dual Bitcoin Merkle" in text:
    raise SystemExit("Invalid CI insertion anchor")
ci.write_text(text.replace(anchor, new + anchor))
names = [l for l in ci.read_text().splitlines() if l.startswith("      - name: ")]
if len(names) != len(set(names)):
    raise SystemExit("Duplicate Rust CI steps")
print(f"CI_STAGE25L_GATES={len(names)}")
PY
git diff --check
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
# Required policy gate before any production Rust-source GitHub push.
bash scripts/ci-preflight.sh --full
python3 scripts/audit_bitcoin_channel_merkle_fixture.py
cargo test --frozen --test channel_inclusion_audit
cargo test --frozen --test bitcoin_inclusion --test channel_witness_audit --test channel_signed_evidence
cargo build --all-targets --release --frozen
echo "STAGE25L_SOURCE_BLOB=$(git hash-object src/channel_inclusion_audit.rs)"
echo "STAGE25L_TEST_BLOB=$(git hash-object tests/channel_inclusion_audit.rs)"
echo "STAGE25L_VECTOR_BLOB=$(git hash-object scripts/audit_bitcoin_channel_merkle_fixture.py)"
echo "STAGE25L_LIB_BLOB=$(git hash-object src/lib.rs)"
echo "STAGE25L_CI_BLOB=$(git hash-object .github/workflows/ci.yml)"
echo "PASS: Stage25L full frozen Rust prepublication check, NO Rust source push"
