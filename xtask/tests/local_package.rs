use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::sync::Mutex;
use tempfile::tempdir;
use xtask::local_package::{
    LINUX_PACKAGE_LABEL, LINUX_RUNTIME_DEPENDENCIES, LinuxPackageRequest, MACOS_DMG_NAME,
    MACOS_PACKAGE_LABEL, MACOS_ZIP_NAME, MAX_ARTIFACT_BYTES, MAX_EXECUTABLE_BYTES,
    MacPackageRequest, assemble_macos_app, create_package_output_root, debian_package_plan,
    finalize_linux_archive_manifest, finalize_macos_dmg_manifest, finalize_macos_zip_manifest,
    linux_archive_plan, macos_adhoc_codesign_plan, macos_codesign_verify_plan, macos_dmg_plan,
    macos_zip_plan, prepare_debian_staging, prepare_linux_layout, prepare_rpm_staging,
    rpm_package_plan, sha256_regular_file,
};
use xtask::local_package_cli::{
    CommandExecutor, LocalPackageCliError, LocalPackageCliRequest, ProcessCommandExecutor,
    run_local_package,
};

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn valid_source_commit() -> String {
    "a".repeat(40)
}

fn mach_o_arm64() -> Vec<u8> {
    let mut bytes = vec![0; 32];
    bytes[..8].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0x00, 0x00, 0x01]);
    bytes
}

fn elf_x86_64() -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
    bytes[18..20].copy_from_slice(&[0x3e, 0x00]);
    bytes
}

#[test]
fn assembles_deterministic_arm64_app_bound_to_candidate_hash() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("candidate with spaces");
    let bytes = mach_o_arm64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("output");
    fs::create_dir(&output).unwrap();

    let receipt = assemble_macos_app(&MacPackageRequest {
        candidate: candidate.clone(),
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.clone(),
        version: "0.1.0".into(),
    })
    .unwrap();

    assert_eq!(receipt.label, MACOS_PACKAGE_LABEL);
    assert_eq!(receipt.build_input_sha256, sha256(&bytes));
    assert_eq!(
        fs::read(
            output
                .join("_staging")
                .join("app")
                .join("FeatherMark.app/Contents/MacOS/FeatherMark")
        )
        .unwrap(),
        bytes
    );
    assert_eq!(
        fs::metadata(
            output
                .join("_staging")
                .join("app")
                .join("FeatherMark.app/Contents/MacOS/FeatherMark")
        )
        .unwrap()
        .permissions()
        .mode()
            & 0o777,
        0o755
    );
    let plist = fs::read_to_string(
        output
            .join("_staging")
            .join("app")
            .join("FeatherMark.app/Contents/Info.plist"),
    )
    .unwrap();
    assert!(plist.contains("<string>arm64</string>"));
    assert!(plist.contains("<string>com.kyanitelabs.feathermark</string>"));

    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(
            output
                .join("_staging")
                .join("app")
                .join("FeatherMark.app/Contents/Resources/package-manifest-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["label"], MACOS_PACKAGE_LABEL);
    assert_eq!(metadata["build_input_sha256"], sha256(&mach_o_arm64()));
    assert_eq!(metadata["source_commit"], valid_source_commit());
    assert_eq!(metadata["version"], "0.1.0");
    assert_eq!(metadata["notarized"], false);
}

#[test]
fn macos_plans_are_argument_vectors_and_dmg_manifest_hashes_existing_artifact() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let app = root.join("Feather Mark.app");
    fs::create_dir(&app).unwrap();
    let zip = root.join("Feather Mark.app.zip");
    let dmg = root.join("Feather Mark.dmg");

    let sign = macos_adhoc_codesign_plan(&app).unwrap();
    assert_eq!(sign.program, "codesign");
    assert_eq!(sign.args.last().unwrap(), app.as_os_str());
    assert!(!sign.args.iter().any(|arg| arg == "sh" || arg == "-c"));

    let verify = macos_codesign_verify_plan(&app).unwrap();
    assert_eq!(verify.program, "codesign");
    assert!(verify.args.iter().any(|arg| arg == "--strict"));

    let zip_plan = macos_zip_plan(&app, &zip).unwrap();
    assert_eq!(zip_plan.program, "ditto");
    assert!(zip_plan.args.iter().any(|arg| arg == "--sequesterRsrc"));
    assert!(zip_plan.args.iter().any(|arg| arg == "--keepParent"));
    assert_eq!(zip_plan.args.last().unwrap(), zip.as_os_str());

    let create = macos_dmg_plan(&app, &dmg).unwrap();
    assert_eq!(create.program, "hdiutil");
    assert_eq!(create.args.last().unwrap(), dmg.as_os_str());
    assert!(!create.args.iter().any(|arg| arg == "-ov"));
    assert!(!create.args.iter().any(|arg| arg == "sh" || arg == "-c"));

    // Finalization reads existing artifacts; use distinct existing paths.
    let existing_zip = root.join("existing.zip");
    let existing_dmg = root.join("existing.dmg");
    fs::write(&existing_zip, b"test-only zip bytes").unwrap();
    fs::write(&existing_dmg, b"test-only dmg bytes").unwrap();

    let zip_manifest = finalize_macos_zip_manifest(
        &existing_zip,
        &sha256(b"candidate"),
        &sha256(b"signed"),
        &valid_source_commit(),
        "0.1.0",
    )
    .unwrap();
    assert_eq!(zip_manifest.label, MACOS_PACKAGE_LABEL);
    assert_eq!(
        zip_manifest.artifact,
        std::path::PathBuf::from("existing.zip")
    );
    assert_eq!(zip_manifest.artifact_sha256, sha256(b"test-only zip bytes"));
    assert_eq!(zip_manifest.target_triple, "aarch64-apple-darwin");
    assert!(!zip_manifest.notarized);

    let manifest = finalize_macos_dmg_manifest(
        &existing_dmg,
        &sha256(b"candidate"),
        &sha256(b"signed"),
        &valid_source_commit(),
        "0.1.0",
    )
    .unwrap();
    assert_eq!(manifest.label, MACOS_PACKAGE_LABEL);
    assert_eq!(manifest.artifact, std::path::PathBuf::from("existing.dmg"));
    assert_eq!(manifest.artifact_sha256, sha256(b"test-only dmg bytes"));
    assert_eq!(manifest.source_commit, valid_source_commit());
    assert_eq!(manifest.version, "0.1.0");
    assert!(!manifest.notarized);
}

