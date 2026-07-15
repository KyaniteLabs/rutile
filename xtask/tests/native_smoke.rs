#![cfg(unix)]
#![allow(clippy::disallowed_methods)] // Harness launches only local fixtures and repository wrappers.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command as ProcessCommand;
use std::time::Duration;

use clap::Parser;
use sha2::{Digest, Sha256};
use xtask::cli::{Cli, Command};
use xtask::native_smoke::{
    NativeSmokeCommand, NativeSmokeFailure, NativeSmokeProfile, PARENT_DEADLINE, SupervisorFaults,
    TERM_GRACE, resolve_repeats, supervise_for_test, supervise_for_test_with_faults,
    supervision_bound,
};

fn shell(script: &str) -> NativeSmokeCommand {
    NativeSmokeCommand::new("/bin/sh", ["-c", script])
}

fn short(
    command: NativeSmokeCommand,
) -> Result<xtask::native_smoke::NativeSmokeReceipt, NativeSmokeFailure> {
    supervise_for_test(
        command,
        Duration::from_millis(150),
        Duration::from_millis(75),
    )
}

fn short_with_faults(
    command: NativeSmokeCommand,
    faults: SupervisorFaults,
) -> Result<xtask::native_smoke::NativeSmokeReceipt, NativeSmokeFailure> {
    supervise_for_test_with_faults(
        command,
        Duration::from_millis(150),
        Duration::from_millis(75),
        faults,
    )
}

#[test]
fn supervised_smoke_parses_stage_and_resize_traces_from_stderr() {
    let receipt = short(shell(
        "printf 'SMOKE_TRACE stage=0 event=resumed\\n' >&2; printf 'SMOKE_TRACE stage=1 event=resize 1200x760\\n' >&2; printf 'feathermark-native-smoke-ok\\n'",
    ))
    .unwrap();

    assert!(receipt.success());
    assert!(receipt.stdout().contains("feathermark-native-smoke-ok"));
    assert_eq!(receipt.stage_trace().len(), 2);
    assert_eq!(receipt.resize_trace().len(), 1);
}

#[test]
fn supervised_smoke_reports_child_failure_with_retained_stderr() {
    let failure = short(shell("printf failed >&2; exit 23")).unwrap_err();

    assert!(matches!(
        failure,
        NativeSmokeFailure::Exited {
            status: Some(23),
            ..
        }
    ));
    assert!(failure.diagnostics().stderr.contains("failed"));
}

#[test]
fn supervised_smoke_times_out_when_child_is_event_starved() {
    let failure = short(shell(
        "printf 'SMOKE_TRACE stage=0 event=resumed\\n'; while :; do :; done",
    ))
    .unwrap_err();

    assert!(matches!(failure, NativeSmokeFailure::TimedOut { .. }));
    assert!(
        failure
            .diagnostics()
            .stage_trace
            .iter()
            .any(|line| line.contains("stage=0"))
    );
}

#[test]
fn supervised_smoke_kills_term_resistant_process_group_and_reaps_leader() {
    let failure = short(shell("trap '' TERM; while :; do :; done")).unwrap_err();

    assert!(
        matches!(failure, NativeSmokeFailure::TimedOut { killed: true, .. }),
        "{failure:?}"
    );
    assert!(failure.diagnostics().reaped);
}

#[test]
fn supervised_smoke_kills_descendant_that_keeps_pipes_open_after_leader_exits() {
    let root = tempfile::tempdir().unwrap();
    let ready = root.path().join("descendant-ready");
    let script = format!(
        "(trap '' TERM; : > '{}'; while :; do :; done) & while [ ! -f '{}' ]; do :; done; trap 'exit 0' TERM; while :; do :; done",
        ready.display(),
        ready.display()
    );
    let failure = short(shell(&script)).unwrap_err();

    assert!(
        matches!(failure, NativeSmokeFailure::TimedOut { killed: true, .. }),
        "{failure:?}"
    );
    assert!(failure.diagnostics().reaped);
}

#[test]
fn production_supervision_bound_is_the_35_second_product_contract() {
    assert_eq!(
        supervision_bound(PARENT_DEADLINE, TERM_GRACE),
        Duration::from_secs(35)
    );
}

#[test]
fn supervised_smoke_cleans_the_group_after_an_injected_wait_failure() {
    let failure = short_with_faults(
        shell("trap '' TERM; while :; do :; done"),
        SupervisorFaults::wait_once(),
    )
    .unwrap_err();

    assert!(matches!(failure, NativeSmokeFailure::Wait { .. }));
    assert!(failure.diagnostics().reaped);
}

