#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# FeatherMark Rutile Linux native gate (WebKitGTK).
#
# Builds the feathermark binary with the Linux product shell (--features
# linux-gtk) and runs the lifecycle harness under a PRIVATE, dynamically
# allocated Xvfb display and an isolated D-Bus session bus. It never attaches
# to an existing DISPLAY: DISPLAY is unset on entry and a fresh display number
# is selected by scanning for an absent X lock file.
#
# Gate-result emission is delegated to `xtask linux-gate`, the canonical
# rutile.gate-result.v1 producer for the Linux native gate. The xtask owns git
# provenance capture, artifact identity, bounded-log retention, and the
# schema-valid JSON document. This script only selects the build profile, the
# cycle floor, and the evidence directory; it never assembles JSON.
#
# Xvfb hardening (QA-004):
# - The Xvfb workspace is a dynamic private directory (mktemp -d) under the
#   evidence root, not a static /tmp path.
# - The blind `sleep 1` is replaced with a readiness wait that polls for the
#   X socket file (/tmp/.X11-unix/X<N>) up to 5 seconds.
# - The Xvfb log is retained to the evidence directory for post-run inspection.
#
# Evidence lands under
#   ${CARGO_TARGET_DIR:-target}/evidence/<commit>/<job>/run-<ms>-<pid>-<n>/
#
# Usage:
#   scripts/ci/linux-native-gate.sh --profile pr
#   scripts/ci/linux-native-gate.sh --profile release --cycles 50
set -euo pipefail

if [ "$(uname -s)" != "Linux" ]; then
  echo "linux-native-gate: requires Linux (this gate is linux-gtk only)" >&2
  exit 2
fi

# Never inherit a display from the host; this gate owns a private one.
unset DISPLAY

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

: "${CARGO_TARGET_DIR:=${REPO_ROOT}/target}"
export CARGO_TARGET_DIR
TARGET_DIR="$CARGO_TARGET_DIR"

command -v cargo >/dev/null 2>&1 || {
  if [ -f "$HOME/.cargo/env" ]; then source "$HOME/.cargo/env"; else
    echo "linux-native-gate: cargo not found" >&2; exit 2;
  fi
}

profile="pr"
cycles=""
job="linux-native-smoke"

usage() {
  cat >&2 <<EOF
usage: $0 --profile pr|release [--cycles COUNT] [--job NAME]
       --cycles defaults to the profile floor (pr=10, release=50) and may only raise it.
EOF
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --profile) profile="${2:-}"; shift 2 ;;
    --cycles) cycles="${2:-}"; shift 2 ;;
    --job) job="${2:-}"; shift 2 ;;
    --help|-h) usage ;;
    *) echo "linux-native-gate: unknown argument: $1" >&2; usage ;;
  esac
done

case "$profile" in
  pr) minimum=10 ;;
  release) minimum=50 ;;
  *) echo "linux-native-gate: --profile must be pr or release" >&2; exit 2 ;;
esac

if [ -z "$cycles" ]; then
  cycles="$minimum"
fi
case "$cycles" in
  *[!0-9]*|'') echo "linux-native-gate: --cycles must be a positive integer" >&2; exit 2 ;;
esac
if [ "$cycles" -lt "$minimum" ]; then
  echo "linux-native-gate: --profile $profile requires --cycles >= $minimum" >&2
  exit 2
fi

for dep in Xvfb dbus-run-session; do
  command -v "$dep" >/dev/null 2>&1 || {
    echo "linux-native-gate: required tool missing: $dep" >&2
    exit 2
  }
done

# --- helpers -----------------------------------------------------------------

now_ms() { date +%s%3N; }
git_commit() { git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo "0000000000000000000000000000000000000000"; }

# --- re-exec under an isolated D-Bus session ---------------------------------
if [ "${FEATHERMARK_LINUX_GATE_ISOLATED:-0}" != "1" ]; then
  export FEATHERMARK_LINUX_GATE_ISOLATED=1
  exec dbus-run-session -- "$0" "$@"
fi

# --- create evidence job directory --------------------------------------------
commit="$(git_commit)"
job_dir="${TARGET_DIR}/evidence/${commit}/${job}"
mkdir -p "$job_dir"