#[test]
fn prepares_linux_layout_with_locked_gtk3_webkitgtk41_dependencies() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("feathermark");
    let bytes = elf_x86_64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("linux-output");
    fs::create_dir(&output).unwrap();

    let receipt = prepare_linux_layout(&LinuxPackageRequest {
        candidate,
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.clone(),
        version: "0.1.0".into(),
    })
    .unwrap();

    assert_eq!(receipt.label, LINUX_PACKAGE_LABEL);
    let executable = output
        .join("_staging")
        .join("archive")
        .join("FeatherMark-linux-x86_64/bin/feathermark");
    assert_eq!(fs::read(&executable).unwrap(), bytes);
    assert_eq!(
        fs::metadata(executable).unwrap().permissions().mode() & 0o777,
        0o755
    );

    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            output
                .join("_staging")
                .join("archive")
                .join("FeatherMark-linux-x86_64/package-manifest-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["label"], LINUX_PACKAGE_LABEL);
    assert_eq!(manifest["wayland_verified"], false);
    assert_eq!(manifest["rpm_runtime_verified"], false);
    let dependencies = manifest["runtime_dependencies"].as_array().unwrap();
    assert!(
        dependencies
            .iter()
            .any(|row| row["soname"] == "libgtk-3.so.0")
    );
    assert!(
        dependencies
            .iter()
            .any(|row| row["soname"] == "libwebkit2gtk-4.1.so.0")
    );
    assert!(
        dependencies
            .iter()
            .any(|row| row["soname"] == "libgtksourceview-4.so.0")
    );
    assert!(
        !dependencies
            .iter()
            .any(|row| row["soname"] == "libgtk-4.so.1")
    );
    assert!(
        !dependencies
            .iter()
            .any(|row| row["soname"] == "libwebkitgtk-6.0.so.4")
    );
}