#[test]
fn supervised_smoke_cleans_the_group_after_an_injected_read_failure() {
    let failure = short_with_faults(
        shell("trap '' TERM; while :; do :; done"),
        SupervisorFaults::stderr_read_once(),
    )
    .unwrap_err();

    assert!(matches!(failure, NativeSmokeFailure::Read { .. }));
    assert!(failure.diagnostics().reaped);
}

#[test]
fn supervised_smoke_bounds_output_flood_without_losing_cleanup() {
    let failure = short(shell("while :; do printf 0123456789; done")).unwrap_err();

    assert!(matches!(failure, NativeSmokeFailure::OutputLimit { .. }));
    assert!(failure.diagnostics().reaped);
    assert!(failure.diagnostics().stdout.len() <= 16 * 1024);
}

#[test]
fn supervised_smoke_bounds_newline_free_trace_flood() {
    let failure = short(shell(
        "while :; do printf 'SMOKE_TRACE stage=0 event=resize '; done",
    ))
    .unwrap_err();

    assert!(matches!(failure, NativeSmokeFailure::OutputLimit { .. }));
    assert!(
        failure
            .diagnostics()
            .stage_trace
            .iter()
            .all(|line| line.len() <= 512)
    );
    assert!(
        failure
            .diagnostics()
            .resize_trace
            .iter()
            .all(|line| line.len() <= 512)
    );
}

#[test]
fn native_smoke_failure_prints_retained_diagnostics() {
    let failure = short(shell("printf out; printf err >&2; exit 9")).unwrap_err();
    let rendered = failure.to_string();

    assert!(rendered.contains("stdout"));
    assert!(rendered.contains("out"));
    assert!(rendered.contains("stderr"));
    assert!(rendered.contains("err"));
}

#[test]
fn native_smoke_cli_requires_an_explicit_profile_and_evidence_directory() {
    let cli = Cli::try_parse_from([
        "xtask",
        "native-smoke",
        "--binary",
        "/tmp/feathermark",
        "--profile",
        "release",
        "--repeat",
        "50",
        "--evidence-dir",
        "/tmp/native-smoke-evidence",
    ])
    .unwrap();

    match cli.command {
        Command::NativeSmoke {
            binary,
            profile,
            repeat,
            evidence_dir,
        } => {
            assert_eq!(binary.to_string_lossy(), "/tmp/feathermark");
            assert_eq!(profile, NativeSmokeProfile::Release);
            assert_eq!(repeat.unwrap().get(), 50);
            assert_eq!(evidence_dir.to_string_lossy(), "/tmp/native-smoke-evidence");
        }
        _ => panic!("expected native-smoke command"),
    }
}

#[test]
fn native_smoke_profiles_enforce_minimum_repeats() {
    assert_eq!(resolve_repeats(NativeSmokeProfile::Pr, None).unwrap(), 10);
    assert_eq!(
        resolve_repeats(NativeSmokeProfile::Release, None).unwrap(),
        50
    );
    assert!(resolve_repeats(NativeSmokeProfile::Pr, Some(9)).is_err());
    assert!(resolve_repeats(NativeSmokeProfile::Release, Some(49)).is_err());
    assert_eq!(
        resolve_repeats(NativeSmokeProfile::Release, Some(51)).unwrap(),
        51
    );
}

#[test]
fn native_smoke_cli_emits_gate_json_and_retains_bounded_run_logs() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fake-feathermark");
    fs::write(
        &binary,
        "#!/bin/sh\nprintf 'SMOKE_TRACE stage=0 event=launch\\n' >&2\nprintf 'feathermark-native-smoke-ok\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let evidence = root.path().join("evidence");

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--repeat", "10", "--evidence-dir"])
        .arg(&evidence)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report_path = gate_report_paths(&evidence).pop().expect("gate report");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["schema"], "rutile.gate-result.v1");
    assert_eq!(report["command_id"], "macos-native-smoke");
    assert_eq!(report["profile"], "pr");
    assert_eq!(report["exit_code"], 0);
    assert_eq!(report["tests"]["total"], 10);
    assert_eq!(report["tests"]["passed"], 10);
    assert_eq!(report["tests"]["failed"], 0);
    assert_eq!(report["tests"]["skipped"], 0);
    assert_eq!(report["required_row"]["status"], "passed");
    assert_eq!(report["retained_logs"].as_array().unwrap().len(), 20);
    for log in report["retained_logs"].as_array().unwrap() {
        let path = report_path
            .parent()
            .unwrap()
            .join(log["path"].as_str().unwrap());
        assert!(path.is_file());
        assert!(fs::metadata(path).unwrap().len() <= 16 * 1024);
        assert_eq!(log["sha256"].as_str().unwrap().len(), 64);
    }
    let schema: serde_json::Value = serde_json::from_slice(
        &fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../schemas/rutile.gate-result.v1.schema.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(schema["properties"]["schema"]["const"], report["schema"]);
}

