#!/usr/bin/env bash
set -euo pipefail

# FeatherMark Linux WebKitGTK lifecycle runner.
#
# Runs the FeatherMark GTK binary for N cycles under an isolated D-Bus session
# and a configured X11 display. Each cycle emits one JSON ready receipt and one
# JSON closed receipt. The final stdout line is:
#   ready=N closed=N failures=0
#
# Usage:
#   bash scripts/feathermark-linux-lifecycle.sh --cycles 50
#   bash scripts/feathermark-linux-lifecycle.sh --cycles 50 --display :1 --binary /path/to/feathermark

# Re-exec under an isolated D-Bus session unless already isolated.
if [[ "${FEATHERMARK_LIFECYCLE_ISOLATED:-}" != "1" ]]; then
  if ! command -v dbus-run-session >/dev/null 2>&1; then
    echo "dbus-run-session is required" >&2
    exit 2
  fi
  export FEATHERMARK_LIFECYCLE_ISOLATED=1
  exec dbus-run-session -- "$0" "$@"
fi

cycles=50
display="${DISPLAY:-}"
binary=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cycles)
      cycles="$2"
      shift 2
      ;;
    --display)
      display="$2"
      shift 2
      ;;
    --binary)
      binary="$2"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$display" ]]; then
  echo "DISPLAY is not set (pass --display or set the environment)" >&2
  exit 2
fi

if [[ -z "$binary" ]]; then
  binary="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/target/release/feathermark"
fi

if [[ ! -x "$binary" ]]; then
  echo "Binary not found or not executable: $binary" >&2
  exit 2
fi

export DISPLAY="$display"

ready=0
closed=0
failures=0

run_cycle() {
  local cycle="$1"
  local stdout_file stderr_file
  stdout_file=$(mktemp)
  stderr_file=$(mktemp)
  # shellcheck disable=SC2064
  trap "rm -f \"$stdout_file\" \"$stderr_file\"" RETURN

  local autoclose_ms=10000
  local deadline_secs=30
  local exit_code=0

  FEATHERMARK_LIFECYCLE_CYCLE="$cycle" \
  FEATHERMARK_SMOKE_AUTOCLOSE_MS="$autoclose_ms" \
    timeout --kill-after=5s "${deadline_secs}s" "$binary" >"$stdout_file" 2>"$stderr_file" || exit_code=$?

  if [[ $exit_code -eq 124 ]] || [[ $exit_code -eq 137 ]]; then
    echo "cycle $cycle: deadline exceeded (exit $exit_code)" >&2
    return 1
  fi

  if grep -q "FEATHERMARK_ACTIVATION_FAILED" "$stderr_file"; then
    echo "cycle $cycle: activation failure" >&2
    return 1
  fi

  local ready_line closed_line
  ready_line=$(grep -E '^\{"type":"ready","cycle":'"$cycle"'\}$' "$stdout_file" || true)
  closed_line=$(grep -E '^\{"type":"closed","cycle":'"$cycle"',"webview_first":true,"closed":true\}$' "$stdout_file" || true)

  if [[ -z "$ready_line" ]]; then
    echo "cycle $cycle: missing ready receipt" >&2
    return 1
  fi

  if [[ -z "$closed_line" ]]; then
    echo "cycle $cycle: missing closed receipt" >&2
    return 1
  fi

  local ready_count closed_count
  ready_count=$(grep -cE '^\{"type":"ready"' "$stdout_file" || true)
  closed_count=$(grep -cE '^\{"type":"closed"' "$stdout_file" || true)

  if [[ "$ready_count" -ne 1 ]] || [[ "$closed_count" -ne 1 ]]; then
    echo "cycle $cycle: extra receipts ready=$ready_count closed=$closed_count" >&2
    return 1
  fi

  return 0
}

for cycle in $(seq 1 "$cycles"); do
  if run_cycle "$cycle"; then
    ready=$((ready + 1))
    closed=$((closed + 1))
  else
    failures=$((failures + 1))
  fi
done

echo "ready=$ready closed=$closed failures=$failures"

if [[ "$failures" -ne 0 ]] || [[ "$ready" -ne "$cycles" ]] || [[ "$closed" -ne "$cycles" ]]; then
  exit 1
fi
