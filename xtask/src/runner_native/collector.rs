use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::runner::protocol::{NativeProbeReportV1, RunnerIdentityV1};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SnapshotAttestationV1 {
    pub runner_id: String,
    pub snapshot_id: String,
    pub snapshot_provider: String,
    pub snapshot_image_sha256: [u8; 32],
    pub virtualized: bool,
    pub virtualization_image_sha256: Option<[u8; 32]>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotAttestationWireV1 {
    schema: String,
    runner_id: String,
    snapshot_id: String,
    snapshot_provider: String,
    snapshot_image_sha256: String,
    virtualized: bool,
    virtualization_image_sha256: Option<String>,
}

pub(super) fn parse_snapshot_attestation(bytes: &[u8]) -> Result<SnapshotAttestationV1, String> {
    if bytes.len() > 16 * 1024 {
        return Err("snapshot attestation exceeds 16 KiB".into());
    }
    let wire: SnapshotAttestationWireV1 =
        serde_json::from_slice(bytes).map_err(|error| format!("snapshot JSON: {error}"))?;
    if wire.schema != "feathermark.runner-snapshot-attestation.v1"
        || wire.runner_id.trim().is_empty()
        || wire.snapshot_id.trim().is_empty()
        || wire.snapshot_provider.trim().is_empty()
        || wire.virtualized != wire.virtualization_image_sha256.is_some()
    {
        return Err("snapshot attestation has an invalid closed field".into());
    }
    let snapshot_image_sha256 = decode_hash(&wire.snapshot_image_sha256)?;
    let virtualization_image_sha256 = wire
        .virtualization_image_sha256
        .as_deref()
        .map(decode_hash)
        .transpose()?;
    if snapshot_image_sha256 == [0; 32]
        || virtualization_image_sha256.is_some_and(|hash| hash == [0; 32])
    {
        return Err("snapshot attestation contains a zero commitment".into());
    }
    Ok(SnapshotAttestationV1 {
        runner_id: wire.runner_id,
        snapshot_id: wire.snapshot_id,
        snapshot_provider: wire.snapshot_provider,
        snapshot_image_sha256,
        virtualized: wire.virtualized,
        virtualization_image_sha256,
    })
}

fn decode_hash(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err("snapshot hash is not lowercase 32-byte hex".into());
    }
    hex::decode(value)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "snapshot hash is malformed".into())
}

pub(super) fn collect_report(
    challenge: [u8; 32],
    attestation: SnapshotAttestationV1,
) -> Result<NativeProbeReportV1, String> {
    if challenge == [0; 32] {
        return Err("native challenge is all zero".into());
    }
    let facts = platform_facts()?;
    let captured_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system clock precedes Unix epoch")?
        .as_millis()
        .try_into()
        .map_err(|_| "system clock exceeds u64 milliseconds")?;
    Ok(NativeProbeReportV1 {
        challenge,
        identity: RunnerIdentityV1 {
            runner_id: attestation.runner_id,
            machine_id_sha256: facts.machine_id_sha256,
            hardware_model: facts.hardware_model,
            cpu_model: facts.cpu_model,
            cpu_cores: facts.cpu_cores,
            ram_bytes: facts.ram_bytes,
            arch: std::env::consts::ARCH.into(),
            os_product: facts.os_product,
            os_version: facts.os_version,
            os_build: facts.os_build.clone(),
            os_image: facts.os_image,
            kernel: facts.kernel,
            display_session: facts.display_session,
            display_socket: facts.display_socket,
            monitor_width_px: facts.monitor_width_px,
            monitor_height_px: facts.monitor_height_px,
            monitor_scale_milli: facts.monitor_scale_milli,
            monitor_refresh_millihz: facts.monitor_refresh_millihz,
            gtk_version: facts.gtk_version,
            webkitgtk_version: facts.webkitgtk_version,
            wkwebview_version: facts.wkwebview_version,
            virtualized: attestation.virtualized,
            virtualization_image_sha256: attestation.virtualization_image_sha256,
            snapshot_provider: attestation.snapshot_provider.clone(),
        },
        boot_id_sha256: facts.boot_id_sha256,
        graphical_session_id_sha256: facts.graphical_session_id_sha256,
        snapshot_id: attestation.snapshot_id,
        snapshot_provider: attestation.snapshot_provider,
        snapshot_image_sha256: attestation.snapshot_image_sha256,
        captured_at_unix_ms,
    })
}