#[test]
fn native_smoke_cli_records_the_real_failure_in_gate_json() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("failing-feathermark");
    fs::write(
        &binary,
        "#!/bin/sh\nprintf 'SMOKE_TRACE stage=0 event=launch\\n' >&2\nprintf 'wrapper-failure\\n' >&2\nexit 23\n",
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let evidence = root.path().join("evidence");

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("wrapper-failure"));
    let report: serde_json::Value = serde_json::from_slice(
        &fs::read(gate_report_paths(&evidence).pop().expect("gate report")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["exit_code"], 1);
    assert_eq!(report["tests"]["total"], 10);
    assert_eq!(report["tests"]["passed"], 0);
    assert_eq!(report["tests"]["failed"], 1);
    assert_eq!(report["tests"]["skipped"], 9);
    assert_eq!(report["required_row"]["status"], "failed");
    assert_eq!(report["runs"].as_array().unwrap().len(), 1);
    assert_eq!(report["runs"][0]["status"], "failed");
    assert_eq!(report["runs"][0]["reaped"], true);
}

#[test]
fn native_smoke_rejects_a_missing_binary_before_creating_evidence() {
    let root = tempfile::tempdir().unwrap();
    let missing_binary = root.path().join("missing-feathermark");
    let evidence = root.path().join("evidence");

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&missing_binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        !evidence.exists(),
        "a rejected binary must not leave an evidence directory behind"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing-feathermark"));
}

#[test]
fn native_smoke_receipt_keeps_the_prelaunch_hash_when_binary_changes() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("mutating-feathermark");
    let changed_contents = "#!/bin/sh\nprintf 'feathermark-native-smoke-ok\\n'\n";
    let initial_contents = format!(
        "#!/bin/sh\nprintf 'feathermark-native-smoke-ok\\n'\nprintf '%s' '{}' > '{}'\nchmod 755 '{}'\n",
        changed_contents.replace('\'', "'\\''"),
        binary.display(),
        binary.display(),
    );
    write_executable(&binary, &initial_contents);
    let expected_hash = hex::encode(Sha256::digest(initial_contents.as_bytes()));
    let evidence = root.path().join("evidence");

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report = gate_report(&evidence);
    assert_eq!(report["artifact_hashes"][0]["sha256"], expected_hash);
    assert_eq!(report["runs"].as_array().unwrap().len(), 2);
    assert_eq!(report["runs"][1]["status"], "failed");
    assert!(
        report["runs"][1]["error"]
            .as_str()
            .unwrap()
            .contains("changed since preflight")
    );
}

#[test]
fn native_smoke_captures_complete_git_provenance_before_the_first_child_launch() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("bin");
    fs::create_dir(&tools).unwrap();
    write_executable(
        &tools.join("git"),
        "#!/bin/sh\nprintf provenance >\"$PROVENANCE_CAPTURE\"\ncase \"$*\" in\n  'rev-parse HEAD') printf '%s\\n' '0123456789abcdef0123456789abcdef01234567' ;;\n  'rev-parse HEAD^{tree}') printf '%s\\n' '0123456789abcdef0123456789abcdef01234567' ;;\n  'status --porcelain --untracked-files=all') printf '%s' '?? untracked-input\\n' ;;\n  *) exit 47 ;;\nesac\n",
    );
    let binary = root.path().join("fake-feathermark");
    write_executable(
        &binary,
        "#!/bin/sh\n[ -f \"$PROVENANCE_CAPTURE\" ] || { echo missing-provenance >&2; exit 29; }\nprintf 'feathermark-native-smoke-ok\\n'\n",
    );
    let evidence = root.path().join("evidence");
    let capture = root.path().join("provenance-captured");
    let path = format!("{}:{}", tools.display(), std::env::var("PATH").unwrap());

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .env("PATH", path)
        .env("PROVENANCE_CAPTURE", &capture)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = gate_report(&evidence);
    assert_eq!(
        report["source"]["commit"],
        "0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(
        report["source"]["tree"],
        "0123456789abcdef0123456789abcdef01234567"
    );
    assert_eq!(report["source"]["dirty"], true);
}

