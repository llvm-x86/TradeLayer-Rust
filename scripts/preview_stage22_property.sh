#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
[[ -z "$(git status --porcelain)" ]] || { echo 'Dirty worktree' >&2; exit 1; }
cp .stage22/property_precedence.rs.txt src/ordered_columns.rs
echo "Verified source SHA256 $(sha256sum src/ordered_columns.rs)"
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
echo 'PASS: Stage22 full-u32 property priority Kani source passed mandatory preflight'
