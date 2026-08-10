use crate::runner::encoding::{
    MAX_NATIVE_REPORT_BYTES, decode_native_challenge, decode_native_report, decode_probe_request,
    encode_native_challenge, encode_native_report, encode_probe_payload,
};
#[cfg(test)]
use crate::runner::protocol::RunnerIdentityV1;
use crate::runner::protocol::{
    NativeProbeChallengeV1, NativeProbeReportV1, ProbePayloadV1, ProbePurpose, ProbeRequestV1,
    SignedRunnerProbeV1,
};
use ed25519_dalek::{Signer, SigningKey};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::{Duration, Instant};
use thiserror::Error;
mod collector;
mod path_policy;
pub(crate) mod platform;
mod replay;
mod service_config;

const COORDINATOR_MAGIC: &[u8; 8] = b"RURP\0v1\0";
const PROBE_INPUT_MAGIC: &[u8; 8] = b"RUPI\0v1\0";
const PROBE_OUTPUT_MAGIC: &[u8; 8] = b"RUPO\0v1\0";
const MAX_COORDINATOR_REQUEST: usize = 16 * 1024;
const NATIVE_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_NATIVE_CHILD_STDERR_BYTES: usize = 8 * 1024;

#[cfg(target_os = "macos")]
const CONFIG_PATH: &str =
    "/Library/Application Support/Rutile Runner/private/launcher-config-v1.json";
#[cfg(target_os = "macos")]
const KEY_PATH: &str = "/Library/Application Support/Rutile Runner/private/runner-key-v1";
#[cfg(target_os = "macos")]
const SNAPSHOT_PATH: &str =
    "/Library/Application Support/Rutile Runner/private/snapshot-attestation-v1.json";
#[cfg(target_os = "macos")]
const REPLAY_PATH: &str = "/Library/Application Support/Rutile Runner/private/replay-cache-v1";
#[cfg(target_os = "macos")]
const PROBE_ROOT: &str = "/Library/Application Support/Rutile Runner/bin";
#[cfg(target_os = "macos")]
const EXECUTION_ROOT: &str = "/private/var/run/rutile-runner";

#[cfg(target_os = "linux")]
const CONFIG_PATH: &str = "/var/lib/rutile-runner/launcher-config-v1.json";
#[cfg(target_os = "linux")]
const KEY_PATH: &str = "/var/lib/rutile-runner/runner-key-v1";
#[cfg(target_os = "linux")]
const SNAPSHOT_PATH: &str = "/var/lib/rutile-runner/snapshot-attestation-v1.json";
#[cfg(target_os = "linux")]
const REPLAY_PATH: &str = "/var/lib/rutile-runner/replay-cache-v1";
#[cfg(target_os = "linux")]
const PROBE_ROOT: &str = "/usr/libexec";

const PROBE_NAME: &str = "rutile-runner-probe";

