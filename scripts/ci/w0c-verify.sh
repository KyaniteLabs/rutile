#!/usr/bin/env bash
# W0-C verification bar — the canonical pre-merge gate for the Rutile remediation.
#
# Codifies the exact checks the ultragoal W0-C adversarial review requires, so the
# bar is reproducible rather than tribal knowledge. The spike-class regression
# (Wave 2 added MacUserEvent::MenuCommand but a feasibility spike kept an
# irrefutable `let`, breaking `cargo clippy --workspace --all-targets`) slipped
# through CI because the portable gate clippy's explicit -p list excludes spikes;
# this script runs the full workspace bar that catches it.
#
# Run on a platform where the workspace members you care about compile
# (your dev Mac catches the macOS spikes; Linux catches the linux-gtk spike).
# Exits non-zero on any gate failure. Bypass only with deliberate intent.
#
#   bash scripts/ci/w0c-verify.sh
set -euo pipefail

say() { printf '\n=== [%s] %s ===\n' "$1" "$2"; }

say 1/5 "cargo fmt --all -- --check"
cargo fmt --all -- --check

say 2/5 "cargo clippy --workspace --all-targets --locked -- -D warnings  (catches cross-crate + spike breakage the per-package portable gate misses)"
cargo clippy --workspace --all-targets --locked -- -D warnings

say 3/5 "cargo test -p xtask --all-targets --locked  (parallel: native_smoke git-provenance capture is hermetic, no shared GIT_* env)"
cargo test -p xtask --all-targets --locked

say 4/5 "git diff --check  (whitespace/conflict markers)"
git diff --check

say 5/5 "evidence schema validation"
python3 -m jsonschema -i release/evidence/release-prerequisite-preflight-v1.json schemas/rutile.release-prerequisite-preflight.v1.schema.json
python3 -m jsonschema -i release/evidence/w0b-stage0-blocked-receipt-v1.json schemas/rutile.w0b-stage0-blocked-receipt.v1.schema.json

# Note: the W0-C public-leak audit (secrets/host-paths/PII) remains a manual
# reviewer judgment — intentional schema rejection-patterns and test@example.com
# in throwaway fixtures are allowed. Grep release/evidence + schemas by hand.

printf '\n=== W0-C verification bar: PASS ===\n'
