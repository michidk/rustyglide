#!/usr/bin/env bash
# The checks CI runs: `gates.sh [all|lints|test|gpu]`.
set -euo pipefail
cd "$(dirname "$0")/../.."
stage="${1:-all}"
case "$stage" in
    all|lints|test|gpu) ;;
    *) echo "usage: gates.sh [all|lints|test|gpu]" >&2; exit 2 ;;
esac
if [[ "$stage" == all || "$stage" == lints ]]; then
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    bash dev/scripts/check-core-boundary.sh
    if command -v cargo-deny >/dev/null; then cargo deny check; else echo "gates: cargo-deny not installed, skipped"; fi
fi
if [[ "$stage" == all || "$stage" == test ]]; then
    cargo test --workspace --all-features --locked
fi
if [[ "$stage" == all || "$stage" == gpu ]]; then
    # GPU tests on lavapipe in Docker when the host has no Vulkan driver.
    if [[ -n "${RUSTYGLIDE_REQUIRE_GPU:-}" ]]; then
        cargo test --workspace --all-features --locked
    else
        bash dev/scripts/gpu-test.sh
    fi
fi
echo "gates: green"
