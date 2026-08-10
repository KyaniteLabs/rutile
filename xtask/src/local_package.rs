//! Deterministic, local-only package assembly for Rutile release candidates.
//!
//! This module prepares packages and command argument vectors. It never invokes
//! signing, disk-image, archive, or shell programs itself.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

pub const LOCAL_BETA_VERSION: &str = "0.2.2";
pub const MAX_EXECUTABLE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_ARTIFACT_BYTES: u64 = 20 * 1024 * 1024;

pub const MACOS_PACKAGE_LABEL: &str = "local-unnotarized-macos-arm64";
pub const LINUX_PACKAGE_LABEL: &str = "linux-x86_64-unverified-wayland";

pub const MACOS_APP_NAME: &str = "Rutile.app";
pub const MACOS_ZIP_NAME: &str = "Rutile-0.2.2-macos-arm64.app.zip";
pub const MACOS_DMG_NAME: &str = "Rutile-0.2.2-macos-arm64.dmg";

pub const LINUX_ARCHIVE_DIR_NAME: &str = "Rutile-linux-x86_64";
pub const LINUX_ARCHIVE_NAME: &str = "Rutile-0.2.2-linux-x86_64.tar.zst";
pub const LINUX_DEB_NAME: &str = "rutile_0.2.2_amd64.deb";
pub const LINUX_RPM_NAME: &str = "rutile-0.2.2-1.x86_64.rpm";

/// License declared in every package manifest and SBOM. Derived from the
/// xtask crate's `CARGO_PKG_LICENSE` (which inherits `MIT` from
/// `[workspace.package]`) so it can never drift from the workspace setting;
/// falls back to `MIT` to match the root LICENSE file if the env var is unset.
pub const PACKAGE_LICENSE: &str = match option_env!("CARGO_PKG_LICENSE") {
    Some(license) => license,
    None => "MIT",
};

/// SBOM filename written into every assembled package.
pub const SBOM_FILE_NAME: &str = "sbom.spdx.json";

/// First-party workspace crates listed in the SBOM dependency inventory.
const SBOM_WORKSPACE_CRATES: &[&str] = &[
    "rutile-types",
    "rutile-core",
    "rutile-protocol",
    "rutile-app",
];

// Platform assets are embedded at compile time via build.rs so the assembled
// packages never depend on builder-relative filesystem layout. build.rs reads
// them from release/assets/ and writes them to OUT_DIR/release-assets/.
const MACOS_DOCUMENT_TYPES_PLIST: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/release-assets/document-types.plist"
));
const MACOS_APP_ICON: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/release-assets/AppIcon.icns"));
const LINUX_DESKTOP_ENTRY: &str =
    include_str!(concat!(env!("OUT_DIR"), "/release-assets/rutile.desktop"));
const LINUX_APPDATA: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/release-assets/rutile.appdata.xml"
));
const LINUX_MIME: &str = include_str!(concat!(
    env!("OUT_DIR"),
    "/release-assets/rutile-markdown.xml"
));