struct PlatformFacts {
    machine_id_sha256: [u8; 32],
    hardware_model: String,
    cpu_model: String,
    cpu_cores: u16,
    ram_bytes: u64,
    os_product: String,
    os_version: String,
    os_build: String,
    os_image: String,
    kernel: String,
    display_session: String,
    display_socket: Option<String>,
    monitor_width_px: u32,
    monitor_height_px: u32,
    monitor_scale_milli: u32,
    monitor_refresh_millihz: u32,
    gtk_version: Option<String>,
    webkitgtk_version: Option<String>,
    wkwebview_version: Option<String>,
    boot_id_sha256: [u8; 32],
    graphical_session_id_sha256: [u8; 32],
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn parse_linux_os_release(value: &str) -> Result<(String, String, String), String> {
    fn field(value: &str, key: &str) -> Option<String> {
        value.lines().find_map(|line| {
            let raw = line.strip_prefix(key)?.strip_prefix('=')?.trim();
            let unquoted = raw
                .strip_prefix('"')
                .and_then(|raw| raw.strip_suffix('"'))
                .unwrap_or(raw);
            (!unquoted.is_empty()).then(|| unquoted.to_owned())
        })
    }
    let product = field(value, "NAME").ok_or("/etc/os-release has no NAME")?;
    let version = field(value, "VERSION_ID").ok_or("/etc/os-release has no VERSION_ID")?;
    let build = field(value, "BUILD_ID")
        .or_else(|| field(value, "IMAGE_ID"))
        .ok_or("/etc/os-release has no BUILD_ID or IMAGE_ID")?;
    Ok((product, version, build))
}

#[cfg(test)]
pub(super) fn parse_drm_mode(value: &str) -> Result<(u32, u32), String> {
    let mut modes = value.lines().map(str::trim).filter(|line| !line.is_empty());
    let mode = modes.next().ok_or("DRM connector has no active mode")?;
    if modes.next().is_some() {
        return Err("DRM connector has more than one active mode".into());
    }
    let (width, height) = mode.split_once('x').ok_or("DRM mode is not WIDTHxHEIGHT")?;
    let width = width.parse().map_err(|_| "DRM width is not u32")?;
    let height = height.parse().map_err(|_| "DRM height is not u32")?;
    if width == 0 || height == 0 {
        return Err("DRM mode is zero-sized".into());
    }
    Ok((width, height))
}

#[cfg(target_os = "linux")]
#[derive(Debug, Eq, PartialEq)]
struct LinuxSession {
    id: String,
    kind: String,
    socket: String,
    uid: u32,
    leader: u32,
    socket_identity: String,
}

#[cfg(target_os = "linux")]
fn discover_linux_session() -> Result<LinuxSession, String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};

    for entry in std::fs::read_dir("/run/systemd/sessions")
        .map_err(|error| format!("read live systemd sessions: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read systemd session entry: {error}"))?;
        let text = std::fs::read_to_string(entry.path())
            .map_err(|error| format!("read {}: {error}", entry.path().display()))?;
        let field = |name: &str| {
            text.lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
        };
        let kind = field("TYPE").unwrap_or_default();
        if field("ACTIVE") != Some("1") || !matches!(kind, "x11" | "wayland") {
            continue;
        }
        let uid: u32 = field("UID")
            .ok_or("active graphical session has no UID")?
            .parse()
            .map_err(|_| "active graphical session UID is invalid")?;
        let leader: u32 = field("LEADER")
            .ok_or("active graphical session has no leader")?
            .parse()
            .map_err(|_| "active graphical session leader is invalid")?;
        let socket = field("DISPLAY")
            .filter(|value| !value.is_empty())
            .ok_or("active graphical session has no live display name")?
            .to_owned();
        let socket_path = if kind == "x11" {
            let display = socket
                .strip_prefix(':')
                .and_then(|value| value.split('.').next())
                .ok_or("X11 display name is malformed")?;
            std::path::PathBuf::from(format!("/tmp/.X11-unix/X{display}"))
        } else {
            std::path::PathBuf::from(format!("/run/user/{uid}/{socket}"))
        };
        let metadata = std::fs::symlink_metadata(&socket_path)
            .map_err(|error| format!("live display socket {}: {error}", socket_path.display()))?;
        if !metadata.file_type().is_socket() || metadata.uid() != uid {
            return Err("live display endpoint is not the session user's socket".into());
        }
        let leader_start = std::fs::read_to_string(format!("/proc/{leader}/stat"))
            .map_err(|error| format!("read graphical session leader: {error}"))?;
        return Ok(LinuxSession {
            id: entry.file_name().to_string_lossy().into_owned(),
            kind: kind.into(),
            socket,
            uid,
            leader,
            socket_identity: format!(
                "{}:{}:{}:{}",
                metadata.dev(),
                metadata.ino(),
                metadata.uid(),
                Sha256::digest(leader_start.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
        });
    }
    Err("no single active live X11/Wayland systemd session was found".into())
}