#[test]
fn linux_runtime_dependency_table_rejects_gtk4_and_webkitgtk6() {
    let sonames: Vec<_> = LINUX_RUNTIME_DEPENDENCIES
        .iter()
        .map(|dep| dep.soname)
        .collect();
    assert!(sonames.contains(&"libgtk-3.so.0"));
    assert!(sonames.contains(&"libwebkit2gtk-4.1.so.0"));
    assert!(!sonames.iter().any(|name| name.starts_with("libgtk-4")));
    assert!(
        !sonames
            .iter()
            .any(|name| name.starts_with("libwebkitgtk-6.0"))
    );
}

#[test]
fn prepares_debian_staging_with_locked_dependencies() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("feathermark");
    let bytes = elf_x86_64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("deb-output");
    fs::create_dir(&output).unwrap();

    let receipt = prepare_debian_staging(&LinuxPackageRequest {
        candidate,
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.clone(),
        version: "0.1.0".into(),
    })
    .unwrap();

    let binary = receipt.output.join("usr/bin/feathermark");
    assert_eq!(fs::read(&binary).unwrap(), bytes);
    let control = fs::read_to_string(receipt.output.join("DEBIAN/control")).unwrap();
    assert!(control.contains(
        "Depends: libgtk-3-0, libgtksourceview-4-0, libwebkit2gtk-4.1-0, libjavascriptcoregtk-4.1-0"
    ));
    assert!(control.contains("Architecture: amd64"));

    let plan = debian_package_plan(&receipt.output, &root.join("out.deb")).unwrap();
    assert_eq!(plan.program, "dpkg-deb");
    assert!(plan.args.iter().any(|arg| arg == "--root-owner-group"));
    assert!(plan.args.iter().any(|arg| arg == "--build"));
    assert!(!plan.args.iter().any(|arg| arg == "sh" || arg == "-c"));
}

#[test]
fn prepares_rpm_staging_with_locked_requirements() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("feathermark");
    let bytes = elf_x86_64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("rpm-output");
    fs::create_dir(&output).unwrap();

    let receipt = prepare_rpm_staging(&LinuxPackageRequest {
        candidate: candidate.clone(),
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.clone(),
        version: "0.1.0".into(),
    })
    .unwrap();

    let spec = fs::read_to_string(receipt.output.join("SPECS/feathermark.spec")).unwrap();
    assert!(spec.contains("Version:        0.1.0"));
    assert!(spec.contains("BuildArch:      x86_64"));
    assert!(spec.contains("Requires:       gtk3, gtksourceview4, webkit2gtk4.1"));
    assert!(spec.contains(&format!("install -D -m 0755 {}", candidate.display())));
    assert!(!spec.contains("%post"));

    let plan = rpm_package_plan(
        &receipt.output,
        &receipt.output.join("SPECS/feathermark.spec"),
    )
    .unwrap();
    assert_eq!(plan.program, "rpmbuild");
    assert!(
        plan.args
            .iter()
            .any(|arg| arg.to_string_lossy().contains("_topdir"))
    );
    assert!(plan.args.iter().any(|arg| arg == "-bb"));
    assert!(!plan.args.iter().any(|arg| arg == "sh" || arg == "-c"));
}

#[test]
fn linux_archive_plan_is_deterministic_and_manifest_hashes_existing_tar_zst() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let layout = root.join("FeatherMark-linux-x86_64");
    fs::create_dir(&layout).unwrap();
    let archive = root.join("FeatherMark linux.tar.zst");
    let plan = linux_archive_plan(&layout, &archive).unwrap();

    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].program, "tar");
    assert!(plan[0].args.iter().any(|arg| arg == "--sort=name"));
    assert!(plan[0].args.iter().any(|arg| arg == "--mtime=@0"));
    assert_eq!(plan[1].program, "zstd");
    assert_eq!(plan[1].args.last().unwrap(), archive.as_os_str());
    assert!(
        plan.iter()
            .all(|step| !step.args.iter().any(|arg| arg == "sh" || arg == "-c"))
    );

    fs::write(&archive, b"test-only tar.zst bytes").unwrap();
    let manifest = finalize_linux_archive_manifest(
        &archive,
        &sha256(b"candidate"),
        &sha256(b"candidate"),
        &valid_source_commit(),
        "0.1.0",
    )
    .unwrap();
    assert_eq!(manifest.label, LINUX_PACKAGE_LABEL);
    assert_eq!(
        manifest.artifact,
        std::path::PathBuf::from("FeatherMark linux.tar.zst")
    );
    assert_eq!(manifest.artifact_sha256, sha256(b"test-only tar.zst bytes"));
    assert_eq!(manifest.target_triple, "x86_64-unknown-linux-gnu");
    assert!(!manifest.wayland_verified);
    assert!(!manifest.rpm_runtime_verified);
}