#[test]
fn native_smoke_fails_closed_when_git_provenance_command_fails() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("bin");
    fs::create_dir(&tools).unwrap();
    write_executable(&tools.join("git"), "#!/bin/sh\nexit 47\n");
    let binary = root.path().join("fake-feathermark");
    let launched = root.path().join("child-launched");
    write_executable(
        &binary,
        "#!/bin/sh\nprintf launched >\"$CHILD_LAUNCHED\"\nprintf 'feathermark-native-smoke-ok\\n'\n",
    );
    let evidence = root.path().join("evidence");
    let path = format!("{}:{}", tools.display(), std::env::var("PATH").unwrap());

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .env("PATH", path)
        .env("CHILD_LAUNCHED", &launched)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!launched.exists(), "the smoke child ran without provenance");
    assert!(String::from_utf8_lossy(&output.stderr).contains("git"));
}

#[test]
fn native_smoke_fails_closed_when_git_provenance_identity_is_empty() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("bin");
    fs::create_dir(&tools).unwrap();
    write_executable(
        &tools.join("git"),
        "#!/bin/sh\ncase \"$*\" in\n  'rev-parse HEAD') exit 0 ;;\n  *) printf '%s\\n' identity ;;\nesac\n",
    );
    let binary = root.path().join("fake-feathermark");
    let launched = root.path().join("child-launched");
    write_executable(
        &binary,
        "#!/bin/sh\nprintf launched >\"$CHILD_LAUNCHED\"\nprintf 'feathermark-native-smoke-ok\\n'\n",
    );
    let evidence = root.path().join("evidence");
    let path = format!("{}:{}", tools.display(), std::env::var("PATH").unwrap());

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .env("PATH", path)
        .env("CHILD_LAUNCHED", &launched)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        !launched.exists(),
        "the smoke child ran with an empty identity"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("empty"));
}

#[test]
fn native_smoke_rejects_uppercase_git_identity_before_child_launch() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("bin");
    fs::create_dir(&tools).unwrap();
    write_executable(
        &tools.join("git"),
        "#!/bin/sh\ncase \"$*\" in\n  'status --porcelain --untracked-files=all') exit 0 ;;\n  *) printf '%s\\n' 'ABCDEF0123456789ABCDEF0123456789ABCDEF01' ;;\nesac\n",
    );
    let binary = root.path().join("fake-feathermark");
    let launched = root.path().join("child-launched");
    write_executable(
        &binary,
        "#!/bin/sh\nprintf launched >\"$CHILD_LAUNCHED\"\nprintf 'feathermark-native-smoke-ok\\n'\n",
    );
    let evidence = root.path().join("evidence");
    let path = format!("{}:{}", tools.display(), std::env::var("PATH").unwrap());

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .env("PATH", path)
        .env("CHILD_LAUNCHED", &launched)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        !launched.exists(),
        "child ran with schema-invalid provenance"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("git object ID"));
}

#[test]
fn native_smoke_bounds_git_provenance_output_before_child_launch() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("bin");
    fs::create_dir(&tools).unwrap();
    write_executable(
        &tools.join("git"),
        "#!/bin/sh\ni=0\nwhile [ \"$i\" -lt 20000 ]; do printf a; i=$((i + 1)); done\n",
    );
    let binary = root.path().join("fake-feathermark");
    let launched = root.path().join("child-launched");
    write_executable(
        &binary,
        "#!/bin/sh\nprintf launched >\"$CHILD_LAUNCHED\"\nprintf 'feathermark-native-smoke-ok\\n'\n",
    );
    let evidence = root.path().join("evidence");
    let path = format!("{}:{}", tools.display(), std::env::var("PATH").unwrap());

    let output = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .env("PATH", path)
        .env("CHILD_LAUNCHED", &launched)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!launched.exists(), "child ran after unbounded git output");
    assert!(String::from_utf8_lossy(&output.stderr).contains("output limit"));
}

