#!/bin/sh
set -eu

install -o root -g wheel -m 0755 target/release/feathermark-runner-launcher /Library/PrivilegedHelperTools/com.feathermark.runner-launcher
install -d -o root -g wheel -m 0755 "/Library/Application Support/FeatherMark Runner/bin"
install -o root -g wheel -m 0500 target/release/feathermark-runner-probe "/Library/Application Support/FeatherMark Runner/bin/feathermark-runner-probe"
install -o root -g wheel -m 0644 xtask/launcher/com.feathermark.runner-launcher.plist /Library/LaunchDaemons/com.feathermark.runner-launcher.plist
launchctl bootstrap system /Library/LaunchDaemons/com.feathermark.runner-launcher.plist
