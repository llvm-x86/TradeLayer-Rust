#!/usr/bin/env bash
# Validates a staged Rust integration test without publishing changes to
# the canonical Rust test tree before the complete local-equivalent preflight.
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
test ! -e tests/native_prefix_checkpoint.rs
cp .stage18/native_prefix_checkpoint.rs.txt tests/native_prefix_checkpoint.rs
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo build --all-targets --release --frozen
python3 scripts/audit_native_wallet_order.py
node scripts/audit_clearing_cadence.cjs --check
node scripts/audit_clearing_math.cjs --check
node scripts/audit_funding_vectors.cjs --check
cargo test --frozen --test clearing_sandbox
echo 'Stage 18 test preview passed; published Rust file remains unchanged.'
