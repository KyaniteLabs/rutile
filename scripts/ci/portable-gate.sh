#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Rutile portable CI gate.
#
# Runs platform-agnostic Rutile gates (fmt / clippy(-D warnings) / docs /
# test, or a bounded libFuzzer smoke) and emits one rutile.gate-result.v1
# document for the whole invocation under
#   ${CARGO_TARGET_DIR:-target}/evidence/<commit>/<job>/run-<ms>-<pid>-<n>/
#
# The portable gate never enables a GUI feature (macos-shell / linux-gtk) and
# never enables test-control; those belong to the native gates and the
# production build respectively. Each selected stage becomes one entry in the
# gate-result `runs[]` array, so partial failures are recorded in full before
# the script fails closed.
#
# Usage:
#   scripts/ci/portable-gate.sh --stage lint,docs,test --profile pr
#   scripts/ci/portable-gate.sh --stage fuzz --profile pr
#
# Stages:
#   fmt   cargo fmt --all --check
#   clippy  cargo clippy ... -- -D warnings  (--workspace: all crates incl. spikes)
#   docs    cargo doc --no-deps              (core + protocol + types)
#   test    cargo test                       (core + protocol + types)
#   xtask-test  cargo test -p xtask --all-targets  (G002 keystone crate tests; runs whenever test runs)
#   build   cargo build --release            (production, no test-control, target/prod root)
#   deny    cargo deny check                 (workspace dependency policy, deny-warnings)
#   fuzz-deny  cargo deny check              (fuzz crate dependency policy, deny-warnings)
#   fuzz    libFuzzer smoke, 60s/target across every fuzz target
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

: "${CARGO_TARGET_DIR:=${REPO_ROOT}/target}"
export CARGO_TARGET_DIR
TARGET_DIR="$CARGO_TARGET_DIR"

profile="pr"
stages="lint,docs,test"
job="portable"
evidence_root="${TARGET_DIR}/evidence"
fuzz_max_total_time="60"
fuzz_toolchain="nightly-2026-07-01"
# Auto-discover fuzz targets from fuzz/fuzz_targets/*.rs (sorted for determinism).
mapfile -t fuzz_targets < <(find "$(dirname "$0")/../../fuzz/fuzz_targets" -name '*.rs' -exec basename {} .rs \; | sort)

usage() {
  cat >&2 <<EOF
usage: $0 [--stage lint,docs,test|fmt,clippy,docs,test|fuzz]
          [--profile pr|release] [--job NAME] [--evidence-root PATH]
          [--fuzz-max-total-time SECONDS]
EOF
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --stage) stages="${2:-}"; shift 2 ;;
    --profile) profile="${2:-}"; shift 2 ;;
    --job) job="${2:-}"; shift 2 ;;
    --evidence-root) evidence_root="${2:-}"; shift 2 ;;
    --fuzz-max-total-time) fuzz_max_total_time="${2:-}"; shift 2 ;;
    --help|-h) usage ;;
    *) echo "portable-gate: unknown argument: $1" >&2; usage ;;
  esac
done

case "$profile" in
  pr|release) ;;
  *) echo "portable-gate: --profile must be pr or release" >&2; exit 2 ;;
esac

# Expand the "lint" alias.
IFS=',' read -r -a stage_list <<<"$stages"
expanded=()
for s in "${stage_list[@]}"; do
  case "$s" in
    lint) expanded+=(fmt clippy) ;;
    test) expanded+=(test xtask-test) ;;
    fmt|clippy|docs|xtask-test|build|fuzz|deny|fuzz-deny) expanded+=("$s") ;;
    *) echo "portable-gate: unknown stage: $s" >&2; exit 2 ;;
  esac
done
# De-duplicate while preserving order.
stage_list=()
for s in "${expanded[@]}"; do
  skip=
  for seen in "${stage_list[@]}"; do [ "$seen" = "$s" ] && skip=1 && break; done
  [ -z "$skip" ] && stage_list+=("$s")
done
[ "${#stage_list[@]}" -gt 0 ] || { echo "portable-gate: no stages selected" >&2; exit 2; }

command -v cargo >/dev/null 2>&1 || {
  if [ -f "$HOME/.cargo/env" ]; then source "$HOME/.cargo/env"; else
    echo "portable-gate: cargo not found" >&2; exit 2;
  fi
}

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

runner_platform() {
  case "$(uname -s)" in
    Darwin) echo "macos" ;;
    Linux)  echo "linux" ;;
    *)      uname -s | tr '[:upper:]' '[:lower:]' ;;
  esac
}
runner_arch() { uname -m; }
runner_name() {
  for v in FORGEJO_RUNNER_NAME RUNNER_NAME HOSTNAME; do
    if [ -n "${!v:-}" ]; then echo "${!v}"; return; fi
  done
  echo "local"
}

