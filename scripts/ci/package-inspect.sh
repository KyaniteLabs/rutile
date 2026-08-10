#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Rutile package + inspect gate.
#
# Wraps the real xtask packaging and artifact-inspection surface for one
# platform kind and emits one rutile.gate-result.v1 document for the whole
# job. The pipeline is:
#
#   1. xtask package local <kind>     -> package artifacts + manifests JSON
#      (manifests JSON carries packaged_executable_sha256 per artifact)
#   2. xtask artifact inspect          -> candidate-mode quarantine/leak audit
#      (--mode candidate only; package-mode publication_authorized is
#      permanently false in this pipeline — no keystone or owner approval
#      path exists here to authorize publication)
#   3. best-effort dependency SBOM     -> CycloneDX if cargo-cyclonedx is
#      installed, otherwise a Cargo.lock-derived SBOM (real data, not a stub)
#
#   4. xtask package smoke-row        -> rutile.package-smoke-row.v1 receipt
#      per produced artifact proving real install/open/uninstall (never
#      planned-only). Platform-tool absence or kind/platform mismatch fails
#      closed inside the command and is recorded as a failed run row.
#
# Provenance binding (`xtask evidence bind`) is owned by evidence-finalize.sh,
# which holds the plain gate index. This gate never authorizes publication:
# publication_authorized is permanently false; no keystone or owner approval
# path is wired here to change that.
#
# Usage:
#   scripts/ci/package-inspect.sh --candidate target/prod/rutile \
#       --kind macos --version 0.2.0 --profile pr
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

: "${CARGO_TARGET_DIR:=${REPO_ROOT}/target}"
export CARGO_TARGET_DIR
TARGET_DIR="$CARGO_TARGET_DIR"

command -v cargo >/dev/null 2>&1 || {
  if [ -f "$HOME/.cargo/env" ]; then source "$HOME/.cargo/env"; else
    echo "package-inspect: cargo not found" >&2; exit 2;
  fi
}

candidate=""
kind=""
version=""
profile="pr"
job="package-inspect"
evidence_root="${TARGET_DIR}/evidence"

usage() {
  cat >&2 <<EOF
usage: $0 --candidate PATH [--kind macos|linux] [--version X.Y.Z]
          [--profile pr|release] [--job NAME]
EOF
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --candidate) candidate="${2:-}"; shift 2 ;;
    --kind) kind="${2:-}"; shift 2 ;;
    --version) version="${2:-}"; shift 2 ;;
    --profile) profile="${2:-}"; shift 2 ;;
    --job) job="${2:-}"; shift 2 ;;
    --evidence-root) evidence_root="${2:-}"; shift 2 ;;
    --help|-h) usage ;;
    *) echo "package-inspect: unknown argument: $1" >&2; usage ;;
  esac
done

[ -n "$candidate" ] || { echo "package-inspect: --candidate is required" >&2; exit 2; }
[ -x "$candidate" ] || { echo "package-inspect: candidate not executable: $candidate" >&2; exit 2; }
case "$profile" in
  pr|release) ;;
  *) echo "package-inspect: --profile must be pr or release" >&2; exit 2 ;;
esac

if [ -z "$kind" ]; then
  case "$(uname -s)" in
    Darwin) kind="macos" ;;
    Linux)  kind="linux" ;;
    *) echo "package-inspect: cannot infer --kind on $(uname -s)" >&2; exit 2 ;;
  esac
fi
case "$kind" in
  macos|linux) ;;
  *) echo "package-inspect: --kind must be macos or linux" >&2; exit 2 ;;
esac