#[derive(Debug, Error)]
#[doc(hidden)]
pub enum NativeServiceError {
    #[error("native runner service accepts no arguments")]
    Arguments,
    #[error("native runner service must execute as root")]
    NotRoot,
    #[error("native runner service I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("native runner service protocol failed: {0}")]
    Protocol(String),
    #[error("native runner service configuration failed: {0}")]
    Configuration(String),
    #[error("native runner coordinator protocol failed: {0}")]
    Runner(#[from] crate::runner::RunnerError),
}

#[doc(hidden)]
pub fn probe_main() -> Result<(), NativeServiceError> {
    reject_arguments()?;
    let bytes = read_frame(&mut std::io::stdin().lock(), PROBE_INPUT_MAGIC, 128)?;
    let challenge = decode_native_challenge(&bytes)?;
    let attestation_bytes = read_secure_file(Path::new(SNAPSHOT_PATH), 16 * 1024, 0)?;
    let attestation = collector::parse_snapshot_attestation(&attestation_bytes)
        .map_err(NativeServiceError::Configuration)?;
    let report = collector::collect_report(challenge.challenge, attestation)
        .map_err(NativeServiceError::Protocol)?;
    let bytes = encode_native_report(&report)?;
    write_frame(&mut std::io::stdout().lock(), PROBE_OUTPUT_MAGIC, &bytes)?;
    Ok(())
}

#[doc(hidden)]
pub fn launcher_main() -> Result<(), NativeServiceError> {
    reject_arguments()?;
    if unsafe { libc::geteuid() } != 0 {
        return Err(NativeServiceError::NotRoot);
    }
    let config_bytes = read_secure_file(Path::new(CONFIG_PATH), 16 * 1024, 0)?;
    let config = service_config::parse_launcher_config(&config_bytes)
        .map_err(NativeServiceError::Configuration)?;
    let request_bytes = read_frame(
        &mut std::io::stdin().lock(),
        COORDINATOR_MAGIC,
        MAX_COORDINATOR_REQUEST,
    )?;
    let request = decode_probe_request(&request_bytes)?;
    validate_request(&request, &config)?;
    replay::check_and_record(
        Path::new(REPLAY_PATH),
        request.run_id,
        request.purpose,
        request.challenge,
        0,
    )?;

    let measured = path_policy::open_measured_probe(
        Path::new(PROBE_ROOT),
        PROBE_NAME,
        config.probe_sha256,
        0,
    )?;
    let challenge = encode_native_challenge(&NativeProbeChallengeV1 {
        challenge: request.challenge,
    });
    let mut child_input = Vec::with_capacity(PROBE_INPUT_MAGIC.len() + 4 + challenge.len());
    child_input.extend_from_slice(PROBE_INPUT_MAGIC);
    child_input.extend_from_slice(&(challenge.len() as u32).to_be_bytes());
    child_input.extend_from_slice(&challenge);
    let started = Instant::now();
    let deadline = started
        .checked_add(NATIVE_PROBE_TIMEOUT)
        .ok_or_else(|| NativeServiceError::Protocol("native probe deadline overflow".into()))?;
    let child_output = execute_probe(&measured, &config, &child_input, deadline)?;
    let report_bytes =
        decode_frame_bytes(&child_output, PROBE_OUTPUT_MAGIC, MAX_NATIVE_REPORT_BYTES)?;
    let report = decode_native_report(report_bytes)?;
    validate_native_report(&report, &request)?;
    if !measured.path_still_matches()? {
        return Err(NativeServiceError::Protocol(
            "installed probe changed before key use".into(),
        ));
    }
    let elapsed_ms: u64 = started
        .elapsed()
        .as_millis()
        .try_into()
        .map_err(|_| NativeServiceError::Protocol("elapsed time overflow".into()))?;
    if elapsed_ms > 30_000 {
        return Err(NativeServiceError::Protocol(
            "native probe exceeded 30-second deadline".into(),
        ));
    }
    let payload = ProbePayloadV1 {
        request,
        identity: report.identity,
        boot_id_sha256: report.boot_id_sha256,
        graphical_session_id_sha256: report.graphical_session_id_sha256,
        captured_at_unix_ms: report.captured_at_unix_ms,
        elapsed_ms,
        launcher_protocol_version: 1,
        measured_probe_sha256: measured.digest(),
    };
    let payload_cbor = encode_probe_payload(&payload);
    let key = read_signing_key(Path::new(KEY_PATH))?;
    let signature = key.sign(&crate::runner::verification::probe_signature_message(
        &payload_cbor,
    )?);
    let signed = SignedRunnerProbeV1 {
        payload_cbor_hex: hex::encode(payload_cbor),
        signature_hex: hex::encode(signature.to_bytes()),
    };
    let output = serde_json::to_vec(&signed)
        .map_err(|error| NativeServiceError::Protocol(error.to_string()))?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&(output.len() as u32).to_be_bytes())?;
    stdout.write_all(&output)?;
    stdout.flush()?;
    Ok(())
}

fn reject_arguments() -> Result<(), NativeServiceError> {
    if std::env::args_os().len() == 1 {
        Ok(())
    } else {
        Err(NativeServiceError::Arguments)
    }
}

