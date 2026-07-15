#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# FeatherMark Rutile evidence finalizer.
#
# Scans ${CARGO_TARGET_DIR:-target}/evidence/<commit>/ for every
# rutile.gate-result.v1 document produced by the verify/release run, validates
# each against the load-bearing schema invariants (required fields present,
# artifact_hashes and runs non-empty, retained logs within the 16 KiB bound),
# and enforces fail-on-skip for the required job set: any required gate that is
# missing, failed, or structurally invalid fails the whole run closed.
#
# It then writes a plain evidence index (a list of the discovered gate-results
# with their sha256) and emits its own rutile.gate-result.v1 attesting that
# every required gate passed. Binding this index into the canonical
# rutile.evidence-index.v1 schema is a Phase B wiring step.
#
# Usage:
#   scripts/ci/evidence-finalize.sh
#   scripts/ci/evidence-finalize.sh --required portable,fuzz-smoke,macos-native-smoke
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

: "${CARGO_TARGET_DIR:=${REPO_ROOT}/target}"
TARGET_DIR="$CARGO_TARGET_DIR"
evidence_root="${TARGET_DIR}/evidence"
commit=""
job="evidence-finalize"
required=""
fail_on_dirty="false"

usage() {
  cat >&2 <<EOF
usage: $0 [--commit SHA] [--evidence-root PATH] [--job NAME]
          [--required comma,of,job,names] [--fail-on-dirty]
EOF
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --commit) commit="${2:-}"; shift 2 ;;
    --evidence-root) evidence_root="${2:-}"; shift 2 ;;
    --job) job="${2:-}"; shift 2 ;;
    --required) required="${2:-}"; shift 2 ;;
    --fail-on-dirty) fail_on_dirty="true"; shift ;;
    --help|-h) usage ;;
    *) echo "evidence-finalize: unknown argument: $1" >&2; usage ;;
  esac
done

command -v python3 >/dev/null 2>&1 || { echo "evidence-finalize: python3 required" >&2; exit 2; }

if [ -z "$commit" ]; then
  commit="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || echo "0000000000000000000000000000000000000000")"
fi
tree="$(git -C "$REPO_ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo "0000000000000000000000000000000000000000")"
if [ "$fail_on_dirty" = "true" ] && [ -n "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=all 2>/dev/null)" ]; then
  dirty="true"
else
  dirty="false"
fi

# --- helpers -----------------------------------------------------------------

now_ms() { python3 -c 'import time; print(int(time.time()*1000))'; }
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

# emit_gate_result: see portable-gate.sh for the full contract.
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

job_dir="${evidence_root}/${commit}/${job}"
mkdir -p "$job_dir"

stdout_raw="$(mktemp)"
stderr_raw="$(mktemp)"
runs_tsv="$(mktemp)"
index_json="${job_dir}/evidence-index.json"
trap 'rm -f "$stdout_raw" "$stderr_raw" "$runs_tsv"' EXIT

started_ms="$(now_ms)"

# Validate every gate-result and build the index in one python pass. A gate is
# "present" when its job directory holds a run-*/gate-result.json; it "passes"
# only when structurally valid AND exit_code == 0 AND required_row.status ==
# "passed". Missing required jobs are recorded as failed (fail-on-skip).
python3 - "$evidence_root" "$commit" "$index_json" "$stdout_raw" "$stderr_raw" "$runs_tsv" "$required" <<'PY'
import json, os, sys, glob, hashlib

evidence_root, commit, index_json, stdout_log, stderr_log, runs_tsv, required = sys.argv[1:8]
required_jobs = [j for j in required.split(",") if j]

commit_dir = os.path.join(evidence_root, commit)
discovered = {}  # job -> {path, sha256, status, error}

def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()

