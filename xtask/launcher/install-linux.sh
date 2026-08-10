#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: $0 LAUNCHER_CONFIG RUNNER_KEY SNAPSHOT_ATTESTATION" >&2
  exit 64
fi

launcher_config=$1
runner_key=$2
snapshot_attestation=$3

install -o root -g root -m 0755 target/release/rutile-runner-launcher /usr/libexec/rutile-runner-launcher
install -o root -g root -m 0500 target/release/rutile-runner-probe /usr/libexec/rutile-runner-probe
install -d -o root -g root -m 0700 /var/lib/rutile-runner /run/rutile-runner
install -o root -g root -m 0400 "$launcher_config" /var/lib/rutile-runner/launcher-config-v1.json
install -o root -g root -m 0400 "$runner_key" /var/lib/rutile-runner/runner-key-v1
install -o root -g root -m 0400 "$snapshot_attestation" /var/lib/rutile-runner/snapshot-attestation-v1.json
if [ ! -e /var/lib/rutile-runner/replay-cache-v1 ]; then
  install -o root -g root -m 0600 /dev/null /var/lib/rutile-runner/replay-cache-v1
fi
install -o root -g root -m 0644 xtask/launcher/rutile-runner-launcher@.service /etc/systemd/system/rutile-runner-launcher@.service
install -o root -g root -m 0644 xtask/launcher/rutile-runner-launcher.socket /etc/systemd/system/rutile-runner-launcher.socket
systemctl daemon-reload
systemctl enable --now rutile-runner-launcher.socket