#[test]
fn rejects_candidate_hash_mismatch_and_symlink_inputs() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("candidate");
    fs::write(&candidate, b"candidate").unwrap();
    let symlinked = root.join("candidate-link");
    symlink(&candidate, &symlinked).unwrap();
    let output = root.join("output");
    fs::create_dir(&output).unwrap();

    let mismatch = assemble_macos_app(&MacPackageRequest {
        candidate: candidate.clone(),
        build_input_sha256: "00".repeat(32),
        source_commit: valid_source_commit(),
        output_root: output.join("mismatch-output"),
        version: "0.1.0".into(),
    })
    .unwrap_err();
    assert!(
        mismatch
            .to_string()
            .contains("build-input SHA-256 mismatch")
    );

    let linked = prepare_linux_layout(&LinuxPackageRequest {
        candidate: symlinked,
        build_input_sha256: sha256(b"candidate"),
        source_commit: valid_source_commit(),
        output_root: output.join("linked-output"),
        version: "0.1.0".into(),
    })
    .unwrap_err();
    assert!(linked.to_string().contains("symlink"));
}

#[test]
fn rejects_relative_and_parent_traversal_paths_before_io() {
    let request = MacPackageRequest {
        candidate: "../candidate".into(),
        build_input_sha256: "00".repeat(32),
        source_commit: valid_source_commit(),
        output_root: "relative/output".into(),
        version: "0.1.0".into(),
    };
    let error = assemble_macos_app(&request).unwrap_err();
    assert!(error.to_string().contains("absolute normalized path"));
}

#[test]
fn rejects_candidates_whose_binary_architecture_conflicts_with_package_label() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("wrong-architecture");
    let bytes = elf_x86_64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("output");
    fs::create_dir(&output).unwrap();

    let error = assemble_macos_app(&MacPackageRequest {
        candidate: candidate.clone(),
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.join("macos-output"),
        version: "0.1.0".into(),
    })
    .unwrap_err();

    assert!(error.to_string().contains("Mach-O arm64"));

    let bytes = mach_o_arm64();
    fs::write(&candidate, &bytes).unwrap();
    let error = prepare_linux_layout(&LinuxPackageRequest {
        candidate,
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.join("linux-output"),
        version: "0.1.0".into(),
    })
    .unwrap_err();
    assert!(error.to_string().contains("ELF x86_64"));
}

#[test]
fn source_commit_must_be_40_lowercase_hex() {
    let ok = "a".repeat(40);
    assert!(xtask::local_package::validate_source_commit(&ok).is_ok());
    let zero = "0".repeat(40);
    assert!(xtask::local_package::validate_source_commit(&zero).is_ok());
    let uppercase = "A".repeat(40);
    assert!(xtask::local_package::validate_source_commit(&uppercase).is_err());
    let short = "a".repeat(39);
    assert!(xtask::local_package::validate_source_commit(&short).is_err());
    let long = "a".repeat(41);
    assert!(xtask::local_package::validate_source_commit(&long).is_err());
    let non_hex = "g".repeat(40);
    assert!(xtask::local_package::validate_source_commit(&non_hex).is_err());
}

#[test]
fn create_output_root_rejects_existing_paths() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let existing = root.join("exists");
    fs::create_dir(&existing).unwrap();
    let err = create_package_output_root(&existing).unwrap_err();
    assert!(err.to_string().contains("output already exists"));
}

#[test]
fn sha256_regular_file_rejects_symlinks_and_hashes_regular_files() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let file = root.join("file");
    fs::write(&file, b"hello").unwrap();
    let link = root.join("link");
    symlink(&file, &link).unwrap();

    assert_eq!(sha256_regular_file(&file).unwrap(), sha256(b"hello"));
    assert!(
        sha256_regular_file(&link)
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
}

