#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
[[ -z "$(git status --porcelain)" ]] || { echo "Dirty checkout" >&2; exit 1; }
cp .stage22/two_byte_ordered_columns.rs.txt src/ordered_columns.rs
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
echo "Staged Stage22 two-byte comparator code passed full Rust preflight"
