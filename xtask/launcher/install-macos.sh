#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: $0 LAUNCHER_CONFIG RUNNER_KEY SNAPSHOT_ATTESTATION" >&2
  exit 64
fi

launcher_config=$1
runner_key=$2
snapshot_attestation=$3

install -o root -g wheel -m 0755 target/release/feathermark-runner-launcher /Library/PrivilegedHelperTools/com.feathermark.runner-launcher
install -d -o root -g wheel -m 0755 "/Library/Application Support/FeatherMark Runner/bin"
install -o root -g wheel -m 0500 target/release/feathermark-runner-probe "/Library/Application Support/FeatherMark Runner/bin/feathermark-runner-probe"
install -d -o root -g wheel -m 0700 "/Library/Application Support/FeatherMark Runner/private" /private/var/run/feathermark-runner
install -o root -g wheel -m 0400 "$launcher_config" "/Library/Application Support/FeatherMark Runner/private/launcher-config-v1.json"
install -o root -g wheel -m 0400 "$runner_key" "/Library/Application Support/FeatherMark Runner/private/runner-key-v1"
install -o root -g wheel -m 0400 "$snapshot_attestation" "/Library/Application Support/FeatherMark Runner/private/snapshot-attestation-v1.json"
if [ ! -e "/Library/Application Support/FeatherMark Runner/private/replay-cache-v1" ]; then
  install -o root -g wheel -m 0600 /dev/null "/Library/Application Support/FeatherMark Runner/private/replay-cache-v1"
fi
install -o root -g wheel -m 0644 xtask/launcher/com.feathermark.runner-launcher.plist /Library/LaunchDaemons/com.feathermark.runner-launcher.plist
launchctl bootstrap system /Library/LaunchDaemons/com.feathermark.runner-launcher.plist