#[test]
fn native_smoke_reruns_create_distinct_evidence_runs_without_overwriting_failure() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fake-feathermark");
    write_executable(&binary, "#!/bin/sh\nprintf first-failure >&2\nexit 23\n");
    let evidence = root.path().join("evidence");

    let first = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .output()
        .unwrap();
    assert!(!first.status.success());
    let first_report_path = gate_report_paths(&evidence).pop().expect("first report");
    let first_report = fs::read(&first_report_path).unwrap();

    write_executable(
        &binary,
        "#!/bin/sh\nprintf 'feathermark-native-smoke-ok\\n'\n",
    );
    let second = ProcessCommand::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["native-smoke", "--binary"])
        .arg(&binary)
        .args(["--profile", "pr", "--evidence-dir"])
        .arg(&evidence)
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let reports = gate_report_paths(&evidence);
    assert_eq!(reports.len(), 2, "each invocation must retain a report");
    let second_report_path = reports
        .into_iter()
        .find(|path| path != &first_report_path)
        .expect("second report");

    assert_ne!(first_report_path, second_report_path);
    assert_eq!(fs::read(&first_report_path).unwrap(), first_report);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&first_report).unwrap()["required_row"]["status"],
        "failed"
    );
    assert!(
        first_report_path
            .parent()
            .unwrap()
            .join("run-0001.stderr.log")
            .is_file()
    );
    assert!(
        second_report_path
            .parent()
            .unwrap()
            .join("run-0001.stdout.log")
            .is_file()
    );
}

fn write_executable(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn gate_report(evidence: &std::path::Path) -> serde_json::Value {
    let report_path = gate_report_paths(evidence).pop().expect("gate report");
    serde_json::from_slice(&fs::read(report_path).unwrap()).unwrap()
}

fn gate_report_paths(evidence: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut reports = Vec::new();
    for entry in fs::read_dir(evidence).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            let report = path.join("gate-result.json");
            if report.is_file() {
                reports.push(report);
            }
        } else if path
            .file_name()
            .is_some_and(|name| name == "gate-result.json")
        {
            reports.push(path);
        }
    }
    reports.sort();
    reports
}

#[test]
fn native_smoke_wrapper_requires_profile_and_rejects_lower_overrides() {
    let root = tempfile::tempdir().unwrap();
    let tools = root.path().join("bin");
    fs::create_dir(&tools).unwrap();
    for (name, body) in [
        ("uname", "#!/bin/sh\nprintf 'Darwin\\n'\n"),
        (
            "cargo",
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >>\"$MOCK_CARGO_LOG\"\n",
        ),
    ] {
        let path = tools.join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/feathermark-macos-native-smoke.sh");
    let cargo_log = root.path().join("cargo.log");
    let path = format!("{}:{}", tools.display(), std::env::var("PATH").unwrap());

    let missing = ProcessCommand::new("/bin/sh")
        .arg(&script)
        .env("PATH", &path)
        .env("MOCK_CARGO_LOG", &cargo_log)
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(
        String::from_utf8_lossy(&missing.stderr).contains("--profile"),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&missing.stdout),
        String::from_utf8_lossy(&missing.stderr)
    );

    let low = ProcessCommand::new("/bin/sh")
        .arg(&script)
        .args(["--profile", "pr", "--repeat", "9"])
        .env("PATH", &path)
        .env("MOCK_CARGO_LOG", &cargo_log)
        .output()
        .unwrap();
    assert!(!low.status.success());
    assert!(String::from_utf8_lossy(&low.stderr).contains("at least 10"));

    let valid = ProcessCommand::new("/bin/sh")
        .arg(&script)
        .args(["--profile", "release", "--repeat", "50", "--evidence-dir"])
        .arg(root.path().join("evidence"))
        .env("PATH", &path)
        .env("MOCK_CARGO_LOG", &cargo_log)
        .env("CARGO_TARGET_DIR", root.path().join("target"))
        .output()
        .unwrap();
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    let calls = fs::read_to_string(cargo_log).unwrap();
    assert!(
        calls
            .contains("build --locked -p feathermark-app --features macos-shell --bin feathermark")
    );
    assert!(calls.contains("run --locked -p xtask --bin xtask -- native-smoke"));
    assert!(calls.contains("--profile release --repeat 50"));
}

#[test]
fn native_smoke_source_traces_launch_before_resume_and_resize_request_before_outcome() {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/feathermark-app/src/platform/macos/native.rs"),
    )
    .unwrap();
    let launch = source
        .find("SMOKE_TRACE stage=0 event=launch")
        .expect("launch trace");
    let runner_setup = source
        .find("ProductRunner::new(session, smoke, display_handle)")
        .expect("runner setup");
    let event_loop_resume = source
        .find(".run_app(&mut runner)")
        .expect("event-loop resume");
    assert!(launch < runner_setup);
    assert!(launch < event_loop_resume);

    let resize_request = source
        .find("event=resize-request")
        .expect("resize-request trace");
    let resize_outcome = source
        .find("event=resize logical=")
        .expect("resize outcome trace");
    assert!(resize_request < resize_outcome);
}