#[test]
fn executable_size_gate_rejects_oversize_candidates() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("big");
    let mut bytes = mach_o_arm64();
    bytes.resize((MAX_EXECUTABLE_BYTES + 1) as usize, 0);
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("output");
    fs::create_dir(&output).unwrap();

    let err = assemble_macos_app(&MacPackageRequest {
        candidate,
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.join("macos-output"),
        version: "0.1.0".into(),
    })
    .unwrap_err();
    assert!(err.to_string().contains("executable exceeds maximum size"));
}

#[test]
fn artifact_size_gate_rejects_oversize_artifacts() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let artifact = root.join("big.dmg");
    let bytes = vec![0u8; (MAX_ARTIFACT_BYTES + 1) as usize];
    fs::write(&artifact, &bytes).unwrap();

    let err = finalize_macos_dmg_manifest(
        &artifact,
        &sha256(b"candidate"),
        &sha256(b"signed"),
        &valid_source_commit(),
        "0.1.0",
    )
    .unwrap_err();
    assert!(err.to_string().contains("artifact exceeds maximum size"));
}

#[derive(Default)]
struct RecordingExecutor {
    calls: Mutex<Vec<(String, Vec<OsString>)>>,
    fail_after: Mutex<Option<usize>>,
}

impl RecordingExecutor {
    fn record(&self, program: &str, args: &[OsString]) {
        self.calls
            .lock()
            .unwrap()
            .push((program.to_owned(), args.to_vec()));
    }

    fn calls(&self) -> Vec<(String, Vec<OsString>)> {
        self.calls.lock().unwrap().clone()
    }

    fn fail_after(&self, n: usize) {
        *self.fail_after.lock().unwrap() = Some(n);
    }
}

impl CommandExecutor for RecordingExecutor {
    fn execute(
        &self,
        plan: &xtask::local_package::CommandPlan,
    ) -> Result<(), LocalPackageCliError> {
        self.record(&plan.program, &plan.args);
        if let Some(fail_after) = *self.fail_after.lock().unwrap() {
            if self.calls.lock().unwrap().len() >= fail_after {
                return Err(LocalPackageCliError::ToolFailed {
                    program: plan.program.clone(),
                    status: Some(1),
                });
            }
        }
        // Create placeholder outputs for artifact-producing tools so that
        // finalize_*_manifest calls can read them in integration tests.
        create_dummy_output_if_needed(plan);
        Ok(())
    }
}

fn create_dummy_output_if_needed(plan: &xtask::local_package::CommandPlan) {
    let output_path: Option<std::path::PathBuf> = match plan.program.as_str() {
        "ditto" | "hdiutil" | "dpkg-deb" | "zstd" => plan.args.last().map(Into::into),
        "tar" => {
            // tar -cf <intermediate.tar> ...
            plan.args
                .windows(2)
                .find(|pair| pair[0] == "-cf")
                .map(|pair| std::path::PathBuf::from(&pair[1]))
        }
        "rpmbuild" => {
            // rpmbuild --define "_topdir <topdir>" -bb <spec>
            plan.args
                .windows(2)
                .find(|pair| pair[0] == "--define")
                .and_then(|pair| {
                    let def = pair[1].to_string_lossy();
                    def.strip_prefix("_topdir ").map(|topdir| {
                        std::path::PathBuf::from(topdir)
                            .join("RPMS/x86_64/feathermark-0.1.0-1.x86_64.rpm")
                    })
                })
        }
        _ => None,
    };
    if let Some(path) = output_path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&path, b"placeholder artifact").ok();
    }
}

#[test]
fn fake_executor_records_ordered_invocations_and_preserves_arguments_with_spaces() {
    let executor = RecordingExecutor::default();
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let app = root.join("Feather Mark.app");
    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    let zip = root.join("Feather Mark.app.zip");
    let dmg = root.join("Feather Mark.dmg");

    executor
        .execute(&macos_zip_plan(&app, &zip).unwrap())
        .unwrap();
    executor
        .execute(&macos_dmg_plan(&app, &dmg).unwrap())
        .unwrap();

    let calls = executor.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "ditto");
    assert!(calls[0].1.contains(&app.as_os_str().to_owned()));
    assert!(calls[0].1.contains(&zip.as_os_str().to_owned()));
    assert_eq!(calls[1].0, "hdiutil");
    assert!(calls[1].1.contains(&dmg.as_os_str().to_owned()));
    assert!(!calls.iter().any(|(program, _)| program == "sh"));
}

