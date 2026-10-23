#!/usr/bin/env bash
# Match the first Rust CI gates before publishing an intermediate commit.
set -euo pipefail
cd "$(dirname "$0")/.."

case "${1:---full}" in
  --fast|--full) mode="${1:---full}" ;;
  *) echo "Usage: bash scripts/ci-preflight.sh [--fast|--full]" >&2; exit 2 ;;
esac
if (( $# > 1 )); then
  echo "Only one option is supported" >&2
  exit 2
fi

# Cargo fmt omits files brought in through include!; check them independently.
cargo fmt --all -- --check
rustfmt --check --edition 2021 \
  src/registry_snapshot_impl.rs src/execution_snapshot_impl.rs \
  src/ledger_channels_impl.rs src/ledger_settlement_impl.rs \
  src/spot_snapshot_impl.rs src/joint_snapshot_impl.rs

cargo clippy --all-targets --locked -- -D warnings
if [[ "$mode" == "--full" ]]; then
  cargo test --locked
  python3 scripts/audit_native_wallet_order.py
fi
echo "Rust CI preflight passed ($mode). GitHub still runs network, Lean and Kani gates."