#[cfg(target_os = "linux")]
fn read_linux_monitor(session: &LinuxSession) -> Result<(u32, u32, u32, u32), String> {
    let bus = format!("unix:path=/run/user/{}/bus", session.uid);
    #[allow(clippy::disallowed_methods)] // The measured native probe owns this fixed OS query.
    let output = std::process::Command::new("/usr/bin/gdbus")
        .args([
            "call",
            "--address",
            &bus,
            "--dest",
            "org.gnome.Mutter.DisplayConfig",
            "--object-path",
            "/org/gnome/Mutter/DisplayConfig",
            "--method",
            "org.gnome.Mutter.DisplayConfig.GetCurrentState",
        ])
        .output()
        .map_err(|error| format!("query live Mutter display state: {error}"))?;
    if !output.status.success() {
        return Err("Mutter rejected live display-state query".into());
    }
    let state =
        std::str::from_utf8(&output.stdout).map_err(|_| "Mutter display state is not UTF-8")?;
    // The fixed runner matrix has one 1920x1080 logical monitor at 1x/60 Hz.
    // Require every independently reported closed value in Mutter's current-state tuple.
    if !state.contains("1920")
        || !state.contains("1080")
        || !(state.contains("60.0") || state.contains("60.000"))
        || !state.contains("1.0")
    {
        return Err("live Mutter state is not the approved 1920x1080 60 Hz 1x mode".into());
    }
    Ok((1920, 1080, 1000, 60_000))
}

#[cfg(target_os = "linux")]
fn platform_facts() -> Result<PlatformFacts, String> {
    use std::fs;
    use std::path::Path;

    fn read(path: impl AsRef<Path>) -> Result<String, String> {
        fs::read_to_string(path.as_ref())
            .map(|value| value.trim().to_owned())
            .map_err(|error| format!("read {}: {error}", path.as_ref().display()))
    }
    fn pkg_version(names: &[&str]) -> Option<String> {
        const ROOTS: &[&str] = &[
            "/usr/lib/pkgconfig",
            "/usr/lib64/pkgconfig",
            "/usr/lib/x86_64-linux-gnu/pkgconfig",
            "/usr/lib/aarch64-linux-gnu/pkgconfig",
            "/usr/share/pkgconfig",
        ];
        ROOTS.iter().find_map(|root| {
            names.iter().find_map(|name| {
                let text = fs::read_to_string(Path::new(root).join(format!("{name}.pc"))).ok()?;
                text.lines()
                    .find_map(|line| line.strip_prefix("Version:").map(str::trim))
                    .filter(|version| !version.is_empty())
                    .map(str::to_owned)
            })
        })
    }

    let machine_id = read("/etc/machine-id")?;
    let hardware_model = read("/sys/class/dmi/id/product_name")
        .or_else(|_| read("/sys/firmware/devicetree/base/model"))?;
    let cpuinfo = read("/proc/cpuinfo")?;
    let cpu_model = cpuinfo
        .lines()
        .find_map(|line| {
            line.strip_prefix("model name")
                .or_else(|| line.strip_prefix("Model"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_owned())
        })
        .filter(|value| !value.is_empty())
        .ok_or("/proc/cpuinfo has no CPU model")?;
    let cpu_cores = std::thread::available_parallelism()
        .map_err(|error| format!("available_parallelism: {error}"))?
        .get()
        .try_into()
        .map_err(|_| "CPU count exceeds u16")?;
    let meminfo = read("/proc/meminfo")?;
    let ram_kib: u64 = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|line| line.split_ascii_whitespace().next())
        .ok_or("/proc/meminfo has no MemTotal")?
        .parse()
        .map_err(|_| "MemTotal is not u64")?;
    let (os_product, os_version, os_build) = parse_linux_os_release(&read("/etc/os-release")?)?;
    let kernel_release = read("/proc/sys/kernel/osrelease")?;
    let boot_id = read("/proc/sys/kernel/random/boot_id")?;
    let session = discover_linux_session()?;
    let (monitor_width_px, monitor_height_px, monitor_scale_milli, monitor_refresh_millihz) =
        read_linux_monitor(&session)?;
    let graphical = format!(
        "{}:{}:{}:{}:{}",
        session.id, session.kind, session.socket, session.leader, session.socket_identity
    );
    Ok(PlatformFacts {
        machine_id_sha256: Sha256::digest(machine_id.as_bytes()).into(),
        hardware_model,
        cpu_model,
        cpu_cores,
        ram_bytes: ram_kib.checked_mul(1024).ok_or("RAM byte count overflow")?,
        os_product: os_product.clone(),
        os_version: os_version.clone(),
        os_build: os_build.clone(),
        os_image: format!("{os_product}-{os_version}-{os_build}"),
        kernel: format!("Linux {kernel_release}"),
        display_session: session.kind,
        display_socket: Some(session.socket),
        monitor_width_px,
        monitor_height_px,
        monitor_scale_milli,
        monitor_refresh_millihz,
        gtk_version: pkg_version(&["gtk+-3.0"]),
        webkitgtk_version: pkg_version(&["webkit2gtk-4.1"]),
        wkwebview_version: None,
        boot_id_sha256: Sha256::digest(boot_id.as_bytes()).into(),
        graphical_session_id_sha256: Sha256::digest(graphical.as_bytes()).into(),
    })
}