# capture <out_log> <err_log> -- <cmd...>
# Runs the command, teeing stdout/stderr to the console and the log files, then
# exports CAPTURE_CODE with the command's exit status (never aborts the script).
CAPTURE_CODE=0
capture() {
  local out="$1" err="$2"; shift 3
  set +e
  "$@" > >(tee "$out") 2> >(tee "$err" >&2)
  CAPTURE_CODE=$?
  set -e
}

# emit_gate_result --flag ... : assembles and writes a rutile.gate-result.v1
# document. Each run is read from --runs-tsv (tab-separated:
# run<TAB>status<TAB>reaped(0|1)<TAB>stage_traces<TAB>resize_traces<TAB>error).
# Fails closed (non-zero) if the document would violate the schema invariants.
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
p.add_argument("--platform", required=True)
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

artifact_hashes = []
if not os.path.isfile(a.artifact):
    sys.stderr.write("emit_gate_result: artifact not found: {}\n".format(a.artifact))
    sys.exit(2)
st = os.stat(a.artifact)
artifact_hashes.append({
    "path": a.artifact,
    "sha256": sha256_file(a.artifact),
    "identity": {"device": int(st.st_dev), "inode": int(st.st_ino), "bytes": int(st.st_size)},
})

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
        err = err or None
        runs.append({
            "run": int(run_n),
            "status": status,
            "reaped": reaped == "1",
            "stage_traces": int(stg or 0),
            "resize_traces": int(rsz or 0),
            "error": err,
        })