if [ -z "$version" ]; then
  version="$(grep -m1 '^version' "${REPO_ROOT}/crates/rutile-app/Cargo.toml" \
    | sed -n 's/^version *= *"\([^"]*\)".*/\1/p' || true)"
  [ -n "$version" ] || version="0.2.0"
fi

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
sha256_arg() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
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

commit="$(git_commit)"
tree="$(git_tree)"
dirty="$(git_dirty)"
job_dir="${evidence_root}/${commit}/${job}"
mkdir -p "$job_dir"

stdout_raw="$(mktemp)"
stderr_raw="$(mktemp)"
runs_tsv="$(mktemp)"
trap 'rm -f "$stdout_raw" "$stderr_raw" "$runs_tsv"' EXIT

output_root="${job_dir}/packages/${kind}"
build_input_sha256="$(sha256_arg "$candidate")"
source_commit="$commit"

started_ms="$(now_ms)"
run_n=0
tests_passed=0
tests_failed=0
overall_failed=0
outstanding=""

record() {
  # record <label> <exit-code> <err-first-line>
  run_n=$((run_n + 1))
  if [ "$2" -eq 0 ]; then
    tests_passed=$((tests_passed + 1))
    printf '%s\tpassed\t0\t0\t0\t\n' "$run_n" >>"$runs_tsv"
  else
    tests_failed=$((tests_failed + 1)); overall_failed=1
    printf '%s\tfailed\t0\t0\t0\t%s\n' "$run_n" "${3:-}" >>"$runs_tsv"
  fi
}

echo "=== package-inspect: xtask package local ${kind} (version=${version}) ==="
set +e
cargo run --locked -p xtask --bin xtask -- package local "$kind" \
  --candidate "$candidate" \
  --build-input-sha256 "$build_input_sha256" \
  --source-commit "$source_commit" \
  --output-root "$output_root" \
  --version "$version" \
  > >(tee "$stdout_raw") 2> >(tee "$stderr_raw" >&2)
pkg_code=$?
set -e
record "package-local:${kind}" "$pkg_code" "$(head -n 1 "$stderr_raw" | tr '\t\n' '  ' | cut -c1-160)"

# Persist the manifests JSON the packager emitted (already on stdout_raw head).
if [ -d "$output_root" ]; then
  cp "$stdout_raw" "${output_root}.manifests.stdout.log" 2>/dev/null || true
fi

# Candidate-mode leak audit over the whole package tree. Package-mode
# publication authorization is permanently out of scope: publication_authorized
# is bound false and no path in this pipeline can change that. The candidate
# audit quarantines and reports leaks without authorizing release.
echo "=== package-inspect: xtask artifact inspect (candidate leak audit) ==="
inspect_json="${job_dir}/artifact-inspection.json"
set +e
cargo run --locked -p xtask --bin xtask -- artifact inspect \
  --artifact "$output_root" \
  --mode candidate \
  > "$inspect_json" 2> >(tee -a "$stderr_raw" >&2)
inspect_code=$?
set -e
# Retain the machine-readable inspection alongside the gate-result.
{ head -c 16384 "$inspect_json"; } >>"$stdout_raw" 2>/dev/null || true
record "artifact-inspect:candidate" "$inspect_code" \
  "$(python3 -c 'import json,sys
try:
  d=json.load(open("'"$inspect_json"'"))
  print((d.get("findings") or [{}])[0].get("code","") or ("accepted=%s" % d.get("accepted")))
except Exception as e:
  print("inspect-unreadable")' 2>/dev/null | tr '\t\n' '  ' | cut -c1-160)"

echo "=== package-inspect: SBOM (best-effort) ==="
# cargo-cyclonedx is the canonical SBOM producer; it is not in the pinned
# build-plan tool list yet. When absent, fall back to a real Cargo.lock-derived
# CycloneDX document (derived from resolved data, never fabricated). The Phase B
# provenance keystone binds whichever SBOM is produced into release evidence.
sbom_json="${job_dir}/sbom.cdx.json"
sbom_code=0
if cargo cyclonedx --output-format json --output-path "$sbom_json" \
   --manifest-path "${REPO_ROOT}/crates/rutile-app/Cargo.toml" \
   >/dev/null 2>>"$stderr_raw"; then
  outstanding="${outstanding}cyclonedx-sbom-produced"
else
  python3 - "$build_input_sha256" "${REPO_ROOT}/Cargo.lock" "$sbom_json" <<'SBOM'
import json, sys, time
src, cargo_lock, out = sys.argv[1], sys.argv[2], sys.argv[3]
rows = []
name = version = None
with open(cargo_lock) as f:
    for line in f:
        s = line.strip()
        if s == "[[package]]":
            if name:
                rows.append({"name": name, "versionInfo": version})
            name = version = None
        elif s.startswith("name = "):
            name = s.split('"', 2)[1] if '"' in s else s.split()[-1]
        elif s.startswith("version = "):
            version = s.split('"', 2)[1] if '"' in s else s.split()[-1]
if name:
    rows.append({"name": name, "versionInfo": version})
doc = {
    "bomFormat": "CycloneDX",
    "specVersion": "1.4",
    "serialNumber": "urn:uuid:rutile-cargolock-fallback",
    "version": 1,
    "metadata": {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "tools": [{"vendor": "Rutile", "name": "cargo-lock-sbom-fallback", "version": "0.2.0"}],
        "component": {"type": "application", "name": "rutile", "version": "0.2.0",
                      "bom-ref": "pkg:cargo-lock@" + src[:12]},
    },
    "components": [{"type": "library", "name": r["name"],
                    "versionInfo": r["versionInfo"],
                    "bom-ref": "pkg:cargo/%s@%s" % (r["name"], r["versionInfo"])}
                   for r in rows],
}
with open(out, "w") as f:
    json.dump(doc, f, indent=2)
    f.write("\n")
SBOM
  outstanding="${outstanding}cargo-lock-sbom-fallback(pending-cyclonedx)"
fi
{ head -c 16384 "$sbom_json"; } >>"$stdout_raw" 2>/dev/null || true
record "sbom" "$sbom_code" ""

# install/open/uninstall smoke: one xtask package smoke-row invocation per
# produced package artifact. Each receipt is a create-only
# rutile.package-smoke-row.v1 document at a FIXED path
# <evidence_dir>/package-smoke-row.json. The smoke engine installs the package
# to the real platform footprint (NOT the candidate binary path), opens the
# installed binary, then uninstalls and verifies no residue.
#
# Per-kind install footprint (confirmed with the smoke engine owner):
#   macos-app-zip / macos-dmg:
#     install_target = /Applications/Rutile.app
#     binary_path    = /Applications/Rutile.app/Contents/MacOS/Rutile
#   linux-deb / linux-rpm:
#     install_target = /
#     binary_path    = /usr/bin/rutile
#   linux-tar-zst:
#     install_target = /opt/rutile
#     binary_path    = /opt/rutile/bin/rutile
#
# The expected executable SHA-256 is parsed from each artifact's sibling
# manifest (packaged_executable_sha256 field) written by `xtask package local`.
# The manifests JSON array was captured to ${output_root}.manifests.stdout.log
# at step 1; we parse it to build an artifact-basename → sha256 lookup. If a
# manifest is missing or the hash absent, that artifact's smoke-row fails
# closed (the engine rejects a missing/empty expected hash).
#
# Inputs:  $output_root                   - package artifacts + manifests log
#          $source_commit                 - git HEAD SHA-40
#          ${output_root}.manifests.stdout.log - JSON array of ArtifactManifest
# Artifacts: ${job_dir}/smoke/<smoke-kind>-<artifact>/package-smoke-row.json
echo "=== package-inspect: xtask package smoke-row (install/open/uninstall) ==="
smoke_root="${job_dir}/smoke"
mkdir -p "$smoke_root"
smoke_count=0
manifests_log="${output_root}.manifests.stdout.log"

# Build an artifact-basename → packaged_executable_sha256 lookup from the
# manifests JSON array captured at step 1. python3 is already required.
exec_sha_lookup="$(python3 - "$manifests_log" <<'PYLOOKUP'
import json, sys, os
path = sys.argv[1]
lookup = {}
if os.path.isfile(path):
    try:
        with open(path) as f:
            data = json.load(f)
        for entry in data:
            art = entry.get("artifact", "")
            sha = entry.get("packaged_executable_sha256", "")
            if art and sha:
                lookup[os.path.basename(art)] = sha
    except Exception:
        pass
for name, sha in sorted(lookup.items()):
    print("%s\t%s" % (name, sha))
PYLOOKUP
)"

if [ -d "$output_root" ]; then
  shopt -s nullglob
  for artifact in "$output_root"/*.app.zip "$output_root"/*.dmg \
                  "$output_root"/*.deb "$output_root"/*.rpm \
                  "$output_root"/*.tar.zst; do
    case "$artifact" in
      *.app.zip) smoke_kind="macos-app-zip"; install_target="/Applications/Rutile.app"; binary_path="/Applications/Rutile.app/Contents/MacOS/Rutile" ;;
      *.dmg)     smoke_kind="macos-dmg";     install_target="/Applications/Rutile.app"; binary_path="/Applications/Rutile.app/Contents/MacOS/Rutile" ;;
      *.deb)     smoke_kind="linux-deb";     install_target="/"; binary_path="/usr/bin/rutile" ;;
      *.rpm)     smoke_kind="linux-rpm";     install_target="/"; binary_path="/usr/bin/rutile" ;;
      *.tar.zst) smoke_kind="linux-tar-zst"; install_target="/opt/rutile"; binary_path="/opt/rutile/bin/rutile" ;;
      *) continue ;;
    esac
    artifact_sha="$(sha256_arg "$artifact")"
    artifact_base="$(basename "$artifact")"
    # Look up the packaged executable SHA-256 from the sibling manifest.
    expected_exec_sha="$(printf '%s\n' "$exec_sha_lookup" | awk -F'\t' -v a="$artifact_base" '$1==a{print $2; exit}')"
    artifact_stem="$(printf '%s' "$artifact_base" | tr -c 'A-Za-z0-9._-' '_')"
    smoke_ev_dir="${smoke_root}/${smoke_kind}-${artifact_stem}"
    mkdir -p "$smoke_ev_dir"
    set +e
    cargo run --locked -p xtask --bin xtask -- package smoke-row \
      --package "$artifact" \
      --kind "$smoke_kind" \
      --package-sha256 "$artifact_sha" \
      --expected-executable-sha256 "$expected_exec_sha" \
      --source-commit "$source_commit" \
      --install-target "$install_target" \
      --binary-path "$binary_path" \
      --evidence-dir "$smoke_ev_dir" \
      > >(tee -a "$stdout_raw") 2> >(tee -a "$stderr_raw" >&2)
    smoke_code=$?
    set -e
    record "package-smoke-row:${smoke_kind}" "$smoke_code" \
      "${smoke_ev_dir}/package-smoke-row.json"
    smoke_count=$((smoke_count + 1))
  done
  shopt -u nullglob
fi
# Fail closed if no smoke-row executed (no packages produced or kind/platform
# mismatch). runs[] already holds one honest row per artifact that did run.
if [ "$smoke_count" -eq 0 ]; then
  echo "package-inspect: no package artifacts found for smoke-row (kind=${kind})" >&2
  overall_failed=1
  run_n=$((run_n + 1))
  tests_failed=$((tests_failed + 1))
  printf '%s\tfailed\t0\t0\t0\tno-smoke-row-artifacts\n' "$run_n" >>"$runs_tsv"
fi

ended_ms="$(now_ms)"

final_exit=0
[ "$overall_failed" -eq 0 ] || final_exit=1
command_id="package-${kind}-${job}"

# runs[] holds one honest row per executed step (package-local,
# artifact-inspect, sbom, one package-smoke-row per produced artifact). No
# synthetic aggregate row is appended: the schema requires run >= 1 and a row
# that does not correspond to a real executed step would overstate coverage.
# Outstanding human/platform inputs (e.g. pending-cyclonedx) are surfaced via
# stdout below and in retained logs, never faked as a passed run.

gate_path="$(emit_gate_result \
  --command-id "$command_id" \
  --profile "$profile" \
  --required-row "$command_id" \
  --exit-code "$final_exit" \
  --started-ms "$started_ms" \
  --ended-ms "$ended_ms" \
  --job-dir "$job_dir" \
  --artifact "$candidate" \
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

echo "package-inspect: gate-result=${gate_path}"
echo "package-inspect: outstanding=${outstanding}"
exit "$final_exit"
