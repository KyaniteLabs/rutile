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
# This gate emits its own rutile.gate-result.v1 document. It cannot delegate to
# `xtask native-smoke` because the Linux GTK adapter does not emit the
# `feathermark-native-smoke-ok` success marker the supervisor requires (only the
# macOS native adapter does). Wiring native-smoke marker support for Linux GTK
# is tracked as a Phase D dependency.
#
# Evidence lands under
#   ${CARGO_TARGET_DIR:-target}/evidence/<commit>/<job>/run-<ms>-<pid>-<n>/ .
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

now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
git_commit() { git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo "0000000000000000000000000000000000000000"; }
git_tree()   { git -C "$REPO_ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo "0000000000000000000000000000000000000000"; }
git_dirty() {
  if [ -n "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=all 2>/dev/null)" ]; then
    echo "true"
  else
    echo "false"
  fi
}
runner_arch() { uname -m; }
runner_name() {
  for v in FORGEJO_RUNNER_NAME RUNNER_NAME HOSTNAME; do
    if [ -n "${!v:-}" ]; then echo "${!v}"; return; fi
  done
  echo "local"
}

# emit_gate_result: see portable-gate.sh for the full contract. Assembles and
# writes a rutile.gate-result.v1 document; fails closed on schema invariants.
emit_gate_result() {
  python3 - "$@" <<'PY'
import argparse, hashlib, json, os, sys

p = argparse.ArgumentParser()
p.add_argument("--command-id", required=True)
p.add_argument("--profile", required=True)
p.add_argument("--required-row", required=True)
p.add_argument("--exit-code", type=int, required=True)
p.add_argument("--started-ms", required=True)
p.add_argument("--ended-ms", required=True)
p.add_argument("--job-dir", required=True)
p.add_argument("--artifact", required=True)
p.add_argument("--stdout-log", required=True)
p.add_argument("--stderr-log", required=True)
p.add_argument("--tests-total", type=int, required=True)
p.add_argument("--tests-passed", type=int, required=True)
p.add_argument("--tests-failed", type=int, required=True)
p.add_argument("--tests-ignored", type=int, required=True)
p.add_argument("--tests-skipped", type=int, required=True)
p.add_argument("--runs-tsv", required=True)
p.add_argument("--arch", required=True)
p.add_argument("--runner-name", required=True)
p.add_argument("--commit", required=True)
p.add_argument("--tree", required=True)
p.add_argument("--dirty", required=True)
a = p.parse_args()

MAX = 16384
os.makedirs(a.job_dir, exist_ok=True)
pid = os.getpid()
counter = 0
while True:
    run_dir = os.path.join(a.job_dir, "run-{}-{}-{}".format(a.started_ms, pid, counter))
    try:
        os.mkdir(run_dir)
        break
    except FileExistsError:
        counter += 1

def retain(src, name, stream):
    data = b""
    if src and os.path.isfile(src):
        with open(src, "rb") as f:
            data = f.read(MAX)
    with open(os.path.join(run_dir, name), "wb") as f:
        f.write(data)
    return {"run": 1, "stream": stream, "path": name,
            "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}

retained = [
    retain(a.stdout_log, "run-0001.stdout.log", "stdout"),
    retain(a.stderr_log, "run-0001.stderr.log", "stderr"),
]

def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()

if not os.path.isfile(a.artifact):
    sys.stderr.write("emit_gate_result: artifact not found: {}\n".format(a.artifact))
    sys.exit(2)
st = os.stat(a.artifact)
artifact_hashes = [{
    "path": a.artifact,
    "sha256": sha256_file(a.artifact),
    "identity": {"device": int(st.st_dev), "inode": int(st.st_ino), "bytes": int(st.st_size)},
}]

runs = []
with open(a.runs_tsv) as f:
    for line in f:
        line = line.rstrip("\n")
        if not line:
            continue
        parts = line.split("\t")
        while len(parts) < 6:
            parts.append("")
        run_n, status, reaped, stg, rsz, err = parts[:6]
        if status not in ("passed", "failed"):
            sys.stderr.write("emit_gate_result: bad run status: {}\n".format(status))
            sys.exit(2)
        runs.append({
            "run": int(run_n),
            "status": status,
            "reaped": reaped == "1",
            "stage_traces": int(stg or 0),
            "resize_traces": int(rsz or 0),
            "error": (err or None),
        })

doc = {
    "schema": "rutile.gate-result.v1",
    "command_id": a.command_id,
    "profile": a.profile,
    "source": {"commit": a.commit, "tree": a.tree, "dirty": a.dirty == "true"},
    "evidence": {"run_directory": os.path.basename(run_dir)},
    "runner": {"platform": "linux", "architecture": a.arch, "name": a.runner_name},
    "started_unix_ms": int(a.started_ms),
    "ended_unix_ms": int(a.ended_ms),
    "exit_code": a.exit_code,
    "tests": {"total": a.tests_total, "passed": a.tests_passed,
              "failed": a.tests_failed, "ignored": a.tests_ignored,
              "skipped": a.tests_skipped},
    "required_row": {"name": a.required_row, "required": True,
                     "status": "passed" if a.exit_code == 0 else "failed"},
    "artifact_hashes": artifact_hashes,
    "retained_logs": retained,
    "runs": runs,
}

assert doc["tests"]["total"] >= 1, "tests.total must be >= 1"
assert len(doc["artifact_hashes"]) >= 1, "artifact_hashes must be non-empty"
assert len(doc["runs"]) >= 1, "runs must be non-empty"
for r in doc["retained_logs"]:
    assert 0 <= r["bytes"] <= MAX, "retained log exceeds 16 KiB"
for r in doc["runs"]:
    assert 0 <= r["stage_traces"] <= 64, "stage_traces out of range"
    assert 0 <= r["resize_traces"] <= 64, "resize_traces out of range"

out = os.path.join(run_dir, "gate-result.json")
tmp = out + ".tmp"
with open(tmp, "w") as f:
    json.dump(doc, f, indent=2)
    f.write("\n")
os.replace(tmp, out)
print(out)
PY
}

# --- re-exec under an isolated D-Bus session ---------------------------------
if [ "${FEATHERMARK_LINUX_GATE_ISOLATED:-0}" != "1" ]; then
  export FEATHERMARK_LINUX_GATE_ISOLATED=1
  exec dbus-run-session -- "$0" "$@"
fi

# --- allocate a private Xvfb display (never attach to an existing one) -------
XVFB_PID=""
ALLOCATED_DISPLAY=""

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
    >/tmp/feathermark-xvfb-${candidate}.log 2>&1 &
  XVFB_PID=$!
  # Give the server a moment to claim (or fail to claim) the display.
  sleep 1
  if kill -0 "$XVFB_PID" 2>/dev/null && [ -e "/tmp/.X11-unix/X${candidate}" ]; then
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
  exit 2
fi
export DISPLAY="$ALLOCATED_DISPLAY"
echo "linux-native-gate: private display=${DISPLAY} pid=${XVFB_PID}"

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

commit="$(git_commit)"
tree="$(git_tree)"
dirty="$(git_dirty)"
job_dir="${TARGET_DIR}/evidence/${commit}/${job}"
mkdir -p "$job_dir"

stdout_raw="$(mktemp)"
stderr_raw="$(mktemp)"
runs_tsv="$(mktemp)"
trap 'cleanup; rm -f "$stdout_raw" "$stderr_raw" "$runs_tsv"' EXIT

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

# Parse the harness summary line: "ready=N closed=N failures=N".
ready=0
closed=0
failures=0
summary="$(grep -Eo '^ready=[0-9]+ closed=[0-9]+ failures=[0-9]+$' "$stdout_raw" | tail -n 1 || true)"
if [ -n "$summary" ]; then
  ready="$(printf '%s' "$summary" | sed -n 's/^ready=\([0-9]*\) .*/\1/p')"
  closed="$(printf '%s' "$summary" | sed -n 's/.* closed=\([0-9]*\) .*/\1/p')"
  failures="$(printf '%s' "$summary" | sed -n 's/.* failures=\([0-9]*\)$/\1/p')"
fi

# The harness fails closed itself, but derive status independently too.
if [ "$harness_code" -ne 0 ] || [ "$failures" -ne 0 ] \
   || [ "$ready" -ne "$cycles" ] || [ "$closed" -ne "$cycles" ]; then
  final_exit=1
  run_status="failed"
  failed_count="$failures"
  if [ "$ready" -lt "$cycles" ]; then failed_count="$((cycles - ready))"; fi
  err_line="$(head -n 1 "$stderr_raw" | tr '\t\n' '  ' | cut -c1-160)"
else
  final_exit=0
  run_status="passed"
  failed_count=0
  err_line=""
fi

printf '1\t%s\t0\t0\t0\t%s\n' "$run_status" "$err_line" >>"$runs_tsv"

command_id="linux-${job}"
gate_path="$(emit_gate_result \
  --command-id "$command_id" \
  --profile "$profile" \
  --required-row "$command_id" \
  --exit-code "$final_exit" \
  --started-ms "$started_ms" \
  --ended-ms "$ended_ms" \
  --job-dir "$job_dir" \
  --artifact "$bin_path" \
  --stdout-log "$stdout_raw" \
  --stderr-log "$stderr_raw" \
  --tests-total "$cycles" \
  --tests-passed "$ready" \
  --tests-failed "$failed_count" \
  --tests-ignored 0 \
  --tests-skipped 0 \
  --runs-tsv "$runs_tsv" \
  --arch "$(runner_arch)" \
  --runner-name "$(runner_name)" \
  --commit "$commit" \
  --tree "$tree" \
  --dirty "$dirty")"

echo "linux-native-gate: gate-result=${gate_path}"
echo "linux-native-gate: cycles=${cycles} ready=${ready} closed=${closed} failures=${failures}"

exit "$final_exit"
