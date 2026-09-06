#!/usr/bin/env bash
set -euo pipefail

sdk_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$sdk_dir"

scripts/check-protocol-tags.sh
cargo fmt --all -- --check
cargo check --locked --no-default-features
cargo check --locked --features ws-client
cargo check --locked --features chain
cargo check --locked --features chain-rpc
cargo test --locked --all-features --no-fail-fast
cargo clippy --locked --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked --all-features --no-deps
cargo package --locked --allow-dirty --offline
