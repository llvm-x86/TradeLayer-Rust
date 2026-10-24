#!/usr/bin/env bash
# Validate candidate Rust source before making any permanent Rust-source push.
set -Eeuo pipefail
cd "$(git rev-parse --show-toplevel)"
src="tests/panic_stage_atomicity.rs"
[[ ! -e "$src" ]] || { echo "unexpected preexisting test path" >&2; exit 1; }
cp .stage18c/panic_stage.rs.txt "$src"
echo "Candidate SHA256 $(sha256sum "$src")"
python3 scripts/check_rust_supply_chain.py
python3 -m unittest discover -s tests -p test_rust_supply_chain.py -v
bash scripts/ci-preflight.sh --full
cargo test --frozen --test panic_stage_atomicity
cargo build --all-targets --release --frozen
echo "PASS Stage18c staged source, production Ledger panic-isolation test"