#[test]
fn fake_executor_propagates_nonzero_failure() {
    let executor = RecordingExecutor::default();
    executor.fail_after(1);

    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let app = root.join("FeatherMark.app");
    fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    let zip = root.join("FeatherMark.zip");

    let result = executor.execute(&macos_zip_plan(&app, &zip).unwrap());
    assert!(result.is_err());
}

#[test]
fn run_local_package_macos_produces_manifests_and_cleans_staging() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("candidate");
    let bytes = mach_o_arm64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("macos-output");

    let executor = RecordingExecutor::default();
    let request = LocalPackageCliRequest::Macos(MacPackageRequest {
        candidate,
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.clone(),
        version: "0.1.0".into(),
    });

    let manifests = run_local_package(request, &executor).unwrap();
    assert_eq!(manifests.len(), 2);
    assert_eq!(
        manifests[0].artifact.as_os_str().to_string_lossy(),
        MACOS_ZIP_NAME
    );
    assert_eq!(
        manifests[1].artifact.as_os_str().to_string_lossy(),
        MACOS_DMG_NAME
    );
    assert_eq!(manifests[0].build_input_sha256, sha256(&bytes));
    assert_eq!(manifests[0].packaged_executable_sha256, sha256(&bytes));

    let calls = executor.calls();
    assert_eq!(calls[0].0, "codesign");
    assert_eq!(calls[1].0, "codesign");
    assert_eq!(calls[2].0, "ditto");
    assert_eq!(calls[3].0, "hdiutil");

    assert!(!output.join("_staging").exists());
}

#[test]
fn run_local_package_linux_produces_manifests_and_cleans_staging() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("candidate");
    let bytes = elf_x86_64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("linux-output");

    let executor = RecordingExecutor::default();
    let request = LocalPackageCliRequest::Linux(LinuxPackageRequest {
        candidate,
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.clone(),
        version: "0.1.0".into(),
    });

    let manifests = run_local_package(request, &executor).unwrap();
    assert_eq!(manifests.len(), 3);
    assert_eq!(
        manifests[0].artifact.as_os_str().to_string_lossy(),
        "FeatherMark-0.1.0-linux-x86_64.tar.zst"
    );
    assert_eq!(manifests[0].build_input_sha256, sha256(&bytes));
    assert_eq!(manifests[0].packaged_executable_sha256, sha256(&bytes));
    assert!(!manifests[0].wayland_verified);
    assert!(!manifests[0].rpm_runtime_verified);

    let calls = executor.calls();
    assert_eq!(calls[0].0, "tar");
    assert_eq!(calls[1].0, "zstd");
    assert_eq!(calls[2].0, "dpkg-deb");
    assert_eq!(calls[3].0, "rpmbuild");

    assert!(!output.join("_staging").exists());
}

#[test]
fn run_local_package_retains_staging_on_failure() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("candidate");
    let bytes = mach_o_arm64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("macos-failure-output");

    let executor = RecordingExecutor::default();
    executor.fail_after(1);
    let request = LocalPackageCliRequest::Macos(MacPackageRequest {
        candidate,
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.clone(),
        version: "0.1.0".into(),
    });

    assert!(run_local_package(request, &executor).is_err());
    assert!(output.join("_staging").exists());
}

#[test]
fn no_overwrite_of_existing_output_root_or_artifacts() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("candidate");
    let bytes = mach_o_arm64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("output");
    fs::create_dir(&output).unwrap();

    let request = LocalPackageCliRequest::Macos(MacPackageRequest {
        candidate,
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.clone(),
        version: "0.1.0".into(),
    });

    let err = run_local_package(request, &RecordingExecutor::default()).unwrap_err();
    assert!(err.to_string().contains("output already exists"));
}

