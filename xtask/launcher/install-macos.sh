#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: $0 LAUNCHER_CONFIG RUNNER_KEY SNAPSHOT_ATTESTATION" >&2
  exit 64
fi

launcher_config=$1
runner_key=$2
snapshot_attestation=$3

install -o root -g wheel -m 0755 target/release/rutile-runner-launcher /Library/PrivilegedHelperTools/com.rutile.runner-launcher
install -d -o root -g wheel -m 0755 "/Library/Application Support/Rutile Runner/bin"
install -o root -g wheel -m 0500 target/release/rutile-runner-probe "/Library/Application Support/Rutile Runner/bin/rutile-runner-probe"
install -d -o root -g wheel -m 0700 "/Library/Application Support/Rutile Runner/private" /private/var/run/rutile-runner
install -o root -g wheel -m 0400 "$launcher_config" "/Library/Application Support/Rutile Runner/private/launcher-config-v1.json"
install -o root -g wheel -m 0400 "$runner_key" "/Library/Application Support/Rutile Runner/private/runner-key-v1"
install -o root -g wheel -m 0400 "$snapshot_attestation" "/Library/Application Support/Rutile Runner/private/snapshot-attestation-v1.json"
if [ ! -e "/Library/Application Support/Rutile Runner/private/replay-cache-v1" ]; then
  install -o root -g wheel -m 0600 /dev/null "/Library/Application Support/Rutile Runner/private/replay-cache-v1"
fi
install -o root -g wheel -m 0644 xtask/launcher/com.rutile.runner-launcher.plist /Library/LaunchDaemons/com.rutile.runner-launcher.plist
launchctl bootstrap system /Library/LaunchDaemons/com.rutile.runner-launcher.plist
