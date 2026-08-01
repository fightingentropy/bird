#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 scripts/vendor-supply-chain.py --check-sbom
bash -n scripts/*.sh
bash scripts/test-installer.sh
cargo build --locked --release -p bird-cli --bin bird
./target/release/bird --version
./target/release/bird transport --json

echo "bird unified check passed"
