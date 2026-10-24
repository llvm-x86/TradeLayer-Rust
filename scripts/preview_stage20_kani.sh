#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
[[ -z "$(git status --porcelain)" ]] || { echo "Dirty checkout" >&2; exit 1; }
cp .stage20/ordered_columns_kani.rs.txt src/ordered_columns.rs
echo "Stage20 candidate Git source SHA256 $(sha256sum src/ordered_columns.rs)"
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
echo 'PASS: exact Stage20 Kani source passed full Rust preflight and frozen Cargo'
