#!/bin/sh
set -eu

install -o root -g root -m 0755 target/release/feathermark-runner-launcher /usr/libexec/feathermark-runner-launcher
install -o root -g root -m 0500 target/release/feathermark-runner-probe /usr/libexec/feathermark-runner-probe
install -d -o root -g root -m 0700 /var/lib/feathermark-runner /run/feathermark-runner
install -o root -g root -m 0644 xtask/launcher/feathermark-runner-launcher.service /etc/systemd/system/feathermark-runner-launcher.service
systemctl daemon-reload
systemctl enable --now feathermark-runner-launcher.service
