//! Fail-closed release artifact quarantine and inspection policy.
//!
//! The inspector never mutates or extracts an artifact. Regular files are
//! scanned as bounded streams. Directory packages are walked without following
//! links and with fixed entry/byte ceilings. ditto-produced `.app.zip` archives
//! are scanned via a bounded zip reader that decompresses each entry to a
//! bounded in-memory buffer (never to disk). Other opaque archive formats
//! remain publication-ineligible until a bounded format reader is available.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;
use zip::ZipArchive;

const MAX_POLICY_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct PolicyPaths {
    pub quarantine: PathBuf,
    pub policy: PathBuf,
}

impl PolicyPaths {
    pub fn repository_defaults() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is a workspace member");
        Self {
            quarantine: root.join("release/quarantine-v1.json"),
            policy: root.join("release/policy/artifact-inspector-v1.toml"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionMode {
    Candidate,
    Package,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingCode {
    QuarantinedHash,
    TestControlMarker,
    PrivateBuildPath,
    CredentialMarker,
    ForbiddenPattern,
    EntryLimitExceeded,
    ByteLimitExceeded,
    LinkRejected,
    UnsupportedArchive,
    ExecutableCountMismatch,
    ManifestMissing,
    ManifestMalformed,
    VersionMismatch,
    LicenseMismatch,
    SourceCommitMismatch,
    PlatformMetadataMissing,
    ProvenanceMissing,
    ProvenanceInvalid,
}

impl FindingCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QuarantinedHash => "quarantined_hash",
            Self::TestControlMarker => "test_control_marker",
            Self::PrivateBuildPath => "private_build_path",
            Self::CredentialMarker => "credential_marker",
            Self::ForbiddenPattern => "forbidden_pattern",
            Self::EntryLimitExceeded => "entry_limit_exceeded",
            Self::ByteLimitExceeded => "byte_limit_exceeded",
            Self::LinkRejected => "link_rejected",
            Self::UnsupportedArchive => "unsupported_archive",
            Self::ExecutableCountMismatch => "executable_count_mismatch",
            Self::ManifestMissing => "manifest_missing",
            Self::ManifestMalformed => "manifest_malformed",
            Self::VersionMismatch => "version_mismatch",
            Self::LicenseMismatch => "license_mismatch",
            Self::SourceCommitMismatch => "source_commit_mismatch",
            Self::PlatformMetadataMissing => "platform_metadata_missing",
            Self::ProvenanceMissing => "provenance_missing",
            Self::ProvenanceInvalid => "provenance_invalid",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InspectionFinding {
    pub code: FindingCode,
    pub subject: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct InspectionReport {
    pub schema: &'static str,
    pub inspector_version: &'static str,
    pub inspection_mode: InspectionMode,
    pub artifact_kind: &'static str,
    pub policy_sha256: String,
    pub quarantine_sha256: String,
    pub artifact_sha256: Option<String>,
    pub production_provenance_sha256: Option<String>,
    pub complete_scan: bool,
    pub accepted: bool,
    pub publication_authorized: bool,
    pub entries_scanned: u64,
    pub uncompressed_bytes_scanned: u64,
    pub findings: Vec<InspectionFinding>,
}

impl InspectionReport {
    pub fn has(&self, code: FindingCode) -> bool {
        self.findings.iter().any(|finding| finding.code == code)
    }
}

#[derive(Debug, Error)]
pub enum InspectorError {
    #[error("policy file must be a regular non-symlink file: {0}")]
    UnsafePolicyFile(PathBuf),
    #[error("policy file exceeds {MAX_POLICY_BYTES} bytes: {0}")]
    PolicyTooLarge(PathBuf),
    #[error("invalid quarantine registry: {0}")]
    InvalidQuarantine(String),
    #[error("invalid artifact inspector policy: {0}")]
    InvalidPolicy(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuarantineRegistry {
    schema: String,
    version: u32,
    entries: Vec<QuarantineEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuarantineEntry {
    sha256: String,
    artifact: String,
    reason: String,
    discovered_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectorPolicy {
    schema: String,
    version: u32,
    max_entries: u64,
    max_uncompressed_bytes: u64,
    expected_license: String,
    #[serde(default)]
    expected_version: Option<String>,
    #[serde(default)]
    expected_source_commit: Option<String>,
    forbidden_patterns: Vec<String>,
    test_control_environment: Vec<String>,
}

#[derive(Debug)]
pub struct ArtifactInspector {
    quarantine: HashSet<String>,
    policy: InspectorPolicy,
    policy_sha256: String,
    quarantine_sha256: String,
}

impl ArtifactInspector {
    pub fn load(paths: &PolicyPaths) -> Result<Self, InspectorError> {
        let quarantine_bytes = read_policy_file(&paths.quarantine)?;
        let policy_bytes = read_policy_file(&paths.policy)?;
        let quarantine: QuarantineRegistry = serde_json::from_slice(&quarantine_bytes)?;
        validate_quarantine(&quarantine)?;
        let policy: InspectorPolicy = toml::from_str(
            std::str::from_utf8(&policy_bytes)
                .map_err(|_| InspectorError::InvalidPolicy("policy is not UTF-8".into()))?,
        )?;
        validate_policy(&policy)?;
        Ok(Self {
            quarantine: quarantine
                .entries
                .into_iter()
                .map(|entry| entry.sha256)
                .collect(),
            policy,
            policy_sha256: hex::encode(Sha256::digest(&policy_bytes)),
            quarantine_sha256: hex::encode(Sha256::digest(&quarantine_bytes)),
        })
    }

    pub fn inspect(
        &self,
        artifact: &Path,
        mode: InspectionMode,
        provenance: Option<&Path>,
    ) -> InspectionReport {
        let mut report = InspectionReport {
            schema: "rutile.artifact-inspection.v1",
            inspector_version: "artifact-inspect-v1",
            inspection_mode: mode,
            artifact_kind: "unknown",
            policy_sha256: self.policy_sha256.clone(),
            quarantine_sha256: self.quarantine_sha256.clone(),
            artifact_sha256: None,
            production_provenance_sha256: None,
            complete_scan: false,
            accepted: false,
            publication_authorized: false,
            entries_scanned: 0,
            uncompressed_bytes_scanned: 0,
            findings: Vec::new(),
        };

        let metadata = match fs::symlink_metadata(artifact) {
            Ok(metadata) => metadata,
            Err(error) => {
                push(
                    &mut report,
                    FindingCode::ManifestMalformed,
                    error.to_string(),
                );
                return report;
            }
        };
        if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
            push(&mut report, FindingCode::LinkRejected, "artifact root");
            return report;
        }

        if metadata.is_file() {
            report.artifact_kind = "regular_file";
            if metadata.len() > self.policy.max_uncompressed_bytes {
                report.uncompressed_bytes_scanned = self.policy.max_uncompressed_bytes;
                push(
                    &mut report,
                    FindingCode::ByteLimitExceeded,
                    metadata.len().to_string(),
                );
                return report;
            }
            let (digest, bytes) = match hash_regular_file(artifact) {
                Ok(value) => value,
                Err(error) => {
                    push(
                        &mut report,
                        FindingCode::ManifestMalformed,
                        error.to_string(),
                    );
                    return report;
                }
            };
            report.artifact_sha256 = Some(digest.clone());
            if self.quarantine.contains(&digest) {
                push(&mut report, FindingCode::QuarantinedHash, digest);
                return report;
            }
            if mode == InspectionMode::Package && is_zip_archive(artifact) {
                // Bounded zip reader: scans each decompressed entry to a bounded
                // in-memory buffer (never the compressed whole-file stream, never
                // extracted to disk) and owns its own entry/byte accounting.
                report.artifact_kind = "zip_archive";
                self.inspect_zip(artifact, mode, &mut report);
            } else if mode == InspectionMode::Package && is_dmg_archive(artifact) {
                // Bounded DMG reader: mounts read-only on macOS and delegates to
                // inspect_directory on the single mounted .app. Non-macOS falls
                // back to UnsupportedArchive (a macOS DMG is not meaningfully
                // inspectable elsewhere; the macOS packaging path is macOS-only).
                report.artifact_kind = "dmg_image";
                self.inspect_dmg(artifact, mode, &mut report);
            } else {
                report.entries_scanned = 1;
                if bytes > self.policy.max_uncompressed_bytes {
                    report.uncompressed_bytes_scanned = self.policy.max_uncompressed_bytes;
                    push(
                        &mut report,
                        FindingCode::ByteLimitExceeded,
                        bytes.to_string(),
                    );
                } else {
                    report.uncompressed_bytes_scanned = bytes;
                    if let Err(error) = self.scan_file(artifact, &mut report) {
                        push(
                            &mut report,
                            FindingCode::ManifestMalformed,
                            error.to_string(),
                        );
                    }
                }
                if mode == InspectionMode::Package {
                    push(
                        &mut report,
                        FindingCode::UnsupportedArchive,
                        "opaque package requires a bounded format reader",
                    );
                }
            }
        } else {
            report.artifact_kind = "directory";
            self.inspect_directory(artifact, mode, &mut report);
        }
        // Bind production provenance: load a provided provenance file or look
        // for a sibling (artifact + ".provenance.json").  For a production
        // artifact (any mode), missing or invalid provenance is a fail-closed
        // finding — production_provenance_sha256 is never silently None.
        bind_provenance(&mut report, artifact, provenance);
        report.complete_scan = !report.has(FindingCode::EntryLimitExceeded)
            && !report.has(FindingCode::ByteLimitExceeded)
            && !report.has(FindingCode::UnsupportedArchive)
            && !report.has(FindingCode::ManifestMalformed);
        // `accepted` reflects scan completeness and content safety.  Provenance
        // findings (missing/invalid) are fail-closed evidence findings — they
        // are always recorded so production_provenance_sha256 is never silently
        // None — but they do not block scan acceptance.  Publication is gated
        // separately by `publication_authorized`, which Wave 0 cannot set.
        report.accepted = report.complete_scan
            && !report.findings.iter().any(|f| {
                !matches!(
                    f.code,
                    FindingCode::ProvenanceMissing | FindingCode::ProvenanceInvalid
                )
            });
        // Wave 0 deliberately cannot authorize publication: production
        // provenance and bounded readers for every emitted archive format are
        // Wave 3 prerequisites. Callers must not equate a clean scan with an
        // authorization receipt.
        report.publication_authorized = false;
        report
    }

    fn inspect_directory(
        &self,
        artifact: &Path,
        mode: InspectionMode,
        report: &mut InspectionReport,
    ) {
        let mut manifests = Vec::new();
        let mut executables = 0_u64;
        for entry in WalkDir::new(artifact).follow_links(false) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    push(report, FindingCode::ManifestMalformed, error.to_string());
                    break;
                }
            };
            if entry.path() == artifact {
                continue;
            }
            if report.entries_scanned == self.policy.max_entries {
                push(
                    report,
                    FindingCode::EntryLimitExceeded,
                    self.policy.max_entries.to_string(),
                );
                break;
            }
            report.entries_scanned += 1;
            if entry.file_type().is_symlink() {
                push(
                    report,
                    FindingCode::LinkRejected,
                    relative_subject(artifact, entry.path()),
                );
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(u64::MAX);
            let attempted = report.uncompressed_bytes_scanned.saturating_add(size);
            if attempted > self.policy.max_uncompressed_bytes {
                report.uncompressed_bytes_scanned = self.policy.max_uncompressed_bytes;
                push(
                    report,
                    FindingCode::ByteLimitExceeded,
                    attempted.to_string(),
                );
                break;
            }
            report.uncompressed_bytes_scanned = attempted;
            if entry.file_name() == "package-manifest-v1.json" {
                manifests.push(entry.path().to_owned());
            }
            let relative = relative_subject(artifact, entry.path());
            if relative == "Contents/MacOS/FeatherMark" || relative == "bin/feathermark" {
                executables += 1;
            }
            if let Err(error) = self.scan_file(entry.path(), report) {
                push(report, FindingCode::ManifestMalformed, error.to_string());
            }
        }
        if mode == InspectionMode::Package {
            if executables != 1 {
                push(
                    report,
                    FindingCode::ExecutableCountMismatch,
                    executables.to_string(),
                );
            }
            if manifests.len() != 1 {
                push(
                    report,
                    FindingCode::ManifestMissing,
                    manifests.len().to_string(),
                );
            } else {
                self.inspect_manifest(artifact, &manifests[0], report);
            }
        }
    }

    fn scan_file(&self, path: &Path, report: &mut InspectionReport) -> io::Result<()> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take(self.policy.max_uncompressed_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        self.scan_bytes(&bytes, report);
        Ok(())
    }

    /// Forbidden-pattern + credential-shape scan over in-memory bytes. Shared by
    /// the regular-file scan and the bounded archive readers (which decompress
    /// each entry to a bounded buffer before calling this).
    fn scan_bytes(&self, bytes: &[u8], report: &mut InspectionReport) {
        for pattern in &self.policy.forbidden_patterns {
            if contains(bytes, pattern.as_bytes()) {
                let code = classify_pattern(pattern, &self.policy.test_control_environment);
                push(report, code, redact_pattern(pattern));
            }
        }
        if contains_sensitive_shape(bytes) {
            push(
                report,
                FindingCode::CredentialMarker,
                "credential or email shape matched",
            );
        }
    }

    fn inspect_manifest(&self, root: &Path, path: &Path, report: &mut InspectionReport) {
        let value: serde_json::Value = match fs::read(path)
            .map_err(InspectorError::from)
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(InspectorError::from))
        {
            Ok(value) => value,
            Err(error) => {
                push(report, FindingCode::ManifestMalformed, error.to_string());
                return;
            }
        };
        self.check_manifest_json(&value, report);

        let is_macos = root.join("Contents/MacOS/FeatherMark").is_file();
        let platform_ok = if is_macos {
            let info = fs::read(root.join("Contents/Info.plist")).unwrap_or_default();
            Self::check_macos_info_plist(&info)
        } else {
            value
                .get("wayland_verified")
                .and_then(|value| value.as_bool())
                == Some(true)
                && value
                    .get("rpm_runtime_verified")
                    .and_then(|value| value.as_bool())
                    == Some(true)
                && root
                    .join("share/applications/feathermark.desktop")
                    .is_file()
                && root
                    .join("share/mime/packages/feathermark-markdown.xml")
                    .is_file()
        };
        if !platform_ok {
            push(
                report,
                FindingCode::PlatformMetadataMissing,
                "package integration",
            );
        }
    }

    /// License / version / source-commit checks on a parsed package manifest.
    /// Shared by the directory and bounded-zip inspection paths.
    fn check_manifest_json(&self, value: &serde_json::Value, report: &mut InspectionReport) {
        if value.get("license").and_then(|value| value.as_str())
            != Some(self.policy.expected_license.as_str())
        {
            push(report, FindingCode::LicenseMismatch, "package manifest");
        }
        if let Some(expected) = &self.policy.expected_version {
            if value.get("version").and_then(|value| value.as_str()) != Some(expected.as_str()) {
                push(report, FindingCode::VersionMismatch, "package manifest");
            }
        }
        if let Some(expected) = &self.policy.expected_source_commit {
            if value.get("source_commit").and_then(|value| value.as_str())
                != Some(expected.as_str())
            {
                push(
                    report,
                    FindingCode::SourceCommitMismatch,
                    "package manifest",
                );
            }
        }
    }

    /// macOS platform-metadata check on Info.plist bytes: CFBundleIdentifier,
    /// CFBundleDocumentTypes, and UTTypeConformsTo must all be present.
    fn check_macos_info_plist(info: &[u8]) -> bool {
        let plist = std::str::from_utf8(info).unwrap_or("");
        plist.contains("CFBundleIdentifier")
            && plist.contains("CFBundleDocumentTypes")
            && plist.contains("UTTypeConformsTo")
    }

    /// Bounded DMG reader. On macOS, mounts the image read-only (`hdiutil
    /// attach -readonly -nobrowse`) and delegates to `inspect_directory` on the
    /// single mounted `.app`, reusing the directory scan exactly. The mount is
    /// cleaned up by `DmgMountGuard` on every path (including panic). On other
    /// platforms, reports UnsupportedArchive (a macOS DMG is only meaningfully
    /// inspectable on macOS, and the macOS packaging path is macOS-only).
    #[cfg(target_os = "macos")]
    #[allow(clippy::disallowed_methods)]
    fn inspect_dmg(&self, artifact: &Path, mode: InspectionMode, report: &mut InspectionReport) {
        use std::process::Command;
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let mount_path = std::env::temp_dir().join(format!(
            "feathermark-dmg-inspect-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        if let Err(error) = fs::create_dir(&mount_path) {
            push(report, FindingCode::ManifestMalformed, error.to_string());
            return;
        }
        let _guard = DmgMountGuard {
            mountpoint: mount_path.clone(),
            detached: false,
        };
        let attached = match Command::new("hdiutil")
            .args(["attach", "-readonly", "-nobrowse", "-mountpoint"])
            .arg(&mount_path)
            .arg(artifact)
            .output()
        {
            Ok(out) => out.status.success(),
            Err(error) => {
                push(report, FindingCode::ManifestMalformed, error.to_string());
                return;
            }
        };
        if !attached {
            push(
                report,
                FindingCode::UnsupportedArchive,
                "hdiutil attach failed",
            );
            return;
        }
        if let Some(app) = find_single_app(&mount_path) {
            self.inspect_directory(&app, mode, report);
        } else {
            push(
                report,
                FindingCode::ManifestMalformed,
                "DMG has no single top-level .app",
            );
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn inspect_dmg(&self, _artifact: &Path, _mode: InspectionMode, report: &mut InspectionReport) {
        push(
            report,
            FindingCode::UnsupportedArchive,
            "dmg inspection requires macOS",
        );
    }

    /// Bounded reader for a ditto-produced `.app.zip`. Decompresses each entry
    /// to a bounded in-memory buffer (never to disk), strips the single
    /// top-level `.app/` directory, skips `__MACOSX/` + `._*` AppleDouble
    /// metadata, and applies the same accounting + scan + manifest checks as
    /// `inspect_directory`. Owns its own entry/byte accounting.
    fn inspect_zip(&self, artifact: &Path, mode: InspectionMode, report: &mut InspectionReport) {
        let file = match File::open(artifact) {
            Ok(file) => file,
            Err(error) => {
                push(report, FindingCode::ManifestMalformed, error.to_string());
                return;
            }
        };
        let mut archive = match ZipArchive::new(file) {
            Ok(archive) => archive,
            Err(error) => {
                push(report, FindingCode::ManifestMalformed, error.to_string());
                return;
            }
        };

        let mut manifests: Vec<Vec<u8>> = Vec::new();
        let mut info_plist: Option<Vec<u8>> = None;
        let mut executables = 0_u64;
        let mut top_prefix: Option<String> = None;

        for index in 0..archive.len() {
            if report.entries_scanned == self.policy.max_entries {
                push(
                    report,
                    FindingCode::EntryLimitExceeded,
                    self.policy.max_entries.to_string(),
                );
                break;
            }
            let entry = match archive.by_index(index) {
                Ok(entry) => entry,
                Err(error) => {
                    push(report, FindingCode::ManifestMalformed, error.to_string());
                    break;
                }
            };
            let name = entry.name().to_string();
            if entry.is_dir() {
                continue;
            }
            // Skip AppleDouble / __MACOSX metadata ditto emits alongside the app.
            if name.starts_with("__MACOSX/") {
                continue;
            }
            if name
                .rsplit('/')
                .next()
                .map(|base| base.starts_with("._"))
                .unwrap_or(false)
            {
                continue;
            }
            if entry.is_symlink() {
                push(report, FindingCode::LinkRejected, name.clone());
                continue;
            }

            // Strip the single top-level dir prefix (e.g. "Rutile.app/") so the
            // relative predicates match inspect_directory (Contents/MacOS/...).
            let relative = {
                if top_prefix.is_none() {
                    if let Some(seg) = name.split('/').next().filter(|seg| !seg.is_empty()) {
                        top_prefix = Some(format!("{seg}/"));
                    }
                }
                top_prefix
                    .as_deref()
                    .and_then(|prefix| name.strip_prefix(prefix))
                    .unwrap_or(&name)
                    .to_string()
            };

            report.entries_scanned += 1;

            // Zip-bomb heuristic: reject extreme compression ratios.
            let uncompressed = entry.size();
            let compressed = entry.compressed_size();
            if compressed > 0 && uncompressed / compressed > 1000 {
                push(
                    report,
                    FindingCode::ByteLimitExceeded,
                    format!("compression ratio {uncompressed}/{compressed}"),
                );
                break;
            }
            let attempted = report
                .uncompressed_bytes_scanned
                .saturating_add(uncompressed);
            if attempted > self.policy.max_uncompressed_bytes {
                report.uncompressed_bytes_scanned = self.policy.max_uncompressed_bytes;
                push(
                    report,
                    FindingCode::ByteLimitExceeded,
                    attempted.to_string(),
                );
                break;
            }
            report.uncompressed_bytes_scanned = attempted;

            let mut buf = Vec::new();
            if let Err(error) = entry
                .take(self.policy.max_uncompressed_bytes.saturating_add(1))
                .read_to_end(&mut buf)
            {
                push(report, FindingCode::ManifestMalformed, error.to_string());
                continue;
            }
            self.scan_bytes(&buf, report);

            if relative == "Contents/MacOS/FeatherMark" {
                executables += 1;
            }
            if relative == "Contents/Resources/package-manifest-v1.json" {
                manifests.push(buf);
            } else if relative == "Contents/Info.plist" {
                info_plist = Some(buf);
            }
        }

        if mode == InspectionMode::Package {
            if executables != 1 {
                push(
                    report,
                    FindingCode::ExecutableCountMismatch,
                    executables.to_string(),
                );
            }
            if manifests.is_empty() {
                push(report, FindingCode::ManifestMissing, "0");
            } else {
                for manifest_bytes in &manifests {
                    match serde_json::from_slice::<serde_json::Value>(manifest_bytes) {
                        Ok(value) => self.check_manifest_json(&value, report),
                        Err(error) => {
                            push(report, FindingCode::ManifestMalformed, error.to_string())
                        }
                    }
                }
                let platform_ok = info_plist
                    .as_deref()
                    .map(Self::check_macos_info_plist)
                    .unwrap_or(false);
                if !platform_ok {
                    push(
                        report,
                        FindingCode::PlatformMetadataMissing,
                        "package integration",
                    );
                }
            }
        }
    }
}

fn validate_quarantine(registry: &QuarantineRegistry) -> Result<(), InspectorError> {
    if registry.schema != "rutile.artifact-quarantine.v1" || registry.version != 1 {
        return Err(InspectorError::InvalidQuarantine(
            "unsupported schema or version".into(),
        ));
    }
    let mut seen = HashSet::new();
    for entry in &registry.entries {
        if !valid_sha256(&entry.sha256) {
            return Err(InspectorError::InvalidQuarantine(format!(
                "invalid SHA-256 for {}",
                entry.artifact
            )));
        }
        if !seen.insert(entry.sha256.as_str()) {
            return Err(InspectorError::InvalidQuarantine(format!(
                "duplicate quarantine SHA-256: {}",
                entry.sha256
            )));
        }
        if entry.artifact.is_empty()
            || entry.reason.trim().is_empty()
            || !valid_date(&entry.discovered_at)
        {
            return Err(InspectorError::InvalidQuarantine(format!(
                "incomplete entry for {}",
                entry.artifact
            )));
        }
    }
    Ok(())
}

fn validate_policy(policy: &InspectorPolicy) -> Result<(), InspectorError> {
    if policy.schema != "rutile.artifact-inspector-policy.v1"
        || policy.version != 1
        || policy.max_entries == 0
        || policy.max_entries > 256
        || policy.max_uncompressed_bytes == 0
        || policy.max_uncompressed_bytes > 64 * 1024 * 1024
        || policy.expected_license.trim().is_empty()
        || policy.forbidden_patterns.is_empty()
        || policy.test_control_environment.is_empty()
        || policy
            .forbidden_patterns
            .iter()
            .any(|pattern| pattern.is_empty())
    {
        return Err(InspectorError::InvalidPolicy(
            "schema, bounds, expected license, and patterns are required".into(),
        ));
    }
    if let Some(commit) = &policy.expected_source_commit {
        if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(InspectorError::InvalidPolicy(
                "expected source commit must be 40 hexadecimal characters".into(),
            ));
        }
    }
    Ok(())
}

fn read_policy_file(path: &Path) -> Result<Vec<u8>, InspectorError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(InspectorError::UnsafePolicyFile(path.to_owned()));
    }
    if metadata.len() > MAX_POLICY_BYTES {
        return Err(InspectorError::PolicyTooLarge(path.to_owned()));
    }
    Ok(fs::read(path)?)
}

fn hash_regular_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total = total.saturating_add(read as u64);
    }
    Ok((hex::encode(hasher.finalize()), total))
}

/// Sniff whether a regular file is a zip archive by its local-file-header or
/// empty-archive magic bytes (`PK\x03\x04` / `PK\x05\x06`). Used to dispatch the
/// bounded zip reader in Package mode.
fn is_zip_archive(path: &Path) -> bool {
    let mut buf = [0u8; 4];
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    if file.read_exact(&mut buf).is_err() {
        return false;
    }
    buf[..2] == [b'P', b'K']
        && ((buf[2] == 0x03 && buf[3] == 0x04) || (buf[2] == 0x05 && buf[3] == 0x06))
}

/// Identify a macOS DMG by its `.dmg` extension. The bounded reader mounts it
/// read-only on macOS; on other platforms the reader reports UnsupportedArchive.
fn is_dmg_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("dmg"))
        .unwrap_or(false)
}

/// RAII guard that detaches a mounted DMG and removes the mountpoint directory
/// on drop, on every path (including panic via unwind).
#[cfg(target_os = "macos")]
struct DmgMountGuard {
    mountpoint: PathBuf,
    detached: bool,
}

#[cfg(target_os = "macos")]
impl Drop for DmgMountGuard {
    fn drop(&mut self) {
        if !self.detached {
            #[allow(clippy::disallowed_methods)]
            let _ = std::process::Command::new("hdiutil")
                .args(["detach", "-force"])
                .arg(&self.mountpoint)
                .status();
        }
        let _ = fs::remove_dir(&self.mountpoint);
    }
}

/// Return the single top-level `.app` directory at a DMG mount point, or None if
/// the layout is not exactly one `.app`.
#[cfg(target_os = "macos")]
fn find_single_app(mount: &Path) -> Option<PathBuf> {
    let entries: Vec<_> = fs::read_dir(mount)
        .ok()?
        .filter_map(|entry| entry.ok())
        .collect();
    if entries.len() != 1 {
        return None;
    }
    let path = entries[0].path();
    let is_app_dir = entries[0]
        .file_type()
        .ok()
        .map(|file_type| file_type.is_dir())
        .unwrap_or(false)
        && path.extension().and_then(|ext| ext.to_str()) == Some("app");
    is_app_dir.then_some(path)
}

fn classify_pattern(pattern: &str, test_env: &[String]) -> FindingCode {
    if test_env.iter().any(|marker| marker == pattern)
        || pattern.contains("TEST_CONTROL")
        || pattern.contains("native-smoke")
    {
        FindingCode::TestControlMarker
    } else if pattern.contains("/home/")
        || pattern.contains("/Users/")
        || pattern.contains("/tmp/")
        || pattern.contains("workspace")
    {
        FindingCode::PrivateBuildPath
    } else if pattern.to_ascii_lowercase().contains("token")
        || pattern.to_ascii_lowercase().contains("secret")
        || pattern.contains('@')
    {
        FindingCode::CredentialMarker
    } else {
        FindingCode::ForbiddenPattern
    }
}

fn redact_pattern(pattern: &str) -> String {
    if classify_pattern(pattern, &[]) == FindingCode::CredentialMarker {
        "credential pattern matched".into()
    } else {
        pattern.into()
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn contains_sensitive_shape(bytes: &[u8]) -> bool {
    const TOKEN_PREFIXES: &[(&[u8], usize)] = &[
        (b"ghp_", 24),
        (b"github_pat_", 28),
        (b"sk-", 24),
        (b"AKIA", 16),
    ];
    if TOKEN_PREFIXES
        .iter()
        .any(|(prefix, minimum_length)| contains_token_shape(bytes, prefix, *minimum_length))
    {
        return true;
    }
    bytes
        .split(|byte| byte.is_ascii_whitespace() || b"<>\"'(),;".contains(byte))
        .any(|word| {
            let Some(at) = word.iter().position(|byte| *byte == b'@') else {
                return false;
            };
            at > 0
                && word[at + 1..].contains(&b'.')
                && word[..at]
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._%+-".contains(byte))
                && word[at + 1..]
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(byte))
        })
}

fn contains_token_shape(bytes: &[u8], prefix: &[u8], minimum_length: usize) -> bool {
    bytes
        .windows(prefix.len())
        .enumerate()
        .any(|(start, window)| {
            if window != prefix || start > 0 && is_token_byte(bytes[start - 1]) {
                return false;
            }
            let end = start.saturating_add(minimum_length);
            end <= bytes.len() && bytes[start..end].iter().copied().all(is_token_byte)
        })
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}

fn push(report: &mut InspectionReport, code: FindingCode, subject: impl Into<String>) {
    let subject = subject.into();
    if !report
        .findings
        .iter()
        .any(|finding| finding.code == code && finding.subject == subject)
    {
        report.findings.push(InspectionFinding { code, subject });
    }
}

fn relative_subject(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

/// Bind production provenance into the inspection report.
///
/// If a provenance path is explicitly provided, load that.  Otherwise, look
/// for a sibling file `<artifact>.provenance.json`.  When a file is found,
/// validate it as JSON with the expected schema field and compute its SHA-256
/// to bind into the report.  Missing or invalid provenance is a fail-closed
/// finding — `production_provenance_sha256` is never silently `None`.
fn bind_provenance(report: &mut InspectionReport, artifact: &Path, provenance: Option<&Path>) {
    let resolved = provenance
        .map(Path::to_path_buf)
        .unwrap_or_else(|| provenance_sibling(artifact));

    if !resolved.is_file() {
        push(report, FindingCode::ProvenanceMissing, "no provenance file");
        return;
    }

    let bytes = match fs::read(&resolved) {
        Ok(bytes) => bytes,
        Err(error) => {
            push(
                report,
                FindingCode::ProvenanceInvalid,
                format!("cannot read provenance: {error}"),
            );
            return;
        }
    };

    // Validate that the file is JSON with the expected schema identifier.
    let schema_ok = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("schema")
                .and_then(|v| v.as_str())
                .map(|s| s == "rutile.production-provenance.v1")
        })
        .unwrap_or(false);

    if !schema_ok {
        push(
            report,
            FindingCode::ProvenanceInvalid,
            "provenance is not a valid rutile.production-provenance.v1 record",
        );
        return;
    }

    let digest = hex::encode(Sha256::digest(&bytes));
    report.production_provenance_sha256 = Some(digest);
}

/// Construct the sibling provenance path: `<artifact>.provenance.json` in the
/// same directory as the artifact.
fn provenance_sibling(artifact: &Path) -> PathBuf {
    let mut name = artifact
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".provenance.json");
    artifact
        .parent()
        .map(|p| p.join(&name))
        .unwrap_or_else(|| PathBuf::from(&name))
}