#[test]
fn clap_parses_local_macos_command() {
    use clap::Parser;
    let hash = "a".repeat(64);
    let commit = "b".repeat(40);
    let args = vec![
        "xtask",
        "package",
        "local",
        "macos",
        "--candidate",
        "/build/feathermark",
        "--build-input-sha256",
        &hash,
        "--source-commit",
        &commit,
        "--output-root",
        "/out/macos",
        "--version",
        "0.1.0",
    ];
    let cli = Cli::parse_from(args);
    match cli.command {
        Command::Package {
            command: PackageCommand::Local { command },
        } => match command {
            LocalPackageCommand::Macos {
                candidate,
                build_input_sha256,
                source_commit,
                output_root,
                version,
            } => {
                assert_eq!(candidate, PathBuf::from("/build/feathermark"));
                assert_eq!(build_input_sha256, hash);
                assert_eq!(source_commit, commit);
                assert_eq!(output_root, PathBuf::from("/out/macos"));
                assert_eq!(version, "0.1.0");
            }
            _ => panic!("expected macos subcommand"),
        },
        _ => panic!("expected package local command"),
    }
}

#[test]
fn clap_parses_local_linux_command() {
    use clap::Parser;
    let hash = "c".repeat(64);
    let commit = "d".repeat(40);
    let args = vec![
        "xtask",
        "package",
        "local",
        "linux",
        "--candidate",
        "/build/feathermark",
        "--build-input-sha256",
        &hash,
        "--source-commit",
        &commit,
        "--output-root",
        "/out/linux",
        "--version",
        "0.1.0",
    ];
    let cli = Cli::parse_from(args);
    match cli.command {
        Command::Package {
            command: PackageCommand::Local { command },
        } => match command {
            LocalPackageCommand::Linux {
                candidate,
                build_input_sha256,
                source_commit,
                output_root,
                version,
            } => {
                assert_eq!(candidate, PathBuf::from("/build/feathermark"));
                assert_eq!(build_input_sha256, hash);
                assert_eq!(source_commit, commit);
                assert_eq!(output_root, PathBuf::from("/out/linux"));
                assert_eq!(version, "0.1.0");
            }
            _ => panic!("expected linux subcommand"),
        },
        _ => panic!("expected package local command"),
    }
}

#[test]
fn process_executor_rejects_nonzero_status() {
    let plan = xtask::local_package::CommandPlan {
        program: "false".into(),
        args: vec![],
    };
    let err = ProcessCommandExecutor.execute(&plan).unwrap_err();
    assert!(err.to_string().contains("tool failed"));
}

#[test]
fn artifact_manifest_contains_exact_locked_fields() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let artifact = root.join("FeatherMark-0.1.0-macos-arm64.dmg");
    fs::write(&artifact, b"x").unwrap();

    let manifest = finalize_macos_dmg_manifest(
        &artifact,
        &"a".repeat(64),
        &"b".repeat(64),
        &valid_source_commit(),
        "0.1.0",
    )
    .unwrap();

    let json = serde_json::to_value(&manifest).unwrap();
    let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    let mut expected = vec![
        "schema",
        "label",
        "artifact",
        "artifact_sha256",
        "build_input_sha256",
        "packaged_executable_sha256",
        "source_commit",
        "version",
        "target_triple",
        "notarized",
        "wayland_verified",
        "rpm_runtime_verified",
    ];
    expected.sort();
    assert_eq!(keys, expected);
}

#[test]
fn json_receipt_hashes_bind_to_artifact_bytes() {
    let temporary = tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let candidate = root.join("candidate");
    let bytes = mach_o_arm64();
    fs::write(&candidate, &bytes).unwrap();
    let output = root.join("receipt-output");

    let executor = RecordingExecutor::default();
    let request = LocalPackageCliRequest::Macos(MacPackageRequest {
        candidate,
        build_input_sha256: sha256(&bytes),
        source_commit: valid_source_commit(),
        output_root: output.clone(),
        version: "0.1.0".into(),
    });

    let manifests = run_local_package(request, &executor).unwrap();
    let json = serde_json::to_string_pretty(&manifests).unwrap();
    assert!(json.contains(&sha256(&bytes)));
    assert!(json.contains(&valid_source_commit()));
    assert!(json.contains("0.1.0"));
}

use xtask::cli::{Cli, Command, LocalPackageCommand, PackageCommand};
