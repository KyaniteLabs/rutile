#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: $0 LAUNCHER_CONFIG RUNNER_KEY SNAPSHOT_ATTESTATION" >&2
  exit 64
fi

launcher_config=$1
runner_key=$2
snapshot_attestation=$3

install -o root -g root -m 0755 target/release/feathermark-runner-launcher /usr/libexec/feathermark-runner-launcher
install -o root -g root -m 0500 target/release/feathermark-runner-probe /usr/libexec/feathermark-runner-probe
install -d -o root -g root -m 0700 /var/lib/feathermark-runner /run/feathermark-runner
install -o root -g root -m 0400 "$launcher_config" /var/lib/feathermark-runner/launcher-config-v1.json
install -o root -g root -m 0400 "$runner_key" /var/lib/feathermark-runner/runner-key-v1
install -o root -g root -m 0400 "$snapshot_attestation" /var/lib/feathermark-runner/snapshot-attestation-v1.json
if [ ! -e /var/lib/feathermark-runner/replay-cache-v1 ]; then
  install -o root -g root -m 0600 /dev/null /var/lib/feathermark-runner/replay-cache-v1
fi
install -o root -g root -m 0644 xtask/launcher/feathermark-runner-launcher@.service /etc/systemd/system/feathermark-runner-launcher@.service
install -o root -g root -m 0644 xtask/launcher/feathermark-runner-launcher.socket /etc/systemd/system/feathermark-runner-launcher.socket
systemctl daemon-reload
systemctl enable --now feathermark-runner-launcher.socket