# --- allocate a private Xvfb display (hardened: QA-004) ----------------------
# Dynamic private workspace under the evidence root (not a static /tmp path).
# Readiness wait replaces the blind sleep: poll for the X socket file.
XVFB_PID=""
ALLOCATED_DISPLAY=""
xvfb_workspace="$(mktemp -d "${job_dir}/.xvfb-XXXXXX")"
xvfb_log="${xvfb_workspace}/xvfb.log"

cleanup() {
  if [ -n "$XVFB_PID" ]; then
    kill "$XVFB_PID" 2>/dev/null || true
    wait "$XVFB_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

for candidate in $(seq 100 199); do
  if [ -e "/tmp/.X${candidate}-lock" ] || [ -e "/tmp/.X11-unix/X${candidate}" ]; then
    continue
  fi
  Xvfb ":${candidate}" -screen 0 1280x720x24 +extension GLX +extension RANDR +render -nolisten tcp -noreset \
    >"$xvfb_log" 2>&1 &
  XVFB_PID=$!
  # Readiness wait: poll for the X socket file up to 5 seconds (50 x 100ms).
  display_ready=false
  for _ in $(seq 1 50); do
    if ! kill -0 "$XVFB_PID" 2>/dev/null; then
      break  # Xvfb died before claiming the display
    fi
    if [ -e "/tmp/.X11-unix/X${candidate}" ]; then
      display_ready=true
      break
    fi
    sleep 0.1
  done
  if $display_ready; then
    ALLOCATED_DISPLAY=":${candidate}"
    break
  fi
  # Display was contended or Xvfb failed; reap and keep scanning.
  kill "$XVFB_PID" 2>/dev/null || true
  wait "$XVFB_PID" 2>/dev/null || true
  XVFB_PID=""
done

if [ -z "$ALLOCATED_DISPLAY" ]; then
  echo "linux-native-gate: could not allocate a private Xvfb display" >&2
  echo "linux-native-gate: xvfb log retained at ${xvfb_log}" >&2
  exit 2
fi
export DISPLAY="$ALLOCATED_DISPLAY"
echo "linux-native-gate: private display=${DISPLAY} pid=${XVFB_PID}"
echo "linux-native-gate: xvfb log=${xvfb_log}"

# --- build + run the lifecycle harness ---------------------------------------
build_profile="debug"
bin_path="${TARGET_DIR}/debug/feathermark"
if [ "$profile" = "release" ]; then
  build_profile="release"
  bin_path="${TARGET_DIR}/release/feathermark"
fi

echo "=== linux-native-gate: cargo build (${build_profile}, linux-gtk) ==="
cargo build --locked -p feathermark-app --features linux-gtk --bin feathermark

if [ ! -x "$bin_path" ]; then
  echo "linux-native-gate: expected binary not found at ${bin_path}" >&2
  exit 2
fi

stdout_raw="$(mktemp)"
stderr_raw="$(mktemp)"
trap 'cleanup; rm -f "$stdout_raw" "$stderr_raw"' EXIT

started_ms="$(now_ms)"

echo "=== linux-native-gate: lifecycle (${cycles} cycles, DISPLAY=${DISPLAY}) ==="
set +e
bash "${REPO_ROOT}/scripts/feathermark-linux-lifecycle.sh" \
  --cycles "$cycles" \
  --display "$DISPLAY" \
  --binary "$bin_path" \
  > >(tee "$stdout_raw") 2> >(tee "$stderr_raw" >&2)
harness_code=$?
set -e

ended_ms="$(now_ms)"

# --- emit schema-valid gate-result.v1 via xtask ------------------------------
# The xtask is the authoritative rutile.gate-result.v1 producer: it captures
# git provenance, artifact identity, bounds retained logs to 16 KiB, creates a
# distinct evidence run directory, and writes the document atomically
# (fail-closed: never overwrites, never fakes green).
echo "=== linux-native-gate: xtask linux-gate (rutile.gate-result.v1) ==="
set +e
cargo run --locked -p xtask --bin xtask -- linux-gate \
  --binary "$bin_path" \
  --profile "$profile" \
  --cycles "$cycles" \
  --exit-code "$harness_code" \
  --started-ms "$started_ms" \
  --ended-ms "$ended_ms" \
  --stdout-log "$stdout_raw" \
  --stderr-log "$stderr_raw" \
  --evidence-dir "$job_dir"
gate_code=$?
set -e

if [ "$gate_code" -ne 0 ]; then
  echo "linux-native-gate: gate failed (exit ${gate_code}); evidence retained under ${job_dir}" >&2
fi

exit "$gate_code"
