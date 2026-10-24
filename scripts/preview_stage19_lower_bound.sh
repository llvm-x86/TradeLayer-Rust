#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
test -z "$(git status --porcelain)" || { echo 'Dirty candidate checkout' >&2; exit 1; }
git merge-base --is-ancestor aab10d5be60f01b27c9c733f9d93fc5533130664 HEAD || { echo 'Unexpected candidate ancestry' >&2; exit 1; }
cp .stage19/ordered_columns.rs.txt src/ordered_columns.rs
echo "Candidate source SHA256 $(sha256sum src/ordered_columns.rs)"
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
python3 scripts/audit_native_wallet_order.py
cargo test --frozen --lib native_lower_bound_matches_independent_linear_count
echo 'PASS: Stage 19 explicit lower-bound candidate passes frozen Rust preflight'
