#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
[[ -z "$(git status --porcelain)" ]] || { echo 'Dirty checkout' >&2; exit 1; }
cp .stage22/ordered_columns_native_kani.rs.txt src/ordered_columns.rs
echo "Staged native string Kani candidate $(sha256sum src/ordered_columns.rs)"
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
echo "PASS: native String Kani candidate passed required source preflight"
