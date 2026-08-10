#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Rutile macOS native smoke gate.
#
# Builds the rutile binary with the macOS product shell (--features
# macos-shell) and runs the externally supervised native-smoke gate through
# `xtask native-smoke`, which is the canonical rutile.gate-result.v1 producer
# for supervised native smoke. The xtask owns per-run supervision, success
# marker verification, artifact identity capture, and the 16 KiB retained-log
# bound; this script only selects the build profile, the repeat floor, and the
# evidence directory so the result lands under
#   ${CARGO_TARGET_DIR:-target}/evidence/<commit>/<job>/run-<ms>-<pid>-<n>/ .
#
# The macOS native adapter is the only platform path that emits the
# `rutile-native-smoke-ok` success marker the supervisor requires, so this
# gate is macOS-only by construction (Linux uses scripts/ci/linux-native-gate.sh).
#
# Usage:
#   scripts/ci/macos-native-gate.sh --profile pr
#   scripts/ci/macos-native-gate.sh --profile release --repeat 50
set -euo pipefail

if [ "$(uname -s)" != "Darwin" ]; then
  echo "macos-native-gate: requires Darwin (this gate is macOS-only)" >&2
  exit 2
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

: "${CARGO_TARGET_DIR:=${REPO_ROOT}/target}"
export CARGO_TARGET_DIR
TARGET_DIR="$CARGO_TARGET_DIR"

command -v cargo >/dev/null 2>&1 || {
  if [ -f "$HOME/.cargo/env" ]; then source "$HOME/.cargo/env"; else
    echo "macos-native-gate: cargo not found" >&2; exit 2;
  fi
}

profile="pr"
repeat=""
job="macos-native-smoke"

usage() {
  cat >&2 <<EOF
usage: $0 --profile pr|release [--repeat COUNT] [--job NAME]
       --repeat defaults to the profile floor (pr=10, release=50) and may only raise it.
EOF
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --profile) profile="${2:-}"; shift 2 ;;
    --repeat) repeat="${2:-}"; shift 2 ;;
    --job) job="${2:-}"; shift 2 ;;
    --help|-h) usage ;;
    *) echo "macos-native-gate: unknown argument: $1" >&2; usage ;;
  esac
done

case "$profile" in
  pr) minimum=10 ;;
  release) minimum=50 ;;
  *) echo "macos-native-gate: --profile must be pr or release" >&2; exit 2 ;;
esac

if [ -z "$repeat" ]; then
  repeat="$minimum"
fi
case "$repeat" in
  *[!0-9]*|'') echo "macos-native-gate: --repeat must be a positive integer" >&2; exit 2 ;;
esac
if [ "$repeat" -lt "$minimum" ]; then
  echo "macos-native-gate: --profile $profile requires --repeat >= $minimum" >&2
  exit 2
fi

commit="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo "0000000000000000000000000000000000000000")"
evidence_dir="${TARGET_DIR}/evidence/${commit}/${job}"
mkdir -p "$evidence_dir"

# Build the native binary. pr smokes the debug artifact (fast feedback);
# release exercises the shipped optimization profile (panic=abort, thin LTO).
build_profile="debug"
bin_path="${TARGET_DIR}/debug/rutile"
if [ "$profile" = "release" ]; then
  build_profile="release"
  bin_path="${TARGET_DIR}/release/rutile"
fi

echo "=== macos-native-gate: cargo build (${build_profile}, macos-shell) ==="
if [ "$profile" = "release" ]; then
  cargo build --locked --release -p rutile-app --features macos-shell --bin rutile
else
  cargo build --locked -p rutile-app --features macos-shell --bin rutile
fi

if [ ! -x "$bin_path" ]; then
  echo "macos-native-gate: expected binary not found at ${bin_path}" >&2
  exit 2
fi

echo "=== macos-native-gate: xtask native-smoke (profile=${profile} repeat=${repeat}) ==="
# xtask native-smoke emits rutile.gate-result.v1 beneath --evidence-dir and
# exits non-zero (fail closed) if any required run does not pass.
cargo run --locked -p xtask --bin xtask -- native-smoke \
  --binary "$bin_path" \
  --profile "$profile" \
  --repeat "$repeat" \
  --evidence-dir "$evidence_dir"

echo "=== macos-native-gate: idle RSS/CPU soak (180 seconds) ==="
python3 scripts/ci/macos-idle-soak.py --binary "$bin_path"