#[derive(Debug, Error)]
pub enum LocalPackageError {
    #[error("{field} must be an absolute normalized path: {path}")]
    UnsafePath { field: &'static str, path: PathBuf },
    #[error("{field} contains a symlink: {path}")]
    Symlink { field: &'static str, path: PathBuf },
    #[error("{field} must be a regular file: {path}")]
    NotRegularFile { field: &'static str, path: PathBuf },
    #[error("invalid SHA-256 for {field}")]
    InvalidHash { field: &'static str },
    #[error("build-input SHA-256 mismatch: expected {expected}, measured {measured}")]
    BuildInputHashMismatch { expected: String, measured: String },
    #[error("source commit must be 40 lowercase hexadecimal characters")]
    InvalidSourceCommit,
    #[error("candidate is not a supported {expected} executable")]
    WrongExecutableArchitecture { expected: &'static str },
    #[error("version must contain only ASCII letters, digits, periods, plus signs, or hyphens")]
    InvalidVersion,
    #[error("executable exceeds maximum size: {path} ({bytes} bytes)")]
    ExecutableTooLarge { path: PathBuf, bytes: u64 },
    #[error("artifact exceeds maximum size: {path} ({bytes} bytes)")]
    ArtifactTooLarge { path: PathBuf, bytes: u64 },
    #[error("output already exists: {0}")]
    OutputExists(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct MacPackageRequest {
    pub candidate: PathBuf,
    pub build_input_sha256: String,
    pub source_commit: String,
    pub output_root: PathBuf,
    pub version: String,
    /// When set, run_macos signs a preview-publication authorization for each
    /// produced artifact (after generating provenance), so the inline Package-mode
    /// inspection passes at the preview tier. None => produce artifacts + provenance
    /// only (the operator runs `release preview-authorize` separately).
    pub release_authority_key: Option<PathBuf>,
    pub preview_signed_at: Option<String>,
    pub preview_expires_at: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LinuxPackageRequest {
    pub candidate: PathBuf,
    pub build_input_sha256: String,
    pub source_commit: String,
    pub output_root: PathBuf,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandPlan {
    pub program: String,
    pub args: Vec<OsString>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AssemblyReceipt {
    pub label: &'static str,
    pub build_input_sha256: String,
    pub output: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct ArtifactManifest {
    pub schema: &'static str,
    pub label: &'static str,
    pub artifact: PathBuf,
    pub artifact_sha256: String,
    pub build_input_sha256: String,
    pub packaged_executable_sha256: String,
    pub source_commit: String,
    pub version: String,
    pub target_triple: &'static str,
    pub notarized: bool,
    pub wayland_verified: bool,
    pub rpm_runtime_verified: bool,
}

#[derive(Serialize)]
struct MacAppManifest<'a> {
    schema: &'static str,
    label: &'static str,
    architecture: &'static str,
    build_input_sha256: &'a str,
    packaged_executable_sha256: &'a str,
    source_commit: &'a str,
    version: &'a str,
    license: &'static str,
    signing: &'static str,
    notarized: bool,
}

#[derive(Serialize)]
struct LinuxLayoutManifest<'a> {
    schema: &'static str,
    label: &'static str,
    architecture: &'static str,
    build_input_sha256: &'a str,
    packaged_executable_sha256: &'a str,
    source_commit: &'a str,
    version: &'a str,
    license: &'static str,
    wayland_verified: bool,
    rpm_runtime_verified: bool,
    runtime_dependencies: &'static [RuntimeDependency],
}

#[derive(Serialize)]
pub struct RuntimeDependency {
    pub soname: &'static str,
    pub debian_package: &'static str,
    pub fedora_package: &'static str,
    pub required_for: &'static str,
}

pub static LINUX_RUNTIME_DEPENDENCIES: &[RuntimeDependency] = &[
    RuntimeDependency {
        soname: "libgtk-3.so.0",
        debian_package: "libgtk-3-0",
        fedora_package: "gtk3",
        required_for: "GTK 3 user interface",
    },
    RuntimeDependency {
        soname: "libgtksourceview-4.so.0",
        debian_package: "libgtksourceview-4-0",
        fedora_package: "gtksourceview4",
        required_for: "GtkSourceView 4 editor widget",
    },
    RuntimeDependency {
        soname: "libwebkit2gtk-4.1.so.0",
        debian_package: "libwebkit2gtk-4.1-0",
        fedora_package: "webkit2gtk4.1",
        required_for: "Wry webview runtime",
    },
    RuntimeDependency {
        soname: "libjavascriptcoregtk-4.1.so.0",
        debian_package: "libjavascriptcoregtk-4.1-0",
        fedora_package: "javascriptcoregtk4.1",
        required_for: "WebKit JavaScript runtime",
    },
];

pub fn create_package_output_root(path: &Path) -> Result<(), LocalPackageError> {
    prepare_new_output_root(path)
}

pub fn sha256_regular_file(path: &Path) -> Result<String, LocalPackageError> {
    let bytes = read_regular_file_no_symlinks(path, "executable")?;
    Ok(hex_sha256(&bytes))
}

pub fn assemble_macos_app(
    request: &MacPackageRequest,
) -> Result<AssemblyReceipt, LocalPackageError> {
    validate_version(&request.version)?;
    validate_source_commit(&request.source_commit)?;
    let candidate = read_hash_bound_candidate(&request.candidate, &request.build_input_sha256)?;
    validate_macho_arm64(&candidate)?;
    enforce_executable_size(&request.candidate, &candidate)?;

    let app = request
        .output_root
        .join("_staging")
        .join("app")
        .join(MACOS_APP_NAME);
    if app.exists() {
        return Err(LocalPackageError::OutputExists(app));
    }
    let contents = app.join("Contents");
    let executable = contents.join("MacOS/Rutile");
    let resources = contents.join("Resources");
    fs::create_dir_all(executable.parent().expect("executable has parent"))?;
    fs::create_dir_all(&resources)?;
    write_executable(&executable, &candidate)?;

    let packaged_executable_sha256 = hex_sha256(&candidate);

    // Build the Info.plist by splicing the document-type/UTI fragment from
    // release/assets/macos/document-types.plist into the assembler-authored
    // base keys. The fragment supplies CFBundleDocumentTypes and
    // UTExportedTypeDeclarations so Finder can route .md files to Rutile.
    let plist_head = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"https://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict>\n\
  <key>CFBundleDisplayName</key><string>Rutile</string>\n\
  <key>CFBundleExecutable</key><string>Rutile</string>\n\
  <key>CFBundleIconFile</key><string>AppIcon</string>\n\
  <key>CFBundleIdentifier</key><string>com.kyanitelabs.rutile</string>\n\
  <key>CFBundleName</key><string>Rutile</string>\n\
  <key>CFBundlePackageType</key><string>APPL</string>\n\
  <key>CFBundleShortVersionString</key><string>{}</string>\n\
  <key>CFBundleVersion</key><string>{}</string>\n\
  <key>LSArchitecturePriority</key><array><string>arm64</string></array>\n",
        request.version, request.version
    );
    let document_types = extract_plist_dict_inner(MACOS_DOCUMENT_TYPES_PLIST);
    let plist = format!("{plist_head}{document_types}</dict></plist>\n");
    write_new_file(&contents.join("Info.plist"), plist.as_bytes())?;
    fs::write(resources.join("AppIcon.icns"), MACOS_APP_ICON)?;
    write_json(
        &resources.join("package-manifest-v1.json"),
        &MacAppManifest {
            schema: "rutile-local-package-v1",
            label: MACOS_PACKAGE_LABEL,
            architecture: "aarch64-apple-darwin",
            build_input_sha256: &request.build_input_sha256,
            packaged_executable_sha256: &packaged_executable_sha256,
            source_commit: &request.source_commit,
            version: &request.version,
            license: PACKAGE_LICENSE,
            signing: "ad-hoc-command-required",
            notarized: false,
        },
    )?;
    write_sbom(
        &resources.join(SBOM_FILE_NAME),
        &request.version,
        &request.source_commit,
        "aarch64-apple-darwin",
        &[],
    )?;

    Ok(AssemblyReceipt {
        label: MACOS_PACKAGE_LABEL,
        build_input_sha256: request.build_input_sha256.clone(),
        output: app,
    })
}

pub fn prepare_linux_layout(
    request: &LinuxPackageRequest,
) -> Result<AssemblyReceipt, LocalPackageError> {
    validate_version(&request.version)?;
    validate_source_commit(&request.source_commit)?;
    let candidate = read_hash_bound_candidate(&request.candidate, &request.build_input_sha256)?;
    validate_elf_x86_64(&candidate)?;
    enforce_executable_size(&request.candidate, &candidate)?;

    let layout = request
        .output_root
        .join("_staging")
        .join("archive")
        .join(LINUX_ARCHIVE_DIR_NAME);
    if layout.exists() {
        return Err(LocalPackageError::OutputExists(layout));
    }
    let executable = layout.join("bin/rutile");
    fs::create_dir_all(executable.parent().expect("executable has parent"))?;
    write_executable(&executable, &candidate)?;
    write_json(
        &layout.join("package-manifest-v1.json"),
        &LinuxLayoutManifest {
            schema: "rutile-local-package-v1",
            label: LINUX_PACKAGE_LABEL,
            architecture: "x86_64-unknown-linux-gnu",
            build_input_sha256: &request.build_input_sha256,
            packaged_executable_sha256: &request.build_input_sha256,
            source_commit: &request.source_commit,
            version: &request.version,
            license: PACKAGE_LICENSE,
            wayland_verified: false,
            rpm_runtime_verified: false,
            runtime_dependencies: LINUX_RUNTIME_DEPENDENCIES,
        },
    )?;
    let runtime_sonames: Vec<&str> = LINUX_RUNTIME_DEPENDENCIES
        .iter()
        .map(|dep| dep.soname)
        .collect();
    write_sbom(
        &layout.join(SBOM_FILE_NAME),
        &request.version,
        &request.source_commit,
        "x86_64-unknown-linux-gnu",
        &runtime_sonames,
    )?;

    Ok(AssemblyReceipt {
        label: LINUX_PACKAGE_LABEL,
        build_input_sha256: request.build_input_sha256.clone(),
        output: layout,
    })
}

pub fn prepare_debian_staging(
    request: &LinuxPackageRequest,
) -> Result<AssemblyReceipt, LocalPackageError> {
    validate_version(&request.version)?;
    validate_source_commit(&request.source_commit)?;
    let candidate = read_hash_bound_candidate(&request.candidate, &request.build_input_sha256)?;
    validate_elf_x86_64(&candidate)?;
    enforce_executable_size(&request.candidate, &candidate)?;

    let staging = request.output_root.join("_staging").join("deb");
    if staging.exists() {
        return Err(LocalPackageError::OutputExists(staging));
    }
    let binary = staging.join("usr/bin/rutile");
    let doc_dir = staging.join("usr/share/doc/rutile");
    let control_dir = staging.join("DEBIAN");
    let applications_dir = staging.join("usr/share/applications");
    let metainfo_dir = staging.join("usr/share/metainfo");
    let mime_packages_dir = staging.join("usr/share/mime/packages");
    fs::create_dir_all(binary.parent().expect("binary has parent"))?;
    fs::create_dir_all(&doc_dir)?;
    fs::create_dir_all(&control_dir)?;
    fs::create_dir_all(&applications_dir)?;
    fs::create_dir_all(&metainfo_dir)?;
    fs::create_dir_all(&mime_packages_dir)?;
    write_executable(&binary, &candidate)?;

    // Install freedesktop platform assets so launchers and file managers can
    // offer "Open with Rutile" and classify Markdown documents.
    write_new_file(
        &applications_dir.join("rutile.desktop"),
        LINUX_DESKTOP_ENTRY.as_bytes(),
    )?;
    write_new_file(
        &metainfo_dir.join("rutile.appdata.xml"),
        LINUX_APPDATA.as_bytes(),
    )?;
    write_new_file(
        &mime_packages_dir.join("rutile-markdown.xml"),
        LINUX_MIME.as_bytes(),
    )?;

    let manifest = LinuxLayoutManifest {
        schema: "rutile-local-package-v1",
        label: LINUX_PACKAGE_LABEL,
        architecture: "x86_64-unknown-linux-gnu",
        build_input_sha256: &request.build_input_sha256,
        packaged_executable_sha256: &request.build_input_sha256,
        source_commit: &request.source_commit,
        version: &request.version,
        license: PACKAGE_LICENSE,
        wayland_verified: false,
        rpm_runtime_verified: false,
        runtime_dependencies: LINUX_RUNTIME_DEPENDENCIES,
    };
    write_json(&doc_dir.join("package-manifest-v1.json"), &manifest)?;
    let runtime_sonames: Vec<&str> = LINUX_RUNTIME_DEPENDENCIES
        .iter()
        .map(|dep| dep.soname)
        .collect();
    write_sbom(
        &doc_dir.join(SBOM_FILE_NAME),
        &request.version,
        &request.source_commit,
        "x86_64-unknown-linux-gnu",
        &runtime_sonames,
    )?;

    let control = format!(
        "Package: rutile\n\
Version: {}\n\
Section: editors\n\
Priority: optional\n\
Architecture: amd64\n\
Depends: libgtk-3-0, libgtksourceview-4-0, libwebkit2gtk-4.1-0, libjavascriptcoregtk-4.1-0\n\
Maintainer: Kyanite Build <build@kyanitelabs.ai>\n\
Description: Rutile — A local-first writing studio by Kyanite.\n",
        request.version
    );
    write_new_file(&control_dir.join("control"), control.as_bytes())?;

    Ok(AssemblyReceipt {
        label: LINUX_PACKAGE_LABEL,
        build_input_sha256: request.build_input_sha256.clone(),
        output: staging,
    })
}

pub fn debian_package_plan(staging: &Path, deb: &Path) -> Result<CommandPlan, LocalPackageError> {
    validate_existing_directory(staging, "staging")?;
    validate_output_artifact_path(deb, "deb")?;
    Ok(CommandPlan {
        program: "dpkg-deb".into(),
        args: vec![
            "--root-owner-group".into(),
            "--build".into(),
            staging.as_os_str().to_owned(),
            deb.as_os_str().to_owned(),
        ],
    })
}

pub fn prepare_rpm_staging(
    request: &LinuxPackageRequest,
) -> Result<AssemblyReceipt, LocalPackageError> {
    validate_version(&request.version)?;
    validate_source_commit(&request.source_commit)?;
    let candidate = read_hash_bound_candidate(&request.candidate, &request.build_input_sha256)?;
    validate_elf_x86_64(&candidate)?;
    enforce_executable_size(&request.candidate, &candidate)?;

    let topdir = request.output_root.join("_staging").join("rpm");
    if topdir.exists() {
        return Err(LocalPackageError::OutputExists(topdir));
    }
    fs::create_dir_all(topdir.join("BUILD"))?;
    fs::create_dir_all(topdir.join("RPMS"))?;
    fs::create_dir_all(topdir.join("SOURCES"))?;
    fs::create_dir_all(topdir.join("SPECS"))?;
    fs::create_dir_all(topdir.join("SRPMS"))?;

    // Copy the candidate binary and platform assets into SOURCES/ under stable,
    // reproducible names. The spec references them via %{_sourcedir} (which
    // rpmbuild resolves to <topdir>/SOURCES at build time) so NO absolute
    // builder path is ever interpolated into the spec file.
    let sources = topdir.join("SOURCES");
    write_new_file(&sources.join("rutile"), &candidate)?;
    write_new_file(
        &sources.join("rutile.desktop"),
        LINUX_DESKTOP_ENTRY.as_bytes(),
    )?;
    write_new_file(
        &sources.join("rutile.appdata.xml"),
        LINUX_APPDATA.as_bytes(),
    )?;
    write_new_file(&sources.join("rutile-markdown.xml"), LINUX_MIME.as_bytes())?;
    let runtime_sonames: Vec<&str> = LINUX_RUNTIME_DEPENDENCIES
        .iter()
        .map(|dep| dep.soname)
        .collect();
    write_sbom(
        &sources.join(SBOM_FILE_NAME),
        &request.version,
        &request.source_commit,
        "x86_64-unknown-linux-gnu",
        &runtime_sonames,
    )?;

    let spec = topdir.join("SPECS/rutile.spec");
    let spec_body = format!(
        "Name:           rutile\n\
Version:        {}\n\
Release:        1%{{?dist}}\n\
Summary:        Rutile — A local-first writing studio by Kyanite.\n\
License:        {}\n\
URL:            https://kyanitelabs.ai\n\
BuildArch:      x86_64\n\
\n\
Requires:       gtk3, gtksourceview4, webkit2gtk4.1\n\
\n\
%description\n\
Rutile — A local-first writing studio by Kyanite.\n\
\n\
%install\n\
install -D -m 0755 %{{_sourcedir}}/rutile %{{buildroot}}/usr/bin/rutile\n\
install -D -m 0644 %{{_sourcedir}}/rutile.desktop %{{buildroot}}/usr/share/applications/rutile.desktop\n\
install -D -m 0644 %{{_sourcedir}}/rutile.appdata.xml %{{buildroot}}/usr/share/metainfo/rutile.appdata.xml\n\
install -D -m 0644 %{{_sourcedir}}/rutile-markdown.xml %{{buildroot}}/usr/share/mime/packages/rutile-markdown.xml\n\
install -D -m 0644 %{{_sourcedir}}/{sbom} %{{buildroot}}/usr/share/doc/rutile/{sbom}\n\
\n\
%files\n\
/usr/bin/rutile\n\
/usr/share/applications/rutile.desktop\n\
/usr/share/metainfo/rutile.appdata.xml\n\
/usr/share/mime/packages/rutile-markdown.xml\n\
/usr/share/doc/rutile/{sbom}\n",
        request.version,
        PACKAGE_LICENSE,
        sbom = SBOM_FILE_NAME
    );
    write_new_file(&spec, spec_body.as_bytes())?;

    Ok(AssemblyReceipt {
        label: LINUX_PACKAGE_LABEL,
        build_input_sha256: request.build_input_sha256.clone(),
        output: topdir,
    })
}

pub fn rpm_package_plan(topdir: &Path, spec: &Path) -> Result<CommandPlan, LocalPackageError> {
    validate_existing_directory(topdir, "topdir")?;
    validate_absolute_normalized(spec, "spec")?;
    reject_existing_symlink_components(spec, "spec")?;
    if !spec.is_file() {
        return Err(LocalPackageError::NotRegularFile {
            field: "spec",
            path: spec.to_owned(),
        });
    }
    // `_topdir` is a build-time invocation argument passed on the rpmbuild
    // command line. It is NOT interpolated into the spec file — the spec
    // references all sources via %{_sourcedir} which rpmbuild resolves
    // relative to _topdir. The topdir path therefore never leaks into the
    // shipped RPM metadata; it is analogous to the staging path passed to
    // dpkg-deb. It must be absolute because the CommandPlan carries no
    // working-directory context.
    Ok(CommandPlan {
        program: "rpmbuild".into(),
        args: vec![
            "--define".into(),
            format!("_topdir {}", topdir.display()).into(),
            "-bb".into(),
            spec.as_os_str().to_owned(),
        ],
    })
}

pub fn macos_adhoc_codesign_plan(app: &Path) -> Result<CommandPlan, LocalPackageError> {
    validate_existing_directory(app, "app")?;
    Ok(CommandPlan {
        program: "codesign".into(),
        args: vec![
            "--force".into(),
            "--sign".into(),
            "-".into(),
            "--timestamp=none".into(),
            app.as_os_str().to_owned(),
        ],
    })
}

pub fn macos_codesign_verify_plan(app: &Path) -> Result<CommandPlan, LocalPackageError> {
    validate_existing_directory(app, "app")?;
    Ok(CommandPlan {
        program: "codesign".into(),
        args: vec![
            "--verify".into(),
            "--deep".into(),
            "--strict".into(),
            "--verbose=2".into(),
            app.as_os_str().to_owned(),
        ],
    })
}

pub fn macos_zip_plan(app: &Path, zip: &Path) -> Result<CommandPlan, LocalPackageError> {
    validate_existing_directory(app, "app")?;
    validate_output_artifact_path(zip, "zip")?;
    Ok(CommandPlan {
        program: "ditto".into(),
        args: vec![
            "-c".into(),
            "-k".into(),
            "--sequesterRsrc".into(),
            "--keepParent".into(),
            app.as_os_str().to_owned(),
            zip.as_os_str().to_owned(),
        ],
    })
}

pub fn macos_dmg_plan(app: &Path, dmg: &Path) -> Result<CommandPlan, LocalPackageError> {
    validate_existing_directory(app, "app")?;
    validate_output_artifact_path(dmg, "dmg")?;
    Ok(CommandPlan {
        program: "hdiutil".into(),
        args: vec![
            "create".into(),
            "-volname".into(),
            "Rutile".into(),
            "-srcfolder".into(),
            app.as_os_str().to_owned(),
            "-format".into(),
            "UDZO".into(),
            dmg.as_os_str().to_owned(),
        ],
    })
}

pub fn linux_archive_plan(
    layout: &Path,
    archive: &Path,
) -> Result<Vec<CommandPlan>, LocalPackageError> {
    validate_existing_directory(layout, "layout")?;
    validate_output_artifact_path(archive, "archive")?;
    let parent = layout.parent().expect("absolute layout has parent");
    let name = layout
        .file_name()
        .ok_or_else(|| LocalPackageError::UnsafePath {
            field: "layout",
            path: layout.to_owned(),
        })?;
    let intermediate = archive.with_extension("");
    Ok(vec![
        CommandPlan {
            program: "tar".into(),
            args: vec![
                "--sort=name".into(),
                "--mtime=@0".into(),
                "--owner=0".into(),
                "--group=0".into(),
                "--numeric-owner".into(),
                "-cf".into(),
                intermediate.as_os_str().to_owned(),
                "-C".into(),
                parent.as_os_str().to_owned(),
                name.to_owned(),
            ],
        },
        CommandPlan {
            program: "zstd".into(),
            args: vec![
                "--quiet".into(),
                "--force".into(),
                "--threads=1".into(),
                "-19".into(),
                intermediate.into_os_string(),
                "-o".into(),
                archive.as_os_str().to_owned(),
            ],
        },
    ])
}

pub fn finalize_macos_zip_manifest(
    zip: &Path,
    build_input_sha256: &str,
    packaged_executable_sha256: &str,
    source_commit: &str,
    version: &str,
) -> Result<ArtifactManifest, LocalPackageError> {
    finalize_artifact(
        zip,
        build_input_sha256,
        packaged_executable_sha256,
        source_commit,
        version,
        MACOS_PACKAGE_LABEL,
        "aarch64-apple-darwin",
        false,
        false,
        false,
    )
}

pub fn finalize_macos_dmg_manifest(
    dmg: &Path,
    build_input_sha256: &str,
    packaged_executable_sha256: &str,
    source_commit: &str,
    version: &str,
) -> Result<ArtifactManifest, LocalPackageError> {
    finalize_artifact(
        dmg,
        build_input_sha256,
        packaged_executable_sha256,
        source_commit,
        version,
        MACOS_PACKAGE_LABEL,
        "aarch64-apple-darwin",
        false,
        false,
        false,
    )
}

pub fn finalize_linux_package_manifest(
    artifact: &Path,
    build_input_sha256: &str,
    packaged_executable_sha256: &str,
    source_commit: &str,
    version: &str,
) -> Result<ArtifactManifest, LocalPackageError> {
    finalize_artifact(
        artifact,
        build_input_sha256,
        packaged_executable_sha256,
        source_commit,
        version,
        LINUX_PACKAGE_LABEL,
        "x86_64-unknown-linux-gnu",
        false,
        false,
        false,
    )
}

pub fn finalize_linux_archive_manifest(
    archive: &Path,
    build_input_sha256: &str,
    packaged_executable_sha256: &str,
    source_commit: &str,
    version: &str,
) -> Result<ArtifactManifest, LocalPackageError> {
    finalize_artifact(
        archive,
        build_input_sha256,
        packaged_executable_sha256,
        source_commit,
        version,
        LINUX_PACKAGE_LABEL,
        "x86_64-unknown-linux-gnu",
        false,
        false,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn finalize_artifact(
    artifact: &Path,
    build_input_sha256: &str,
    packaged_executable_sha256: &str,
    source_commit: &str,
    version: &str,
    label: &'static str,
    target_triple: &'static str,
    notarized: bool,
    wayland_verified: bool,
    rpm_runtime_verified: bool,
) -> Result<ArtifactManifest, LocalPackageError> {
    validate_hash(build_input_sha256, "build_input_sha256")?;
    validate_hash(packaged_executable_sha256, "packaged_executable_sha256")?;
    validate_source_commit(source_commit)?;
    validate_version(version)?;
    let bytes = read_regular_file_no_symlinks(artifact, "artifact")?;
    let len = bytes.len() as u64;
    if len > MAX_ARTIFACT_BYTES {
        return Err(LocalPackageError::ArtifactTooLarge {
            path: artifact.to_owned(),
            bytes: len,
        });
    }
    let artifact_name = artifact
        .file_name()
        .expect("validated artifact has filename")
        .to_owned();
    let manifest = ArtifactManifest {
        schema: "rutile-local-artifact-v1",
        label,
        artifact: PathBuf::from(&artifact_name),
        artifact_sha256: hex_sha256(&bytes),
        build_input_sha256: build_input_sha256.to_owned(),
        packaged_executable_sha256: packaged_executable_sha256.to_owned(),
        source_commit: source_commit.to_owned(),
        version: version.to_owned(),
        target_triple,
        notarized,
        wayland_verified,
        rpm_runtime_verified,
    };
    let file_name = artifact_name.to_string_lossy();
    write_json(
        &artifact.with_file_name(format!("{file_name}.manifest-v1.json")),
        &manifest,
    )?;
    Ok(manifest)
}

fn validate_version(version: &str) -> Result<(), LocalPackageError> {
    if version.is_empty()
        || version.len() > 64
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-'))
    {
        return Err(LocalPackageError::InvalidVersion);
    }
    Ok(())
}

pub fn validate_source_commit(source_commit: &str) -> Result<(), LocalPackageError> {
    if source_commit.len() != 40
        || !source_commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(LocalPackageError::InvalidSourceCommit);
    }
    Ok(())
}

fn enforce_executable_size(path: &Path, bytes: &[u8]) -> Result<(), LocalPackageError> {
    let len = bytes.len() as u64;
    if len > MAX_EXECUTABLE_BYTES {
        return Err(LocalPackageError::ExecutableTooLarge {
            path: path.to_owned(),
            bytes: len,
        });
    }
    Ok(())
}

fn read_hash_bound_candidate(path: &Path, expected: &str) -> Result<Vec<u8>, LocalPackageError> {
    validate_hash(expected, "build_input_sha256")?;
    let bytes = read_regular_file_no_symlinks(path, "candidate")?;
    let measured = hex_sha256(&bytes);
    if measured != expected {
        return Err(LocalPackageError::BuildInputHashMismatch {
            expected: expected.to_owned(),
            measured,
        });
    }
    Ok(bytes)
}

fn read_regular_file_no_symlinks(
    path: &Path,
    field: &'static str,
) -> Result<Vec<u8>, LocalPackageError> {
    validate_absolute_normalized(path, field)?;
    reject_existing_symlink_components(path, field)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(LocalPackageError::NotRegularFile {
            field,
            path: path.to_owned(),
        });
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn prepare_new_output_root(path: &Path) -> Result<(), LocalPackageError> {
    validate_absolute_normalized(path, "output_root")?;
    reject_existing_symlink_components(path, "output_root")?;
    if path.exists() {
        return Err(LocalPackageError::OutputExists(path.to_owned()));
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn validate_existing_directory(path: &Path, field: &'static str) -> Result<(), LocalPackageError> {
    validate_absolute_normalized(path, field)?;
    reject_existing_symlink_components(path, field)?;
    if !path.is_dir() {
        return Err(LocalPackageError::NotRegularFile {
            field,
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn validate_output_artifact_path(
    path: &Path,
    field: &'static str,
) -> Result<(), LocalPackageError> {
    validate_absolute_normalized(path, field)?;
    reject_existing_symlink_components(path, field)?;
    if path.exists() {
        return Err(LocalPackageError::OutputExists(path.to_owned()));
    }
    if let Some(parent) = path.parent() {
        if !parent.is_dir() {
            return Err(LocalPackageError::NotRegularFile {
                field,
                path: parent.to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_absolute_normalized(path: &Path, field: &'static str) -> Result<(), LocalPackageError> {
    let normalized = path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                Component::RootDir | Component::Prefix(_) | Component::Normal(_)
            )
        });
    if !normalized {
        return Err(LocalPackageError::UnsafePath {
            field,
            path: path.to_owned(),
        });
    }
    Ok(())
}

fn reject_existing_symlink_components(
    path: &Path,
    field: &'static str,
) -> Result<(), LocalPackageError> {
    for component in path.ancestors() {
        if let Ok(metadata) = fs::symlink_metadata(component) {
            if metadata.file_type().is_symlink() {
                return Err(LocalPackageError::Symlink {
                    field,
                    path: component.to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn validate_hash(hash: &str, field: &'static str) -> Result<(), LocalPackageError> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(LocalPackageError::InvalidHash { field });
    }
    Ok(())
}

fn validate_macho_arm64(bytes: &[u8]) -> Result<(), LocalPackageError> {
    // 64-bit little-endian Mach-O magic followed by CPU_TYPE_ARM64.
    if bytes.get(..8) != Some(&[0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01]) {
        return Err(LocalPackageError::WrongExecutableArchitecture {
            expected: "Mach-O arm64",
        });
    }
    Ok(())
}

fn validate_elf_x86_64(bytes: &[u8]) -> Result<(), LocalPackageError> {
    let valid = bytes.get(..6) == Some(&[0x7f, b'E', b'L', b'F', 2, 1])
        && bytes.get(18..20) == Some(&[0x3e, 0x00]);
    if !valid {
        return Err(LocalPackageError::WrongExecutableArchitecture {
            expected: "ELF x86_64",
        });
    }
    Ok(())
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), LocalPackageError> {
    write_new_file(path, bytes)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), LocalPackageError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), LocalPackageError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_new_file(path, &bytes)
}

fn hex_sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Extract the inner key/value pairs from a plist fragment whose root is a
/// `<dict>`. Returns everything between the outermost `<dict>` and its
/// matching `</dict>`, so the caller can splice the keys into another
/// `<dict>` without introducing a nested root element.
fn extract_plist_dict_inner(fragment: &str) -> &str {
    let plist_pos = fragment
        .find("<plist")
        .expect("document-types fragment contains <plist");
    let after_plist = &fragment[plist_pos..];
    let dict_open = after_plist
        .find("<dict>")
        .expect("document-types fragment contains <dict>");
    let content_start = plist_pos + dict_open + "<dict>".len();
    let plist_end = fragment
        .rfind("</plist>")
        .expect("document-types fragment contains </plist>");
    let before_end = &fragment[..plist_end];
    let dict_close = before_end
        .rfind("</dict>")
        .expect("document-types fragment contains </dict>");
    &fragment[content_start..dict_close]
}

#[derive(Serialize)]
struct SpdxDocument {
    spdx_version: &'static str,
    data_license: &'static str,
    spdx_id: &'static str,
    name: &'static str,
    document_namespace: String,
    creation_info: SpdxCreationInfo,
    packages: Vec<SpdxPackage>,
    relationships: Vec<SpdxRelationship>,
}

#[derive(Serialize)]
struct SpdxCreationInfo {
    creators: &'static [&'static str],
    created: &'static str,
}

#[derive(Serialize)]
struct SpdxPackage {
    name: &'static str,
    spdx_id: &'static str,
    version_info: String,
    license_concluded: &'static str,
    license_declared: &'static str,
    download_location: &'static str,
    files_analyzed: bool,
    copyright_text: &'static str,
    supplier: &'static str,
    rutile_workspace_crates: &'static [&'static str],
    rutile_runtime_libraries: Vec<String>,
}

#[derive(Serialize)]
struct SpdxRelationship {
    spdx_element_id: &'static str,
    relationship_type: &'static str,
    related_spdx_element: &'static str,
}

/// Write a minimal SPDX 2.3 JSON SBOM into the package. The document declares
/// the MIT license and inventories the first-party workspace crates plus the
/// runtime shared libraries the package depends on. The `created` timestamp is
/// fixed to the Unix epoch for reproducibility (matching the `--mtime=@0`
/// convention used by the tar archive plan); the full dependency graph remains
/// available via `cargo metadata` at release time.
fn write_sbom(
    path: &Path,
    version: &str,
    source_commit: &str,
    target_triple: &str,
    runtime_libraries: &[&str],
) -> Result<(), LocalPackageError> {
    let document = SpdxDocument {
        spdx_version: "SPDX-2.3",
        data_license: "CC0-1.0",
        spdx_id: "SPDXRef-DOCUMENT",
        name: "rutile",
        document_namespace: format!(
            "https://kyanitelabs.ai/spdx/rutile-{version}-{source_commit}-{target_triple}"
        ),
        creation_info: SpdxCreationInfo {
            creators: &["Tool: rutile-xtask", "Organization: Kyanite"],
            created: "1970-01-01T00:00:00Z",
        },
        packages: vec![SpdxPackage {
            name: "rutile",
            spdx_id: "SPDXRef-Package-rutile",
            version_info: version.to_owned(),
            license_concluded: PACKAGE_LICENSE,
            license_declared: PACKAGE_LICENSE,
            download_location: "NOASSERTION",
            files_analyzed: false,
            copyright_text: "NOASSERTION",
            supplier: "Organization: Kyanite",
            rutile_workspace_crates: SBOM_WORKSPACE_CRATES,
            rutile_runtime_libraries: runtime_libraries
                .iter()
                .map(|soname| (*soname).to_owned())
                .collect(),
        }],
        relationships: vec![SpdxRelationship {
            spdx_element_id: "SPDXRef-DOCUMENT",
            relationship_type: "DESCRIBES",
            related_spdx_element: "SPDXRef-Package-rutile",
        }],
    };
    let mut bytes = serde_json::to_vec_pretty(&document)?;
    bytes.push(b'\n');
    write_new_file(path, &bytes)
}