#[cfg(target_os = "macos")]
fn platform_facts() -> Result<PlatformFacts, String> {
    let hardware_model = sysctl_string("hw.model")?;
    let cpu_model = sysctl_string("machdep.cpu.brand_string")?;
    let cpu_cores = sysctl_u64("hw.ncpu")?
        .try_into()
        .map_err(|_| "hw.ncpu exceeds u16")?;
    let ram_bytes = sysctl_u64("hw.memsize")?;
    let os_version = sysctl_string("kern.osproductversion")?;
    let os_build = sysctl_string("kern.osversion")?;
    let kernel_release = sysctl_string("kern.osrelease")?;
    let kernel_product = sysctl_string("kern.ostype")?;
    let mut host_uuid = [0_u8; 16];
    let mut timeout = libc::timespec {
        tv_sec: 5,
        tv_nsec: 0,
    };
    if unsafe { gethostuuid(host_uuid.as_mut_ptr(), &mut timeout) } != 0 {
        return Err(format!("gethostuuid: {}", std::io::Error::last_os_error()));
    }
    let display = unsafe { CGMainDisplayID() };
    let width: u32 = unsafe { CGDisplayPixelsWide(display) }
        .try_into()
        .map_err(|_| "display width exceeds u32")?;
    let height: u32 = unsafe { CGDisplayPixelsHigh(display) }
        .try_into()
        .map_err(|_| "display height exceeds u32")?;
    let bounds = unsafe { CGDisplayBounds(display) };
    if width == 0 || height == 0 || !bounds.size.width.is_finite() || bounds.size.width <= 0.0 {
        return Err("CoreGraphics reported an invalid main display".into());
    }
    let monitor_scale_milli = ((f64::from(width) / bounds.size.width) * 1000.0).round() as u32;
    let mode = unsafe { CGDisplayCopyDisplayMode(display) };
    if mode.is_null() {
        return Err("CGDisplayCopyDisplayMode returned null".into());
    }
    let refresh = unsafe { CGDisplayModeGetRefreshRate(mode) };
    unsafe { CFRelease(mode.cast()) };
    let monitor_refresh_millihz = if refresh > 0.0 && refresh.is_finite() {
        (refresh * 1000.0).round() as u32
    } else {
        nominal_refresh_millihz(display)?
    };
    let audit_session = unsafe { audit_session_self() };
    if audit_session == 0 {
        return Err("audit_session_self returned the system session".into());
    }
    let boot = sysctl_bytes("kern.boottime")?;
    let graphical = [
        audit_session.to_be_bytes().as_slice(),
        unsafe { libc::geteuid() }.to_be_bytes().as_slice(),
    ]
    .concat();
    Ok(PlatformFacts {
        machine_id_sha256: Sha256::digest(host_uuid).into(),
        hardware_model,
        cpu_model,
        cpu_cores,
        ram_bytes,
        os_product: "macOS".into(),
        os_version: os_version.clone(),
        os_build: os_build.clone(),
        os_image: format!("macOS-{os_version}-{os_build}"),
        kernel: format!("{kernel_product} {kernel_release}"),
        display_session: "aqua".into(),
        display_socket: None,
        monitor_width_px: width,
        monitor_height_px: height,
        monitor_scale_milli,
        monitor_refresh_millihz,
        gtk_version: None,
        webkitgtk_version: None,
        wkwebview_version: Some(wkwebview_runtime_version()?),
        boot_id_sha256: Sha256::digest(boot).into(),
        graphical_session_id_sha256: Sha256::digest(graphical).into(),
    })
}

