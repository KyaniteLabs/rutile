#!/usr/bin/env bash
set -euo pipefail

# Rutile Linux product-shell gate.
#
# Runs formatting, clippy, unit/integration tests, builds the release binary,
# and executes the 50-cycle lifecycle harness under an isolated Xvfb + D-Bus
# session. This script is intended for Niko / NUCBox x86_64 Linux.

if ! command -v cargo >/dev/null 2>&1; then
  if [[ -f "$HOME/.cargo/env" ]]; then
    # shellcheck source=/dev/null
    source "$HOME/.cargo/env"
  else
    echo "cargo not found and $HOME/.cargo/env is missing" >&2
    exit 2
  fi
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

: "${CARGO_TARGET_DIR:=${REPO_ROOT}/target}"
export CARGO_TARGET_DIR

echo "=== Rutile Linux gate ==="
echo "pwd=$(pwd)"
echo "rustc=$(rustc --version)"
echo "cargo=$(cargo --version)"

echo "=== cargo fmt --check ==="
cargo fmt --check

echo "=== cargo clippy (lib + tests) ==="
cargo clippy --locked -p rutile-app \
  --no-default-features --features linux-gtk,test-control \
  --lib --tests -- -D warnings

echo "=== start Xvfb ==="
DISPLAY_NUM=99
export DISPLAY=":${DISPLAY_NUM}"
Xvfb ":${DISPLAY_NUM}" -screen 0 1280x720x24 +extension GLX +extension RANDR +render -noreset &
XVFB_PID=$!
trap 'kill "${XVFB_PID}" 2>/dev/null || true' EXIT
sleep 2

echo "=== cargo test linux-gtk,test-control (DISPLAY=${DISPLAY}) ==="
cargo test --locked -p rutile-app \
  --no-default-features --features linux-gtk,test-control

echo "=== cargo build release linux-gtk,test-control ==="
cargo build --locked -p rutile-app --release \
  --no-default-features --features linux-gtk,test-control

echo "=== lifecycle smoke 50 cycles ==="
bash scripts/rutile-linux-lifecycle.sh \
  --cycles 50 \
  --display ":${DISPLAY_NUM}" \
  --binary "${CARGO_TARGET_DIR}/release/rutile"

echo "=== Linux gate passed ==="
