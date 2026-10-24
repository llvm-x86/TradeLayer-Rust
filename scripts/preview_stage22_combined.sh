#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
[[ -z "$(git status --porcelain)" ]] || { echo 'Dirty staged checkout' >&2; exit 1; }
cp .stage22/combined_native_ordered_columns.rs.txt src/ordered_columns.rs
echo "Combined Stage22 exact Rust bytes $(sha256sum src/ordered_columns.rs)"
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
echo 'PASS: combined two-byte and full-u32 property Kani source frozen preflight'