doc = {
    "schema": "rutile.gate-result.v1",
    "command_id": a.command_id,
    "profile": a.profile,
    "source": {"commit": a.commit, "tree": a.tree, "dirty": a.dirty == "true"},
    "evidence": {"run_directory": os.path.basename(run_dir)},
    "runner": {"platform": a.platform, "architecture": a.arch, "name": a.runner_name},
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

# Fail closed on the load-bearing schema invariants.
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

# --- run stages --------------------------------------------------------------

commit="$(git_commit)"
tree="$(git_tree)"
dirty="$(git_dirty)"
job_dir="${evidence_root}/${commit}/${job}"
mkdir -p "$job_dir"

stdout_raw="$(mktemp)"
stderr_raw="$(mktemp)"
runs_tsv="$(mktemp)"
trap 'rm -f "$stdout_raw" "$stderr_raw" "$runs_tsv"' EXIT

started_ms="$(now_ms)"
run_n=0
tests_passed=0
tests_failed=0
overall_failed=0

# record_stage <label> <code> <stage_out> <stage_err>
# Appends one honest run row and folds the bounded stage logs into the combined
# job logs (the 16 KiB cap is applied at emit time).
record_stage() {
  local label="$1" code="$2" stage_out="$3" stage_err="$4"
  run_n=$((run_n + 1))
  { head -c 16384 "$stage_out"; } >>"$stdout_raw" 2>/dev/null || true
  { head -c 16384 "$stage_err"; } >>"$stderr_raw" 2>/dev/null || true
  local status err_line
  if [ "$code" -eq 0 ]; then
    status="passed"; tests_passed=$((tests_passed + 1))
    err_line=""
  else
    status="failed"; tests_failed=$((tests_failed + 1)); overall_failed=1
    err_line="$(head -n 1 "$stage_err" | tr '\t\n' '  ' | cut -c1-160)"
  fi
  printf '%s\t%s\t0\t0\t0\t%s\n' "$run_n" "$status" "$err_line" >>"$runs_tsv"
  rm -f "$stage_out" "$stage_err"
}

# run_stage <label> -- <cmd...>: capture a command and record its real outcome.
run_stage() {
  local label="$1"; shift  # shift past the "--" separator is done by capture
  local stage_out stage_err code
  stage_out="$(mktemp)"
  stage_err="$(mktemp)"
  echo "=== portable/${label} ==="
  capture "$stage_out" "$stage_err" -- "$@"
  code="$CAPTURE_CODE"
  record_stage "$label" "$code" "$stage_out" "$stage_err"
}

# run_deny_stage <label> <manifest-path|->  -- cargo-deny with deny-warnings.
# cargo-deny exits non-zero only on `deny`-level findings; this gate additionally
# fails closed on any warn-level finding line so "deny warnings" is enforced
# without mutating the shared deny.toml (which this gate does not own).
run_deny_stage() {
  local label="$1" manifest="${2:-}"
  local stage_out stage_err code
  stage_out="$(mktemp)"
  stage_err="$(mktemp)"
  echo "=== portable/${label} ==="
  command -v cargo-deny >/dev/null 2>&1 || {
    record_stage "$label" 2 "$stage_out" "$stage_err"
    echo "portable-gate: cargo-deny not installed" >"$stage_err"
    return
  }
  if [ -n "$manifest" ]; then
    capture "$stage_out" "$stage_err" -- cargo deny --manifest-path "$manifest" check
  else
    capture "$stage_out" "$stage_err" -- cargo deny check
  fi
  code="$CAPTURE_CODE"
  if [ "$code" -eq 0 ] && grep -Eiq '(^|[^[:alpha:]])warning([^[:alpha:]]|$)' "$stage_out" "$stage_err" 2>/dev/null; then
    code=1
    echo "portable-gate: cargo-deny emitted a warn-level finding (deny-warnings)" >>"$stage_err"
  fi
  record_stage "$label" "$code" "$stage_out" "$stage_err"
}

for stage in "${stage_list[@]}"; do
  case "$stage" in
    fmt)
      run_stage fmt cargo fmt --all --check
      ;;
    clippy)
      run_stage clippy cargo clippy --locked --workspace --all-targets -- -D warnings
      ;;
    docs)
      run_stage docs cargo doc --locked --no-deps \
        -p rutile-types -p rutile-core -p rutile-protocol
      ;;
    test)
      run_stage test cargo test --locked \
        -p rutile-types -p rutile-core -p rutile-protocol
      ;;
    xtask-test)
      # G002 keystone crate (evidence_bind, package_smoke, readiness_keystone)
      # under locked all-targets coverage. Tests use in-process fakes (no native
      # shell deps) so this stays portable alongside the types/core/protocol row.
      # Serial (RUST_TEST_THREADS=1): the native-probe prompt-reaping tests
      # assert a real 10s NATIVE_PROBE_PROMPT_BOUND that parallel test threads
      # can starve under the full gate's CPU load. Serial execution removes
      # intra-suite contention WITHOUT weakening the safety constant.
      run_stage xtask-test env RUST_TEST_THREADS=1 cargo test --locked -p xtask --all-targets
      ;;
    build)
      # Production build: no GUI feature and no test-control, in a separate
      # target root (target/prod) so test scaffolding can never contaminate the
      # shipped artifact. The produced binary feeds the package job.
      run_stage build env CARGO_TARGET_DIR="${TARGET_DIR}/prod" \
        cargo build --locked --release -p rutile-app --bin rutile
      ;;
    deny)
      run_deny_stage deny ""
      ;;
    fuzz-deny)
      run_deny_stage fuzz-deny "${REPO_ROOT}/fuzz/Cargo.toml"
      ;;
    fuzz)
      if ! rustup toolchain list 2>/dev/null | grep -q "$fuzz_toolchain"; then
        echo "portable-gate: fuzz stage requires rustup toolchain $fuzz_toolchain" >&2
        exit 2
      fi
      if ! command -v cargo-fuzz >/dev/null 2>&1; then
        echo "portable-gate: cargo-fuzz not installed" >&2
        exit 2
      fi
      for target in "${fuzz_targets[@]}"; do
        # -max_total_time bounds each target; the harness is the oracle.
        run_stage "fuzz:${target}" cargo "+$fuzz_toolchain" fuzz run "$target" -- \
          "-max_total_time=${fuzz_max_total_time}"
      done
      ;;
  esac
done

ended_ms="$(now_ms)"

# When every selected stage was skipped this would be 0; force at least one run.
[ "$run_n" -ge 1 ] || { echo "portable-gate: no stages executed" >&2; exit 2; }

final_exit=0
[ "$overall_failed" -eq 0 ] || final_exit=1

command_id="portable-${job}"
gate_path="$(emit_gate_result \
  --command-id "$command_id" \
  --profile "$profile" \
  --required-row "$command_id" \
  --exit-code "$final_exit" \
  --started-ms "$started_ms" \
  --ended-ms "$ended_ms" \
  --job-dir "$job_dir" \
  --artifact "Cargo.lock" \
  --stdout-log "$stdout_raw" \
  --stderr-log "$stderr_raw" \
  --tests-total "$run_n" \
  --tests-passed "$tests_passed" \
  --tests-failed "$tests_failed" \
  --tests-ignored 0 \
  --tests-skipped 0 \
  --runs-tsv "$runs_tsv" \
  --platform "$(runner_platform)" \
  --arch "$(runner_arch)" \
  --runner-name "$(runner_name)" \
  --commit "$commit" \
  --tree "$tree" \
  --dirty "$dirty")"

echo "portable-gate: gate-result=${gate_path}"
echo "portable-gate: stages=${stage_list[*]} passed=${tests_passed} failed=${tests_failed}"

exit "$final_exit"
