#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
cp .stage17/ordered_columns.rs.txt src/ordered_columns.rs
echo "Candidate Rust blob SHA256: $(sha256sum src/ordered_columns.rs)"
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
node scripts/audit_clearing_cadence.cjs --check
node scripts/audit_clearing_math.cjs --check
node scripts/audit_funding_vectors.cjs --check
cargo test --frozen --test clearing_sandbox
echo 'Staged Stage 17 Rust source passed full preflight. Kani gate follows.'
