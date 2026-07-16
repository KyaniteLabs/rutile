//! Locked CLI surface for deterministic local Rutile packaging.
//!
//! This module validates host/tool availability, invokes typed assembly APIs,
//! executes each direct argument-vector plan, verifies exit status, finalizes
//! manifests, prints machine-readable JSON receipts, and cleans up staging on
//! success. It never invokes a shell.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

use crate::artifact_inspector::{ArtifactInspector, InspectionMode, InspectorError, PolicyPaths};

use crate::local_package::{
    ArtifactManifest, CommandPlan, LINUX_ARCHIVE_NAME, LINUX_DEB_NAME, LINUX_RPM_NAME,
    LinuxPackageRequest, LocalPackageError, MACOS_DMG_NAME, MACOS_ZIP_NAME, MAX_ARTIFACT_BYTES,
    MAX_EXECUTABLE_BYTES, MacPackageRequest,
};

#[derive(Debug, Error)]
pub enum LocalPackageCliError {
    #[error(transparent)]
    Package(#[from] LocalPackageError),
    #[error("tool failed: {program} exited with status {status:?}")]
    ToolFailed {
        program: String,
        status: Option<i32>,
    },
    #[error("tool terminated by signal: {program}")]
    ToolSignaled { program: String },
    #[error("missing embedded executable: {0}")]
    MissingEmbeddedExecutable(PathBuf),
    #[error("executable exceeds maximum size: {path} ({bytes} bytes)")]
    ExecutableTooLarge { path: PathBuf, bytes: u64 },
    #[error("artifact exceeds maximum size: {path} ({bytes} bytes)")]
    ArtifactTooLarge { path: PathBuf, bytes: u64 },
    #[error(transparent)]
    Inspector(#[from] InspectorError),
    #[error("artifact policy rejected {path}: {findings}")]
    PolicyRejected { path: PathBuf, findings: String },
    #[error("preview bless failed: {0}")]
    Bless(String),
}

pub enum LocalPackageCliRequest {
    Macos(MacPackageRequest),
    Linux(LinuxPackageRequest),
}

pub trait CommandExecutor {
    fn execute(&self, plan: &CommandPlan) -> Result<(), LocalPackageCliError>;
}

pub struct ProcessCommandExecutor;

impl CommandExecutor for ProcessCommandExecutor {
    #[allow(clippy::disallowed_methods)]
    fn execute(&self, plan: &CommandPlan) -> Result<(), LocalPackageCliError> {
        let status = Command::new(&plan.program)
            .args(&plan.args)
            .status()
            .map_err(LocalPackageError::from)?;
        if let Some(code) = status.code() {
            if code != 0 {
                return Err(LocalPackageCliError::ToolFailed {
                    program: plan.program.clone(),
                    status: Some(code),
                });
            }
        } else {
            return Err(LocalPackageCliError::ToolSignaled {
                program: plan.program.clone(),
            });
        }
        Ok(())
    }
}

pub fn run_local_package(
    request: LocalPackageCliRequest,
    executor: &dyn CommandExecutor,
) -> Result<Vec<ArtifactManifest>, LocalPackageCliError> {
    let inspector = ArtifactInspector::load(&PolicyPaths::repository_defaults())?;
    run_local_package_with_inspector(request, executor, &inspector)
}

pub fn run_local_package_with_inspector(
    request: LocalPackageCliRequest,
    executor: &dyn CommandExecutor,
    inspector: &ArtifactInspector,
) -> Result<Vec<ArtifactManifest>, LocalPackageCliError> {
    let (candidate, output_root) = match &request {
        LocalPackageCliRequest::Macos(request) => (&request.candidate, &request.output_root),
        LocalPackageCliRequest::Linux(request) => (&request.candidate, &request.output_root),
    };
    enforce_inspection(inspector, candidate, InspectionMode::Candidate, None)?;
    let output_root = output_root.clone();
    let manifests = match request {
        LocalPackageCliRequest::Macos(request) => run_macos(request, executor),
        LocalPackageCliRequest::Linux(request) => run_linux(request, executor),
    }?;
    for manifest in &manifests {
        enforce_inspection(
            inspector,
            &output_root.join(&manifest.artifact),
            InspectionMode::Package,
            None,
        )?;
    }
    Ok(manifests)
}

#[doc(hidden)]
pub fn enforce_inspection(
    inspector: &ArtifactInspector,
    path: &Path,
    mode: InspectionMode,
    provenance: Option<&Path>,
) -> Result<(), LocalPackageCliError> {
    let report = inspector.inspect(path, mode, provenance);
    if report.accepted
        && (mode == InspectionMode::Candidate
            || report.publication_authorized
            || report.preview_authorized)
    {
        return Ok(());
    }
    let mut findings = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<Vec<_>>()
        .join(",");
    if report.accepted
        && mode == InspectionMode::Package
        && !report.publication_authorized
        && !report.preview_authorized
    {
        findings = "publication_not_authorized".into();
    }
    Err(LocalPackageCliError::PolicyRejected {
        path: path.to_owned(),
        findings,
    })
}

/// Write a production-provenance sibling for `artifact` (generated from the
/// candidate), and — when a release-authority key + signing window are provided
/// — sign a preview-publication authorization sibling so the Package-mode
/// inspection passes at the preview tier. The operator asserts the candidate
/// was produced by `reproducible-build`.
fn bless_macos_artifact(
    artifact: &Path,
    request: &MacPackageRequest,
) -> Result<(), LocalPackageCliError> {
    use sha2::Digest;
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| LocalPackageCliError::Bless("cannot resolve workspace root".into()))?;
    let provenance = crate::provenance::generate_with_reproducible_env(
        &crate::provenance::ProvenanceRequest {
            candidate: request.candidate.clone(),
            repo_root,
            features: vec!["macos-shell".to_string()],
            target_root: "target/prod".to_string(),
        },
        "feathermark-app",
        &request.version,
    )
    .map_err(|error| LocalPackageCliError::Bless(error.to_string()))?;
    let json = provenance
        .canonical_json()
        .map_err(|error| LocalPackageCliError::Bless(error.to_string()))?;
    let provenance_path = sibling_path(artifact, ".provenance.json");
    fs::write(&provenance_path, &json)
        .map_err(|error| LocalPackageCliError::Bless(error.to_string()))?;
    let provenance_sha = provenance
        .provenance_sha256()
        .map_err(|error| LocalPackageCliError::Bless(error.to_string()))?;

    if let (Some(key_path), Some(signed_at), Some(expires_at)) = (
        request.release_authority_key.as_ref(),
        request.preview_signed_at.as_ref(),
        request.preview_expires_at.as_ref(),
    ) {
        let signing = crate::release_authority::read_signing_key(key_path)
            .map_err(|error| LocalPackageCliError::Bless(error.to_string()))?;
        let artifact_bytes =
            fs::read(artifact).map_err(|error| LocalPackageCliError::Bless(error.to_string()))?;
        let statement = crate::release_authority::PreviewAuthorizationStatement {
            artifact_sha256: hex::encode(sha2::Sha256::digest(&artifact_bytes)),
            provenance_sha256: provenance_sha,
            tier: crate::release_authority::PREVIEW_TIER.to_string(),
            product: "feathermark-app".to_string(),
            version_label: request.version.clone(),
            signed_at: signed_at.clone(),
            expires_at: expires_at.clone(),
        };
        let record = crate::release_authority::sign(&statement, &signing)
            .map_err(|error| LocalPackageCliError::Bless(error.to_string()))?;
        let auth = serde_json::to_string_pretty(&record)
            .map_err(|error| LocalPackageCliError::Bless(error.to_string()))?;
        fs::write(sibling_path(artifact, ".preview-authorization.json"), auth)
            .map_err(|error| LocalPackageCliError::Bless(error.to_string()))?;
    }
    Ok(())
}

/// `<artifact><suffix>` next to the artifact (e.g. `.provenance.json`).
fn sibling_path(artifact: &Path, suffix: &str) -> PathBuf {
    let mut name = artifact
        .file_name()
        .map(|file_name| file_name.to_os_string())
        .unwrap_or_default();
    name.push(suffix);
    artifact.with_file_name(name)
}

fn run_macos(
    request: MacPackageRequest,
    executor: &dyn CommandExecutor,
) -> Result<Vec<ArtifactManifest>, LocalPackageCliError> {
    use crate::local_package::{
        assemble_macos_app, create_package_output_root, finalize_macos_dmg_manifest,
        finalize_macos_zip_manifest, macos_adhoc_codesign_plan, macos_codesign_verify_plan,
        macos_dmg_plan, macos_zip_plan, sha256_regular_file,
    };

    create_package_output_root(&request.output_root)?;

    let app_receipt = assemble_macos_app(&request)?;
    let app = app_receipt.output;

    executor.execute(&macos_adhoc_codesign_plan(&app)?)?;
    executor.execute(&macos_codesign_verify_plan(&app)?)?;

    let embedded = app.join("Contents/MacOS/FeatherMark");
    if !embedded.is_file() {
        return Err(LocalPackageCliError::MissingEmbeddedExecutable(embedded));
    }
    let embedded_meta = fs::metadata(&embedded).map_err(LocalPackageError::from)?;
    if embedded_meta.len() > MAX_EXECUTABLE_BYTES {
        return Err(LocalPackageCliError::ExecutableTooLarge {
            path: embedded.clone(),
            bytes: embedded_meta.len(),
        });
    }
    let packaged_executable_sha256 = sha256_regular_file(&embedded)?;

    let zip = request.output_root.join(MACOS_ZIP_NAME);
    let dmg = request.output_root.join(MACOS_DMG_NAME);

    executor.execute(&macos_zip_plan(&app, &zip)?)?;
    executor.execute(&macos_dmg_plan(&app, &dmg)?)?;

    enforce_artifact_size(&zip)?;
    enforce_artifact_size(&dmg)?;

    let zip_manifest = finalize_macos_zip_manifest(
        &zip,
        &request.build_input_sha256,
        &packaged_executable_sha256,
        &request.source_commit,
        &request.version,
    )?;
    let dmg_manifest = finalize_macos_dmg_manifest(
        &dmg,
        &request.build_input_sha256,
        &packaged_executable_sha256,
        &request.source_commit,
        &request.version,
    )?;

    // Bless artifacts (provenance + signed preview authorization) only when the
    // operator provides a release-authority key — the one-shot preview path. With
    // no key, artifacts are produced unembellished and the operator runs
    // `provenance generate` + `release preview-authorize` separately.
    if request.release_authority_key.is_some() {
        bless_macos_artifact(&zip, &request)?;
        bless_macos_artifact(&dmg, &request)?;
    }

    remove_staging(&request.output_root)?;

    Ok(vec![zip_manifest, dmg_manifest])
}

fn run_linux(
    request: LinuxPackageRequest,
    executor: &dyn CommandExecutor,
) -> Result<Vec<ArtifactManifest>, LocalPackageCliError> {
    use crate::local_package::{
        create_package_output_root, debian_package_plan, finalize_linux_archive_manifest,
        finalize_linux_package_manifest, linux_archive_plan, prepare_debian_staging,
        prepare_linux_layout, prepare_rpm_staging, rpm_package_plan,
    };

    create_package_output_root(&request.output_root)?;

    // Linux packaging does not mutate the executable, so the build-input hash is
    // also the packaged-executable hash.
    let packaged_executable_sha256 = request.build_input_sha256.clone();

    let layout_receipt = prepare_linux_layout(&request)?;
    let deb_receipt = prepare_debian_staging(&request)?;
    let rpm_receipt = prepare_rpm_staging(&request)?;

    let archive = request.output_root.join(LINUX_ARCHIVE_NAME);
    let deb = request.output_root.join(LINUX_DEB_NAME);
    let rpm = request.output_root.join(LINUX_RPM_NAME);

    for plan in linux_archive_plan(&layout_receipt.output, &archive)? {
        executor.execute(&plan)?;
    }
    executor.execute(&debian_package_plan(&deb_receipt.output, &deb)?)?;

    let spec = rpm_receipt.output.join("SPECS/feathermark.spec");
    executor.execute(&rpm_package_plan(&rpm_receipt.output, &spec)?)?;

    // rpmbuild places built RPMs under <topdir>/RPMS/<arch>/; move the exact
    // artifact to the output root so all final artifacts share one directory.
    let built_rpm = rpm_receipt.output.join("RPMS/x86_64").join(LINUX_RPM_NAME);
    if built_rpm != rpm {
        fs::rename(&built_rpm, &rpm).map_err(LocalPackageError::from)?;
    }

    enforce_artifact_size(&archive)?;
    enforce_artifact_size(&deb)?;
    enforce_artifact_size(&rpm)?;

    let archive_manifest = finalize_linux_archive_manifest(
        &archive,
        &request.build_input_sha256,
        &packaged_executable_sha256,
        &request.source_commit,
        &request.version,
    )?;
    let deb_manifest = finalize_linux_package_manifest(
        &deb,
        &request.build_input_sha256,
        &packaged_executable_sha256,
        &request.source_commit,
        &request.version,
    )?;
    let rpm_manifest = finalize_linux_package_manifest(
        &rpm,
        &request.build_input_sha256,
        &packaged_executable_sha256,
        &request.source_commit,
        &request.version,
    )?;

    remove_staging(&request.output_root)?;

    Ok(vec![archive_manifest, deb_manifest, rpm_manifest])
}

fn enforce_artifact_size(path: &Path) -> Result<(), LocalPackageCliError> {
    let bytes = fs::metadata(path).map_err(LocalPackageError::from)?.len();
    if bytes > MAX_ARTIFACT_BYTES {
        return Err(LocalPackageCliError::ArtifactTooLarge {
            path: path.to_owned(),
            bytes,
        });
    }
    Ok(())
}

fn remove_staging(output_root: &Path) -> Result<(), LocalPackageCliError> {
    let staging = output_root.join("_staging");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(LocalPackageError::from)?;
    }
    Ok(())
}
