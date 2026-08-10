#![allow(clippy::disallowed_methods)]

use std::process::Command;

#[test]
fn native_binaries_reject_cli_configuration_and_emit_no_protocol_bytes() {
    for binary in [
        env!("CARGO_BIN_EXE_rutile-runner-probe"),
        env!("CARGO_BIN_EXE_rutile-runner-launcher"),
    ] {
        let output = Command::new(binary)
            .arg("--config-from-caller")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("accepts no arguments"));
    }
}

#[test]
fn service_definitions_are_one_request_root_sockets_and_install_private_inputs() {
    let linux_service = include_str!("../launcher/rutile-runner-launcher@.service");
    let linux_socket = include_str!("../launcher/rutile-runner-launcher.socket");
    let macos = include_str!("../launcher/com.rutile.runner-launcher.plist");
    let linux_install = include_str!("../launcher/install-linux.sh");
    let macos_install = include_str!("../launcher/install-macos.sh");

    assert!(linux_service.contains("StandardInput=socket"));
    assert!(linux_service.contains("StandardOutput=socket"));
    assert!(linux_socket.contains("Accept=yes"));
    assert!(linux_socket.contains("SocketMode=0600"));
    assert!(macos.contains("<key>inetdCompatibility</key>"));
    assert!(macos.contains("<key>SockPathMode</key><integer>384</integer>"));
    for install in [linux_install, macos_install] {
        assert!(install.contains("launcher-config-v1.json"));
        assert!(install.contains("runner-key-v1"));
        assert!(install.contains("snapshot-attestation-v1.json"));
        assert!(install.contains("-m 0400"));
        assert!(install.contains("-m 0500"));
    }
}