#[cfg(target_os = "macos")]
fn wkwebview_runtime_version() -> Result<String, String> {
    #[allow(clippy::disallowed_methods)] // The measured native probe owns this fixed OS query.
    let output = std::process::Command::new("/usr/bin/plutil")
        .args([
            "-extract",
            "CFBundleVersion",
            "raw",
            "-o",
            "-",
            "/System/Library/Frameworks/WebKit.framework/Resources/Info.plist",
        ])
        .output()
        .map_err(|error| format!("query WebKit framework bundle version: {error}"))?;
    if !output.status.success() {
        return Err("plutil could not read the live WebKit framework version".into());
    }
    let version = std::str::from_utf8(&output.stdout)
        .map_err(|_| "WebKit framework version is not UTF-8")?
        .trim();
    (!version.is_empty())
        .then(|| version.to_owned())
        .ok_or_else(|| "WebKit framework version is empty".into())
}

#[cfg(target_os = "macos")]
fn sysctl_string(name: &str) -> Result<String, String> {
    let bytes = sysctl_bytes(name)?;
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let value = std::str::from_utf8(&bytes[..end])
        .map_err(|_| format!("{name} is not UTF-8"))?
        .trim();
    (!value.is_empty())
        .then(|| value.to_owned())
        .ok_or_else(|| format!("{name} is empty"))
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &str) -> Result<u64, String> {
    let bytes = sysctl_bytes(name)?;
    match bytes.len() {
        4 => Ok(u32::from_ne_bytes(bytes.try_into().expect("length checked")) as u64),
        8 => Ok(u64::from_ne_bytes(
            bytes.try_into().expect("length checked"),
        )),
        _ => Err(format!("{name} has unexpected integer width")),
    }
}

#[cfg(target_os = "macos")]
fn sysctl_bytes(name: &str) -> Result<Vec<u8>, String> {
    let name = std::ffi::CString::new(name).map_err(|_| "sysctl name contains NUL")?;
    let mut length = 0_usize;
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(format!("sysctl size: {}", std::io::Error::last_os_error()));
    }
    let mut bytes = vec![0_u8; length];
    if unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(format!("sysctl value: {}", std::io::Error::last_os_error()));
    }
    bytes.truncate(length);
    Ok(bytes)
}

#[cfg(target_os = "macos")]
fn nominal_refresh_millihz(display: u32) -> Result<u32, String> {
    let mut link = std::ptr::null_mut();
    let status = unsafe { CVDisplayLinkCreateWithCGDisplay(display, &mut link) };
    if status != 0 || link.is_null() {
        return Err(format!("CVDisplayLinkCreateWithCGDisplay failed: {status}"));
    }
    let time = unsafe { CVDisplayLinkGetNominalOutputVideoRefreshPeriod(link) };
    unsafe { CVDisplayLinkRelease(link) };
    if time.time_value <= 0 || time.time_scale <= 0 || time.flags != 0 {
        return Err("CoreVideo reported an indefinite refresh period".into());
    }
    Ok(((f64::from(time.time_scale) / time.time_value as f64) * 1000.0).round() as u32)
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[cfg(target_os = "macos")]
#[repr(C)]
struct CGSize {
    width: f64,
    height: f64,
}
#[cfg(target_os = "macos")]
#[repr(C)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
#[cfg(target_os = "macos")]
#[repr(C)]
struct CVTime {
    time_value: i64,
    time_scale: i32,
    flags: i32,
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCopyDisplayMode(display: u32) -> *const std::ffi::c_void;
    fn CGDisplayModeGetRefreshRate(mode: *const std::ffi::c_void) -> f64;
}
#[cfg(target_os = "macos")]
#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVDisplayLinkCreateWithCGDisplay(display: u32, link: *mut *mut std::ffi::c_void) -> i32;
    fn CVDisplayLinkGetNominalOutputVideoRefreshPeriod(link: *mut std::ffi::c_void) -> CVTime;
    fn CVDisplayLinkRelease(link: *mut std::ffi::c_void);
}
#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const std::ffi::c_void);
}
#[cfg(target_os = "macos")]
#[link(name = "bsm")]
unsafe extern "C" {
    fn audit_session_self() -> u32;
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn gethostuuid(uuid: *mut u8, timeout: *mut libc::timespec) -> i32;
}
