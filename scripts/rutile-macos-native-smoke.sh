#!/bin/sh
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
  echo "macOS native smoke requires Darwin" >&2
  exit 2
fi

target_dir=${CARGO_TARGET_DIR:-target}
profile=
repeat=
evidence_dir=

usage() {
  echo "usage: $0 --profile pr|release [--repeat COUNT] [--evidence-dir PATH]" >&2
  echo "each invocation retains a new immutable evidence run beneath --evidence-dir" >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --profile) [ "$#" -ge 2 ] || usage; profile=$2; shift 2 ;;
    --repeat) [ "$#" -ge 2 ] || usage; repeat=$2; shift 2 ;;
    --evidence-dir) [ "$#" -ge 2 ] || usage; evidence_dir=$2; shift 2 ;;
    *) usage ;;
  esac
done

case "$profile" in
  pr) minimum=10 ;;
  release) minimum=50 ;;
  *) usage ;;
esac

if [ -z "$repeat" ]; then
  repeat=$minimum
fi
if [ -z "$evidence_dir" ]; then
  evidence_dir="$target_dir/evidence/native-smoke-$profile"
fi

case "$repeat" in
  *[!0-9]*|'') echo "--repeat must be a positive integer" >&2; exit 2 ;;
esac
if [ "$repeat" -lt "$minimum" ]; then
  echo "--profile $profile requires --repeat of at least $minimum" >&2
  exit 2
fi

cargo build --locked -p rutile-app --features macos-shell --bin rutile
cargo run --locked -p xtask --bin xtask -- native-smoke \
  --binary "$target_dir/debug/rutile" \
  --profile "$profile" \
  --repeat "$repeat" \
  --evidence-dir "$evidence_dir"