fn validate_request(
    request: &ProbeRequestV1,
    config: &service_config::LauncherConfigV1,
) -> Result<(), NativeServiceError> {
    let optional_shape = match request.purpose {
        ProbePurpose::Enroll => {
            request.enrollment_commitment.is_none()
                && request.final_lock_sha256.is_none()
                && request.candidate_manifest_sha256.is_none()
        }
        ProbePurpose::PostLock => {
            request.enrollment_commitment.is_some()
                && request.final_lock_sha256.is_none()
                && request.candidate_manifest_sha256.is_none()
        }
        ProbePurpose::PreSpawn => {
            request.enrollment_commitment.is_none()
                && request.final_lock_sha256.is_some()
                && request.candidate_manifest_sha256.is_some()
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| NativeServiceError::Protocol("system clock precedes Unix epoch".into()))?
        .as_millis() as u64;
    if request.runner_id != config.runner_id
        || request.run_id == [0; 32]
        || request.challenge == [0; 32]
        || request.expected_probe_sha256 != config.probe_sha256
        || request.not_after_unix_ms != request.issued_at_unix_ms.checked_add(30_000).unwrap_or(0)
        || now < request.issued_at_unix_ms.saturating_sub(5_000)
        || now > request.not_after_unix_ms.saturating_add(5_000)
        || request.expected_snapshot_id.is_empty()
        || request.expected_snapshot_provider.is_empty()
        || request.expected_image_sha256 == [0; 32]
        || !optional_shape
    {
        return Err(NativeServiceError::Protocol(
            "coordinator request is stale, malformed, or not locally provisioned".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn execute_probe(
    measured: &path_policy::MeasuredProbe,
    _config: &service_config::LauncherConfigV1,
    input: &[u8],
    deadline: Instant,
) -> Result<Vec<u8>, NativeServiceError> {
    platform::linux::fexecve_capture(
        measured,
        input,
        MAX_NATIVE_REPORT_BYTES + 12,
        MAX_NATIVE_CHILD_STDERR_BYTES,
        deadline,
    )
    .map_err(NativeServiceError::from)
}

#[cfg(target_os = "macos")]
fn execute_probe(
    measured: &path_policy::MeasuredProbe,
    config: &service_config::LauncherConfigV1,
    input: &[u8],
    deadline: Instant,
) -> Result<Vec<u8>, NativeServiceError> {
    use std::os::unix::fs::PermissionsExt;

    match std::fs::create_dir(EXECUTION_ROOT) {
        Ok(()) => std::fs::set_permissions(EXECUTION_ROOT, std::fs::Permissions::from_mode(0o700))?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(EXECUTION_ROOT)?;
    if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o7777 != 0o700 {
        return Err(NativeServiceError::Configuration(
            "macOS execution root is not a root-owned 0700 directory".into(),
        ));
    }
    let pins = platform::macos::SecurityPins {
        designated_requirement: config
            .macos_designated_requirement
            .clone()
            .ok_or_else(|| NativeServiceError::Configuration("missing macOS requirement".into()))?,
        cdhash: config
            .macos_cdhash
            .ok_or_else(|| NativeServiceError::Configuration("missing macOS cdhash".into()))?,
    };
    platform::macos::copy_verify_posix_spawn_capture(
        measured,
        Path::new(EXECUTION_ROOT),
        &pins,
        0,
        input,
        platform::child_io::OutputLimits {
            stdout: MAX_NATIVE_REPORT_BYTES + 12,
            stderr: MAX_NATIVE_CHILD_STDERR_BYTES,
        },
        deadline,
    )
    .map_err(NativeServiceError::from)
}

fn read_frame(
    reader: &mut impl Read,
    magic: &[u8; 8],
    maximum: usize,
) -> Result<Vec<u8>, NativeServiceError> {
    let mut header = [0_u8; 12];
    reader.read_exact(&mut header)?;
    if &header[..8] != magic {
        return Err(NativeServiceError::Protocol("invalid frame magic".into()));
    }
    let length = u32::from_be_bytes(header[8..].try_into().expect("four bytes")) as usize;
    if length == 0 || length > maximum {
        return Err(NativeServiceError::Protocol("invalid frame length".into()));
    }
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn decode_frame_bytes<'a>(
    bytes: &'a [u8],
    magic: &[u8; 8],
    maximum: usize,
) -> Result<&'a [u8], NativeServiceError> {
    if bytes.len() < 12 || &bytes[..8] != magic {
        return Err(NativeServiceError::Protocol(
            "invalid child frame magic".into(),
        ));
    }
    let length = u32::from_be_bytes(bytes[8..12].try_into().expect("four bytes")) as usize;
    if length == 0 || length > maximum || bytes.len() != 12 + length {
        return Err(NativeServiceError::Protocol(
            "invalid child frame length".into(),
        ));
    }
    Ok(&bytes[12..])
}

fn write_frame(
    writer: &mut impl Write,
    magic: &[u8; 8],
    bytes: &[u8],
) -> Result<(), NativeServiceError> {
    let length: u32 = bytes
        .len()
        .try_into()
        .map_err(|_| NativeServiceError::Protocol("frame length exceeds u32".into()))?;
    writer.write_all(magic)?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()?;
    Ok(())
}

fn read_secure_file(
    path: &Path,
    maximum: usize,
    expected_uid: u32,
) -> Result<Vec<u8>, NativeServiceError> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != expected_uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
        || metadata.len() > maximum as u64
    {
        return Err(NativeServiceError::Configuration(format!(
            "{} ownership/mode/size is invalid",
            path.display()
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_signing_key(path: &Path) -> Result<SigningKey, NativeServiceError> {
    let bytes = read_secure_file(path, 65, 0)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| NativeServiceError::Configuration("runner key is not UTF-8".into()))?
        .trim_end_matches('\n');
    if text.len() != 64
        || text
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(NativeServiceError::Configuration(
            "runner key is not canonical 32-byte lowercase hex".into(),
        ));
    }
    let secret: [u8; 32] = hex::decode(text)
        .ok()
        .and_then(|value| value.try_into().ok())
        .ok_or_else(|| NativeServiceError::Configuration("runner key is malformed".into()))?;
    if secret == [0; 32] {
        return Err(NativeServiceError::Configuration(
            "runner key is all zero".into(),
        ));
    }
    Ok(SigningKey::from_bytes(&secret))
}

fn validate_native_report(
    report: &NativeProbeReportV1,
    request: &ProbeRequestV1,
) -> Result<(), crate::runner::RunnerError> {
    // The launcher binds measured facts to the request here. The coordinator separately compares
    // every identity field to the independently compiled dispatch-manifest row before enrollment.
    let earliest = request.issued_at_unix_ms.saturating_sub(5_000);
    let latest = request.not_after_unix_ms.saturating_add(5_000);
    if report.challenge != request.challenge
        || report.identity.runner_id != request.runner_id
        || report.snapshot_id != request.expected_snapshot_id
        || report.snapshot_provider != request.expected_snapshot_provider
        || report.identity.snapshot_provider != request.expected_snapshot_provider
        || report.snapshot_image_sha256 != request.expected_image_sha256
        || report.captured_at_unix_ms < earliest
        || report.captured_at_unix_ms > latest
    {
        return Err(crate::runner::RunnerError::Protocol(
            "native probe report does not bind the request and provisioned snapshot".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn native_report_fixture() -> NativeProbeReportV1 {
    NativeProbeReportV1 {
        challenge: [7; 32],
        identity: RunnerIdentityV1 {
            runner_id: "rutile-macos-arm64-v1".into(),
            machine_id_sha256: [1; 32],
            hardware_model: "Mac14,2".into(),
            cpu_model: "Apple M1".into(),
            cpu_cores: 8,
            ram_bytes: 16 * 1024 * 1024 * 1024,
            arch: "aarch64".into(),
            os_product: "macOS".into(),
            os_version: "15.5".into(),
            os_build: "24F74".into(),
            os_image: "macOS-15.5-24F74".into(),
            kernel: "Darwin 24.5.0".into(),
            display_session: "aqua".into(),
            display_socket: None,
            monitor_width_px: 2560,
            monitor_height_px: 1600,
            monitor_scale_milli: 1000,
            monitor_refresh_millihz: 60_000,
            gtk_version: None,
            webkitgtk_version: None,
            wkwebview_version: Some("620.2.4".into()),
            virtualized: false,
            virtualization_image_sha256: None,
            snapshot_provider: "apfs".into(),
        },
        boot_id_sha256: [2; 32],
        graphical_session_id_sha256: [3; 32],
        snapshot_id: "rutile-macos-arm64-v1-pristine".into(),
        snapshot_provider: "apfs".into(),
        snapshot_image_sha256: [4; 32],
        captured_at_unix_ms: 1_750_000_000_000,
    }
}

#[cfg(test)]
fn probe_request_fixture() -> crate::runner::protocol::ProbeRequestV1 {
    use crate::runner::protocol::{ProbePurpose, ProbeRequestV1};
    ProbeRequestV1 {
        purpose: ProbePurpose::Enroll,
        run_id: [5; 32],
        runner_id: "rutile-macos-arm64-v1".into(),
        challenge: [7; 32],
        issued_at_unix_ms: 1_750_000_000_000,
        not_after_unix_ms: 1_750_000_030_000,
        expected_snapshot_id: "rutile-macos-arm64-v1-pristine".into(),
        expected_snapshot_provider: "apfs".into(),
        expected_image_sha256: [4; 32],
        expected_probe_sha256: [8; 32],
        enrollment_commitment: None,
        final_lock_sha256: None,
        candidate_manifest_sha256: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::time::Duration;

    #[cfg(target_os = "macos")]
    fn macos_test_probe(
        root: &Path,
    ) -> (
        path_policy::MeasuredProbe,
        platform::macos::SecurityPins,
        u32,
    ) {
        let installed_root = root.join("installed");
        fs::create_dir(&installed_root).unwrap();
        let installed = installed_root.join("probe");
        fs::copy(std::env::current_exe().unwrap(), &installed).unwrap();
        fs::set_permissions(&installed, fs::Permissions::from_mode(0o500)).unwrap();
        let uid = fs::metadata(&installed).unwrap().uid();
        let digest = {
            let mut file = fs::File::open(&installed).unwrap();
            path_policy::hash_file(&mut file).unwrap()
        };
        let pins = platform::macos::read_security_pins(&installed).unwrap();
        let measured =
            path_policy::open_measured_probe(&installed_root, "probe", digest, uid).unwrap();
        (measured, pins, uid)
    }

    #[cfg(target_os = "linux")]
    fn linux_test_probe(root: &Path) -> path_policy::MeasuredProbe {
        let installed_root = root.join("installed");
        fs::create_dir(&installed_root).unwrap();
        let installed = installed_root.join("probe");
        fs::copy(std::env::current_exe().unwrap(), &installed).unwrap();
        fs::set_permissions(&installed, fs::Permissions::from_mode(0o500)).unwrap();
        let uid = fs::metadata(&installed).unwrap().uid();
        let digest = {
            let mut file = fs::File::open(&installed).unwrap();
            path_policy::hash_file(&mut file).unwrap()
        };
        path_policy::open_measured_probe(&installed_root, "probe", digest, uid).unwrap()
    }

    fn assert_reaped(pid_path: &Path) {
        let pid: libc::pid_t = fs::read_to_string(pid_path)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const NATIVE_PROBE_HELPER_ARGS: &[&str] = &[
        "rutile-native-probe-test-child",
        "--ignored",
        "--exact",
        "runner_native::tests::native_probe_test_child",
    ];

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const NATIVE_PROBE_NON_HANG_TEST_DEADLINE: Duration = Duration::from_secs(15);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const NATIVE_PROBE_PROMPT_BOUND: Duration = Duration::from_secs(10);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const NATIVE_PROBE_HANG_TEST_DEADLINE: Duration = Duration::from_secs(10);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const NATIVE_PROBE_HANG_PROMPT_BOUND: Duration = Duration::from_secs(12);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    #[ignore = "executed only as a real native probe child"]
    fn native_probe_test_child() {
        let mut command = String::new();
        std::io::stdin().read_to_string(&mut command).unwrap();
        let (behavior, pid_path) = command.trim().split_once('\n').unwrap();
        fs::write(pid_path, unsafe { libc::getpid() }.to_string()).unwrap();
        match behavior {
            "hang" => loop {
                std::hint::spin_loop();
            },
            "flood_stdout" => {
                std::io::stdout().write_all(&vec![b'x'; 4096]).unwrap();
            }
            "flood_stderr" => {
                std::io::stderr().write_all(&vec![b'x'; 4096]).unwrap();
            }
            "success" => {
                std::io::stdout().write_all(b"native-probe-ok").unwrap();
            }
            "invalid" => panic!("intentional native probe child failure"),
            _ => panic!("unknown native probe child behavior"),
        }
    }

    #[test]
    fn probe_report_round_trips_only_the_closed_bounded_tuple() {
        let report = native_report_fixture();
        let bytes = encode_native_report(&report).unwrap();
        assert!(bytes.len() <= MAX_NATIVE_REPORT_BYTES);
        assert_eq!(decode_native_report(&bytes).unwrap(), report);

        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode_native_report(&trailing).is_err());
        assert!(decode_native_report(&vec![0; MAX_NATIVE_REPORT_BYTES + 1]).is_err());
    }

    #[test]
    fn probe_challenge_is_fixed_and_rejects_unknown_or_trailing_fields() {
        let challenge = NativeProbeChallengeV1 { challenge: [7; 32] };
        let bytes = encode_native_challenge(&challenge);
        assert_eq!(decode_native_challenge(&bytes).unwrap(), challenge);

        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode_native_challenge(&trailing).is_err());
    }

    #[test]
    fn measured_probe_rejects_symlink_hardlink_writable_and_hash_substitution() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let canonical_root = root.path().canonicalize().unwrap();
        let probe = canonical_root.join("probe");
        fs::write(&probe, b"measured probe bytes").unwrap();
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o500)).unwrap();
        let uid = fs::metadata(&probe).unwrap().uid();
        let digest: [u8; 32] = sha2::Sha256::digest(b"measured probe bytes").into();

        let measured =
            path_policy::open_measured_probe(&canonical_root, "probe", digest, uid).unwrap();
        assert_eq!(measured.digest(), digest);

        let symlink = canonical_root.join("symlink");
        std::os::unix::fs::symlink(&probe, &symlink).unwrap();
        assert!(path_policy::open_measured_probe(&canonical_root, "symlink", digest, uid).is_err());

        let hardlink = canonical_root.join("hardlink");
        fs::hard_link(&probe, &hardlink).unwrap();
        assert!(path_policy::open_measured_probe(&canonical_root, "probe", digest, uid).is_err());
        fs::remove_file(hardlink).unwrap();

        fs::set_permissions(&probe, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(path_policy::open_measured_probe(&canonical_root, "probe", digest, uid).is_err());
        fs::set_permissions(&probe, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(path_policy::open_measured_probe(&canonical_root, "probe", [9; 32], uid).is_err());
    }

    #[test]
    fn launcher_accepts_only_a_challenge_bound_native_snapshot_report() {
        let report = native_report_fixture();
        let request = probe_request_fixture();
        validate_native_report(&report, &request).unwrap();

        let mut wrong_challenge = report.clone();
        wrong_challenge.challenge[0] ^= 1;
        assert!(validate_native_report(&wrong_challenge, &request).is_err());

        let mut wrong_snapshot = report.clone();
        wrong_snapshot.snapshot_id.push_str("-substituted");
        assert!(validate_native_report(&wrong_snapshot, &request).is_err());

        let mut wrong_provider = report.clone();
        wrong_provider.snapshot_provider = "caller-supplied".into();
        assert!(validate_native_report(&wrong_provider, &request).is_err());

        let mut wrong_image = report;
        wrong_image.snapshot_image_sha256[0] ^= 1;
        assert!(validate_native_report(&wrong_image, &request).is_err());
    }

    #[test]
    fn snapshot_attestation_is_closed_and_requires_nonzero_image_commitment() {
        let json = br#"{
          "schema":"rutile.runner-snapshot-attestation.v1",
          "runner_id":"rutile-macos-arm64-v1",
          "snapshot_id":"rutile-macos-arm64-v1-pristine",
          "snapshot_provider":"apfs",
          "snapshot_image_sha256":"0404040404040404040404040404040404040404040404040404040404040404",
          "virtualized":false,
          "virtualization_image_sha256":null
        }"#;
        let parsed = collector::parse_snapshot_attestation(json).unwrap();
        assert_eq!(parsed.snapshot_image_sha256, [4; 32]);

        let unknown = String::from_utf8(json.to_vec())
            .unwrap()
            .replace("\n        }", ",\n          \"unknown\":true\n        }");
        assert!(collector::parse_snapshot_attestation(unknown.as_bytes()).is_err());
        let zero = String::from_utf8(json.to_vec())
            .unwrap()
            .replace(&"04".repeat(32), &"00".repeat(32));
        assert!(collector::parse_snapshot_attestation(zero.as_bytes()).is_err());
    }

    #[test]
    fn platform_sources_name_the_required_native_execution_apis() {
        let linux = include_str!("runner_native/platform/linux.rs");
        let macos = include_str!("runner_native/platform/macos.rs");
        assert!(linux.contains("libc::fexecve("));
        assert!(macos.contains("SecStaticCodeCreateWithPath("));
        assert!(macos.contains("SecStaticCodeCheckValidityWithErrors("));
        assert!(macos.contains("posix_spawn("));
        assert!(!macos.contains("static mut environ"));
    }

    #[test]
    fn linux_native_fact_parsers_reject_ambiguous_monitor_and_os_inputs() {
        let os = collector::parse_linux_os_release(
            "NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nVERSION=\"24.04.2 LTS (Noble Numbat)\"\nPRETTY_NAME=\"Ubuntu 24.04.2 LTS\"\n",
        )
        .unwrap();
        assert_eq!(
            os,
            (
                "Ubuntu".into(),
                "24.04".into(),
                "24.04.2 LTS (Noble Numbat)".into(),
                "Ubuntu 24.04.2 LTS".into()
            )
        );
        assert!(collector::parse_linux_os_release("NAME=Ubuntu\n").is_err());
        assert_eq!(
            collector::parse_drm_mode("2560x1440\n").unwrap(),
            (2560, 1440)
        );
        assert!(collector::parse_drm_mode("1920x1080\n2560x1440\n").is_err());

        let mutter = "(uint32 7, [(('HDMI-1', 'Vendor', 'Product', 'Serial'), [('decoy', 9999, 8888, 144.0, 2.0, [1.0, 2.0], {'is-current': <false>}), ('current', 1920, 1080, 60.0, 1.0, [1.0], {'is-current': <true>})], {})], [(0, 0, 1.0, uint32 0, true, [('HDMI-1', 'Vendor', 'Product', 'Serial')], {})], {})";
        assert_eq!(
            collector::parse_mutter_current_state(mutter).unwrap(),
            (1920, 1080, 1000, 60_000)
        );
        let ambiguous = mutter.replace(
            "[(0, 0, 1.0, uint32 0, true, [('HDMI-1', 'Vendor', 'Product', 'Serial')], {})]",
            "[(0, 0, 1.0, uint32 0, true, [('HDMI-1', 'Vendor', 'Product', 'Serial')], {}), (0, 0, 1.0, uint32 0, false, [('HDMI-1', 'Vendor', 'Product', 'Serial')], {})]",
        );
        assert!(collector::parse_mutter_current_state(&ambiguous).is_err());
        assert_eq!(
            collector::select_linux_display_environment(
                "wayland",
                b"PATH=/usr/bin\0WAYLAND_DISPLAY=wayland-1\0"
            )
            .unwrap(),
            "wayland-1"
        );
        assert!(
            collector::select_linux_display_environment(
                "wayland",
                b"WAYLAND_DISPLAY=wayland-1\0DISPLAY=:0\0"
            )
            .is_err()
        );
        assert_eq!(
            collector::select_linux_display_environment("x11", b"DISPLAY=:3\0").unwrap(),
            ":3"
        );
    }

    #[test]
    fn root_replay_cache_persists_and_rejects_duplicate_request_tuple() {
        use crate::runner::protocol::ProbePurpose;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("replay-cache-v1");
        replay::check_and_record(&path, [1; 32], ProbePurpose::Enroll, [2; 32], unsafe {
            libc::geteuid()
        })
        .unwrap();
        assert!(
            replay::check_and_record(&path, [1; 32], ProbePurpose::Enroll, [2; 32], unsafe {
                libc::geteuid()
            },)
            .is_err()
        );
        replay::check_and_record(&path, [1; 32], ProbePurpose::PostLock, [2; 32], unsafe {
            libc::geteuid()
        })
        .unwrap();
    }

    #[test]
    fn launcher_service_config_is_closed_and_pins_key_transport_and_probe() {
        let json = br#"{
          "schema":"rutile.runner-launcher-config.v1",
          "runner_id":"rutile-macos-arm64-v1",
          "key_id":"runner-key-1",
          "probe_sha256":"0202020202020202020202020202020202020202020202020202020202020202",
          "macos_designated_requirement":"identifier \"com.rutile.runner-probe\"",
          "macos_cdhash":"0303030303030303030303030303030303030303"
        }"#;
        let config = service_config::parse_launcher_config(json).unwrap();
        assert_eq!(config.probe_sha256, [2; 32]);
        assert_eq!(config.macos_cdhash, Some([3; 20]));

        let unknown = String::from_utf8(json.to_vec())
            .unwrap()
            .replace("\n        }", ",\n          \"unknown\":true\n        }");
        assert!(service_config::parse_launcher_config(unknown.as_bytes()).is_err());

        let linux = br#"{
          "schema":"rutile.runner-launcher-config.v1",
          "runner_id":"rutile-ubuntu-wayland-v1",
          "key_id":"runner-key-2",
          "probe_sha256":"0202020202020202020202020202020202020202020202020202020202020202",
          "macos_designated_requirement":null,
          "macos_cdhash":null
        }"#;
        let linux = service_config::parse_launcher_config(linux).unwrap();
        assert_eq!(linux.runner_id, "rutile-ubuntu-wayland-v1");
    }

    #[test]
    fn linux_probe_sources_live_session_monitor_and_exact_runtime_families() {
        let collector = include_str!("runner_native/collector.rs");
        let launcher = include_str!("runner_native/platform/linux.rs");
        assert!(collector.contains("discover_linux_session"));
        assert!(collector.contains("read_linux_monitor"));
        assert!(collector.contains("pkg_version(&[\"gtk+-3.0\"])"));
        assert!(collector.contains("pkg_version(&[\"webkit2gtk-4.1\"])"));
        assert!(!collector.contains("gtk+-3.0\", \"gtk4"));
        assert!(!collector.contains("webkit2gtk-4.0"));
        assert!(!collector.contains("webkitgtk-6.0"));
        assert!(!launcher.contains("RUTILE_MONITOR_"));
        assert!(!launcher.contains("XDG_SESSION_TYPE="));
    }

    #[test]
    fn macos_probe_does_not_alias_os_build_as_wkwebview_runtime() {
        let collector = include_str!("runner_native/collector.rs");
        assert!(collector.contains("wkwebview_runtime_version"));
        assert!(!collector.contains("wkwebview_version: Some(os_build)"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_collector_populates_the_closed_host_identity_and_session_fields() {
        let attestation = collector::SnapshotAttestationV1 {
            runner_id: "rutile-macos-arm64-v1".into(),
            snapshot_id: "local-test-snapshot".into(),
            snapshot_provider: "local-test-provider".into(),
            snapshot_image_sha256: [9; 32],
            virtualized: false,
            virtualization_image_sha256: None,
        };
        let report = collector::collect_report([6; 32], attestation).unwrap();
        assert_eq!(report.challenge, [6; 32]);
        assert_ne!(report.identity.machine_id_sha256, [0; 32]);
        assert_ne!(report.boot_id_sha256, [0; 32]);
        assert_ne!(report.graphical_session_id_sha256, [0; 32]);
        assert!(!report.identity.hardware_model.is_empty());
        assert!(!report.identity.cpu_model.is_empty());
        assert!(report.identity.monitor_width_px > 0);
        assert!(report.identity.monitor_height_px > 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn security_framework_validates_designated_requirement_and_cdhash() {
        let executable = std::env::current_exe().unwrap();
        let pins = platform::macos::read_security_pins(&executable).unwrap();
        platform::macos::verify_security_pins(&executable, &pins).unwrap();

        let mut wrong = pins.clone();
        wrong.cdhash[0] ^= 1;
        assert!(platform::macos::verify_security_pins(&executable, &wrong).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_immutable_copy_posix_spawn_rejects_replacement_races() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let installed_root = root.join("installed");
        let execution_root = root.join("run");
        fs::create_dir(&installed_root).unwrap();
        fs::create_dir(&execution_root).unwrap();
        let installed = installed_root.join("probe");
        fs::copy(std::env::current_exe().unwrap(), &installed).unwrap();
        fs::set_permissions(&installed, fs::Permissions::from_mode(0o500)).unwrap();
        let uid = fs::metadata(&installed).unwrap().uid();
        let digest = {
            let mut file = fs::File::open(&installed).unwrap();
            path_policy::hash_file(&mut file).unwrap()
        };
        let pins = platform::macos::read_security_pins(&installed).unwrap();
        let measured =
            path_policy::open_measured_probe(&installed_root, "probe", digest, uid).unwrap();
        platform::macos::copy_verify_posix_spawn(
            &measured,
            &execution_root,
            &pins,
            uid,
            &platform::macos::NoRace,
            &["probe", "--list"],
        )
        .unwrap();

        for phase in [
            platform::macos::RacePhase::BeforeCopy,
            platform::macos::RacePhase::DuringCopy,
            platform::macos::RacePhase::BeforeSpawn,
        ] {
            let _ = fs::remove_file(&installed);
            let _ = fs::remove_file(installed.with_extension("raced"));
            fs::copy(std::env::current_exe().unwrap(), &installed).unwrap();
            fs::set_permissions(&installed, fs::Permissions::from_mode(0o500)).unwrap();
            let measured =
                path_policy::open_measured_probe(&installed_root, "probe", digest, uid).unwrap();
            let hook = platform::macos::ReplacePathAt { phase };
            assert!(
                platform::macos::copy_verify_posix_spawn(
                    &measured,
                    &execution_root,
                    &pins,
                    uid,
                    &hook,
                    &["probe", "--list"],
                )
                .is_err()
            );
            let _ = fs::remove_file(&installed);
            let _ = fs::remove_file(installed.with_extension("raced"));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_native_probe_hang_hits_total_deadline_and_is_reaped() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let execution_root = root.join("run");
        fs::create_dir(&execution_root).unwrap();
        let (measured, pins, uid) = macos_test_probe(&root);
        let pid_path = root.join("hung-child.pid");
        let command = format!("hang\n{}\n", pid_path.display());
        let started = Instant::now();
        let error = platform::macos::copy_verify_posix_spawn_capture_for_test(
            &measured,
            &execution_root,
            &pins,
            uid,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            platform::child_io::OutputLimits {
                stdout: 1024,
                stderr: 1024,
            },
            started + NATIVE_PROBE_HANG_TEST_DEADLINE,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < NATIVE_PROBE_HANG_PROMPT_BOUND);
        assert_reaped(&pid_path);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_native_probe_stdout_flood_is_bounded_and_reaped() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let execution_root = root.join("run");
        fs::create_dir(&execution_root).unwrap();
        let (measured, pins, uid) = macos_test_probe(&root);
        let pid_path = root.join("stdout-flood-child.pid");
        let command = format!("flood_stdout\n{}\n", pid_path.display());
        let started = Instant::now();
        let error = platform::macos::copy_verify_posix_spawn_capture_for_test(
            &measured,
            &execution_root,
            &pins,
            uid,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            platform::child_io::OutputLimits {
                stdout: 1024,
                stderr: 1024,
            },
            started + NATIVE_PROBE_NON_HANG_TEST_DEADLINE,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(started.elapsed() < NATIVE_PROBE_PROMPT_BOUND);
        assert_reaped(&pid_path);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_native_probe_stderr_flood_is_bounded_and_reaped() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let execution_root = root.join("run");
        fs::create_dir(&execution_root).unwrap();
        let (measured, pins, uid) = macos_test_probe(&root);
        let pid_path = root.join("stderr-flood-child.pid");
        let command = format!("flood_stderr\n{}\n", pid_path.display());
        let started = Instant::now();
        let error = platform::macos::copy_verify_posix_spawn_capture_for_test(
            &measured,
            &execution_root,
            &pins,
            uid,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            platform::child_io::OutputLimits {
                stdout: 8192,
                stderr: 1024,
            },
            started + NATIVE_PROBE_NON_HANG_TEST_DEADLINE,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(started.elapsed() < NATIVE_PROBE_PROMPT_BOUND);
        assert_reaped(&pid_path);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_native_probe_normal_success_returns_output_and_reaps() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let execution_root = root.join("run");
        fs::create_dir(&execution_root).unwrap();
        let (measured, pins, uid) = macos_test_probe(&root);
        let pid_path = root.join("successful-child.pid");
        let command = format!("success\n{}\n", pid_path.display());
        let started = Instant::now();
        let output = platform::macos::copy_verify_posix_spawn_capture_for_test(
            &measured,
            &execution_root,
            &pins,
            uid,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            platform::child_io::OutputLimits {
                stdout: 8192,
                stderr: 8192,
            },
            started + NATIVE_PROBE_NON_HANG_TEST_DEADLINE,
        )
        .unwrap();

        assert!(
            output
                .windows(b"native-probe-ok".len())
                .any(|window| window == b"native-probe-ok")
        );
        assert!(started.elapsed() < NATIVE_PROBE_PROMPT_BOUND);
        assert_reaped(&pid_path);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_native_probe_invalid_exit_is_reaped() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let execution_root = root.join("run");
        fs::create_dir(&execution_root).unwrap();
        let (measured, pins, uid) = macos_test_probe(&root);
        let pid_path = root.join("invalid-child.pid");
        let command = format!("invalid\n{}\n", pid_path.display());
        let error = platform::macos::copy_verify_posix_spawn_capture_for_test(
            &measured,
            &execution_root,
            &pins,
            uid,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            platform::child_io::OutputLimits {
                stdout: 8192,
                stderr: 8192,
            },
            Instant::now() + NATIVE_PROBE_NON_HANG_TEST_DEADLINE,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_reaped(&pid_path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_native_probe_hang_hits_total_deadline_and_is_reaped() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let measured = linux_test_probe(&root);
        let pid_path = root.join("hung-child.pid");
        let command = format!("hang\n{}\n", pid_path.display());
        let started = Instant::now();
        let error = platform::linux::fexecve_capture_for_test(
            &measured,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            1024,
            1024,
            started + NATIVE_PROBE_HANG_TEST_DEADLINE,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < NATIVE_PROBE_HANG_PROMPT_BOUND);
        assert_reaped(&pid_path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_native_probe_stdout_flood_is_bounded_and_reaped() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let measured = linux_test_probe(&root);
        let pid_path = root.join("stdout-flood-child.pid");
        let command = format!("flood_stdout\n{}\n", pid_path.display());
        let started = Instant::now();
        let error = platform::linux::fexecve_capture_for_test(
            &measured,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            1024,
            1024,
            started + NATIVE_PROBE_NON_HANG_TEST_DEADLINE,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(started.elapsed() < NATIVE_PROBE_PROMPT_BOUND);
        assert_reaped(&pid_path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_native_probe_stderr_flood_is_bounded_and_reaped() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let measured = linux_test_probe(&root);
        let pid_path = root.join("stderr-flood-child.pid");
        let command = format!("flood_stderr\n{}\n", pid_path.display());
        let started = Instant::now();
        let error = platform::linux::fexecve_capture_for_test(
            &measured,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            8192,
            1024,
            started + NATIVE_PROBE_NON_HANG_TEST_DEADLINE,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(started.elapsed() < NATIVE_PROBE_PROMPT_BOUND);
        assert_reaped(&pid_path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_native_probe_normal_success_returns_output_and_reaps() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let measured = linux_test_probe(&root);
        let pid_path = root.join("successful-child.pid");
        let command = format!("success\n{}\n", pid_path.display());
        let started = Instant::now();
        let output = platform::linux::fexecve_capture_for_test(
            &measured,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            8192,
            8192,
            started + NATIVE_PROBE_NON_HANG_TEST_DEADLINE,
        )
        .unwrap();

        assert!(
            output
                .windows(b"native-probe-ok".len())
                .any(|window| window == b"native-probe-ok")
        );
        assert!(started.elapsed() < NATIVE_PROBE_PROMPT_BOUND);
        assert_reaped(&pid_path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_native_probe_invalid_exit_is_reaped() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let measured = linux_test_probe(&root);
        let pid_path = root.join("invalid-child.pid");
        let command = format!("invalid\n{}\n", pid_path.display());
        let error = platform::linux::fexecve_capture_for_test(
            &measured,
            NATIVE_PROBE_HELPER_ARGS,
            command.as_bytes(),
            8192,
            8192,
            Instant::now() + NATIVE_PROBE_NON_HANG_TEST_DEADLINE,
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_reaped(&pid_path);
    }
}