# Collect the newest gate-result per job directory directly under <commit>/.
for entry in sorted(os.listdir(commit_dir)) if os.path.isdir(commit_dir) else []:
    job_dir_child = os.path.join(commit_dir, entry)
    if not os.path.isdir(job_dir_child) or entry == os.path.basename(os.path.dirname(index_json)):
        continue
    runs = sorted(glob.glob(os.path.join(job_dir_child, "run-*", "gate-result.json")))
    if not runs:
        continue
    path = runs[-1]
    sha = sha256_file(path)
    status = "failed"
    err = "unread"
    try:
        with open(path) as f:
            doc = json.load(f)
        if doc.get("schema") != "rutile.gate-result.v1":
            err = "schema-mismatch:%s" % doc.get("schema")
        else:
            rr = doc.get("required_row", {})
            # Structural invariants.
            assert len(doc.get("artifact_hashes", [])) >= 1, "artifact_hashes empty"
            assert len(doc.get("runs", [])) >= 1, "runs empty"
            for r in doc.get("retained_logs", []):
                assert 0 <= int(r.get("bytes", -1)) <= 16384, "log>16KiB"
            assert rr.get("required") is True, "required_row not required"
            if int(doc.get("exit_code", 1)) == 0 and rr.get("status") == "passed":
                status = "passed"
                err = ""
            else:
                err = "exit=%s row=%s" % (doc.get("exit_code"), rr.get("status"))
    except Exception as e:
        status = "failed"
        err = "invalid:%s" % type(e).__name__
    discovered[entry] = {"path": os.path.relpath(path, evidence_root),
                         "sha256": sha, "status": status, "error": err}

# Fail-on-skip: required jobs that were not discovered count as failed.
rows = []
for j in required_jobs:
    if j not in discovered:
        discovered[j] = {"path": None, "sha256": None, "status": "failed",
                         "error": "missing-required"}
run_n = 0
passed = 0
failed = 0
for name in sorted(discovered):
    run_n += 1
    info = discovered[name]
    if info["status"] == "passed":
        passed += 1
    else:
        failed += 1
    rows.append((run_n, info["status"], info["error"] or ""))

with open(index_json, "w") as f:
    json.dump({
        "commit": commit,
        "gate_count": len(discovered),
        "passed": passed,
        "failed": failed,
        "required_jobs": required_jobs,
        "gates": [{"job": k, **discovered[k]} for k in sorted(discovered)],
    }, f, indent=2)
    f.write("\n")

with open(runs_tsv, "a") as f:
    for run_n, status, err in rows:
        f.write("%d\t%s\t0\t0\t0\t%s\n" % (run_n, status, err[:160]))
with open(stdout_log, "a") as f:
    f.write("validated %d gates (%d passed, %d failed)\n" % (len(discovered), passed, failed))
    for k in sorted(discovered):
        f.write("%s status=%s error=%s\n" % (k, discovered[k]["status"], discovered[k]["error"]))

# Exit 0 only if every discovered+required gate passed.
sys.exit(0 if failed == 0 else 1)
PY
validate_code=$?

ended_ms="$(now_ms)"

final_exit=0
[ "$validate_code" -eq 0 ] || final_exit=1

command_id="evidence-${job}"
gate_path="$(emit_gate_result \
  --command-id "$command_id" \
  --profile "release" \
  --required-row "$command_id" \
  --exit-code "$final_exit" \
  --started-ms "$started_ms" \
  --ended-ms "$ended_ms" \
  --job-dir "$job_dir" \
  --artifact "$index_json" \
  --stdout-log "$stdout_raw" \
  --stderr-log "$stderr_raw" \
  --tests-total "$(wc -l <"$runs_tsv" | tr -d ' ')" \
  --tests-passed "$(awk -F'\t' '$2=="passed"{c++}END{print c+0}' "$runs_tsv")" \
  --tests-failed "$(awk -F'\t' '$2=="failed"{c++}END{print c+0}' "$runs_tsv")" \
  --tests-ignored 0 \
  --tests-skipped 0 \
  --runs-tsv "$runs_tsv" \
  --platform "$(runner_platform)" \
  --arch "$(runner_arch)" \
  --runner-name "$(runner_name)" \
  --commit "$commit" \
  --tree "$tree" \
  --dirty "$dirty")" || final_exit=1

echo "evidence-finalize: index=${index_json}"
echo "evidence-finalize: gate-result=${gate_path}"
echo "evidence-finalize: exit=${final_exit}"
exit "$final_exit"
