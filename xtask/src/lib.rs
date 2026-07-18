//! Deterministic FeatherMark build and evidence driver.

#[allow(dead_code)] // Task 1C wires the capability boundary to real application launchers.
mod app_launch;
pub mod artifact_inspector;
#[allow(dead_code)] // Task 1C wires the capability boundary to real scenario owners.
mod candidate;
pub mod comparator;
pub mod evidence;
pub mod evidence_bind;
pub mod fixtures;
pub mod gui;
#[cfg(unix)]
pub mod linux_gate;
pub mod local_package;
pub mod local_package_cli;
pub mod metrics;
#[cfg(unix)]
pub mod native_smoke;
pub mod package;
pub mod package_smoke;
pub mod provenance;
pub mod readiness;
pub mod readiness_keystone;
pub mod release_authority;
pub mod release_preflight;
pub mod reproducible_build;
pub mod runner;
#[doc(hidden)]
pub mod runner_native;
mod tool_process;

/// Clap-driven CLI surface shared between the xtask binary and its tests.
pub mod cli {
    use std::num::NonZeroUsize;
    use std::path::PathBuf;

    #[cfg(unix)]
    use crate::native_smoke::NativeSmokeProfile;
    use crate::package_smoke::PackageKind;
    use clap::{Parser, Subcommand, ValueEnum};

    #[derive(Parser)]
    #[command(
        name = "xtask",
        version,
        about = "FeatherMark build and evidence driver"
    )]
    pub struct Cli {
        #[command(subcommand)]
        pub command: Command,
    }

    #[derive(Subcommand)]
    pub enum Command {
        Artifact {
            #[command(subcommand)]
            command: ArtifactCommand,
        },
        Fixtures {
            #[command(subcommand)]
            command: FixtureCommand,
        },
        Comparator {
            #[command(subcommand)]
            command: ComparatorCommand,
        },
        Evidence {
            #[command(subcommand)]
            command: EvidenceCommand,
        },
        Gui {
            #[command(subcommand)]
            command: GuiCommand,
        },
        Metrics {
            #[command(subcommand)]
            command: MetricsCommand,
        },
        #[cfg(unix)]
        NativeSmoke {
            #[arg(long)]
            binary: PathBuf,
            #[arg(long, value_enum)]
            profile: NativeSmokeProfile,
            #[arg(long)]
            repeat: Option<NonZeroUsize>,
            #[arg(long)]
            evidence_dir: PathBuf,
        },
        #[cfg(unix)]
        LinuxGate {
            #[arg(long)]
            binary: PathBuf,
            #[arg(long, value_enum)]
            profile: NativeSmokeProfile,
            #[arg(long)]
            cycles: NonZeroUsize,
            #[arg(long)]
            exit_code: i32,
            #[arg(long)]
            started_ms: u128,
            #[arg(long)]
            ended_ms: u128,
            #[arg(long)]
            stdout_log: PathBuf,
            #[arg(long)]
            stderr_log: PathBuf,
            #[arg(long)]
            evidence_dir: PathBuf,
        },
        Package {
            #[command(subcommand)]
            command: PackageCommand,
        },
        Runner {
            #[command(subcommand)]
            command: RunnerCommand,
        },
        ReleasePreflight {
            #[arg(long)]
            input: PathBuf,
            #[arg(long)]
            out: PathBuf,
        },
        ReproducibleBuild {
            #[arg(long, default_value = "feathermark-app")]
            package: String,
            #[arg(long, default_value = "feathermark")]
            bin: String,
            #[arg(long)]
            features: Option<String>,
        },
        Provenance {
            #[command(subcommand)]
            command: ProvenanceCommand,
        },
        Release {
            #[command(subcommand)]
            command: ReleaseCommand,
        },
        Readiness {
            #[command(subcommand)]
            command: ReadinessCommand,
        },
    }

    #[derive(Subcommand)]
    pub enum ReleaseCommand {
        /// Generate a fresh release-authority ed25519 keypair. Writes the public
        /// key to --public-out (committed as the pinned trust anchor) and prints
        /// the secret hex to stdout for the operator to install privately (0600).
        Keygen {
            #[arg(long)]
            public_out: PathBuf,
        },
        /// Sign a preview-publication authorization binding an artifact to its
        /// production provenance, using the release-authority secret key.
        PreviewAuthorize {
            #[arg(long)]
            artifact: PathBuf,
            #[arg(long)]
            provenance: PathBuf,
            #[arg(long)]
            product: String,
            #[arg(long)]
            version_label: String,
            #[arg(long)]
            signed_at: String,
            #[arg(long)]
            expires_at: String,
            #[arg(long, env = "FEATHERMARK_RELEASE_AUTHORITY_KEY")]
            key_path: PathBuf,
            #[arg(long)]
            out: PathBuf,
        },
    }

    #[derive(Subcommand)]
    pub enum ProvenanceCommand {
        /// Generate a production-provenance record for a candidate produced by
        /// `reproducible-build`. Re-derives SOURCE_DATE_EPOCH + the
        /// --remap-path-prefix RUSTFLAGS from the same sources reproducible-build
        /// uses, so the command is self-contained. The operator asserts the
        /// candidate was built reproducibly; the record anchors the candidate
        /// SHA-256 so a reviewer can rebuild and compare.
        Generate {
            #[arg(long)]
            candidate: PathBuf,
            #[arg(long)]
            repo_root: PathBuf,
            #[arg(long, value_delimiter = ',')]
            features: Vec<String>,
            #[arg(long, default_value = "target/prod")]
            target_root: String,
            #[arg(long)]
            product: String,
            #[arg(long)]
            product_version: String,
            #[arg(long)]
            out: PathBuf,
        },
    }

    #[derive(Subcommand)]
    pub enum ArtifactCommand {
        Inspect {
            #[arg(long)]
            artifact: PathBuf,
            #[arg(long)]
            quarantine: Option<PathBuf>,
            #[arg(long)]
            policy: Option<PathBuf>,
            #[arg(long, value_enum, default_value_t = ArtifactInspectionMode::Package)]
            mode: ArtifactInspectionMode,
            #[arg(long)]
            provenance: Option<PathBuf>,
        },
    }

    #[derive(Clone, Copy, Debug, ValueEnum)]
    pub enum ArtifactInspectionMode {
        Candidate,
        Package,
    }

    #[derive(Subcommand)]
    pub enum FixtureCommand {
        Generate {
            #[arg(long)]
            out: PathBuf,
        },
        Verify {
            #[arg(long)]
            dir: PathBuf,
        },
    }

    #[derive(Subcommand)]
    pub enum ComparatorCommand {
        Scaffold {
            #[command(subcommand)]
            command: ScaffoldCommand,
        },
    }

    #[derive(Subcommand)]
    pub enum EvidenceCommand {
        /// Validate a JSON instance against a checked-in rutile schema.
        Validate {
            #[arg(long)]
            input: PathBuf,
            /// Schema kind, e.g. "production-provenance" or "evidence-index".
            /// Maps to schemas/rutile.<kind>.v1.schema.json.
            #[arg(long)]
            schema: String,
        },
        /// Bind a plain gate index to production provenance and emit a canonical
        /// `rutile.evidence-index.v1` record. Create-only; never signs.
        Bind {
            #[arg(long)]
            plain_index: PathBuf,
            #[arg(long)]
            provenance: PathBuf,
            #[arg(long)]
            evidence_root: PathBuf,
            #[arg(long)]
            out: PathBuf,
        },
    }

    #[derive(Subcommand)]
    pub enum GuiCommand {
        ValidateTranscript {
            #[arg(long)]
            commands: PathBuf,
            #[arg(long)]
            events: PathBuf,
        },
    }

    #[derive(Subcommand)]
    pub enum MetricsCommand {
        AssertRecord {
            #[arg(long)]
            input: PathBuf,
            #[arg(long)]
            minimum_samples: usize,
            #[arg(long)]
            maximum_p95: u64,
        },
    }

    #[derive(Subcommand)]
    pub enum PackageCommand {
        AssertFile {
            #[arg(long)]
            path: PathBuf,
            #[arg(long)]
            sha256: String,
            #[arg(long)]
            maximum_bytes: u64,
        },
        Local {
            #[command(subcommand)]
            command: LocalPackageCommand,
        },
        /// Run the bounded install/open/uninstall smoke row against a produced
        /// package on the current host. Proves each stage ran; fails closed on
        /// any nonzero/timeout/residue. Never invokes a shell.
        SmokeRow {
            #[arg(long)]
            package: PathBuf,
            #[arg(long, value_enum)]
            kind: PackageSmokeKind,
            #[arg(long)]
            package_sha256: String,
            #[arg(long)]
            source_commit: String,
            #[arg(long)]
            install_target: PathBuf,
            #[arg(long)]
            binary_path: PathBuf,
            /// Expected SHA-256 (64-char lowercase hex) of the executable at
            /// `--binary-path` after install. The engine re-hashes the installed
            /// binary and rejects any drift.
            #[arg(long)]
            expected_executable_sha256: String,
            #[arg(long)]
            evidence_dir: PathBuf,
            #[arg(long)]
            deadline_secs: Option<u64>,
            #[arg(long)]
            term_grace_secs: Option<u64>,
        },
    }

    #[derive(Subcommand)]
    pub enum LocalPackageCommand {
        Macos {
            #[arg(long)]
            candidate: PathBuf,
            #[arg(long)]
            build_input_sha256: String,
            #[arg(long)]
            source_commit: String,
            #[arg(long)]
            output_root: PathBuf,
            #[arg(long)]
            version: String,
            /// Optional release-authority secret key. When present (with
            /// --preview-signed-at + --preview-expires-at), bless each produced
            /// artifact with provenance + a signed preview authorization so the
            /// inline Package-mode inspection passes at the preview tier.
            #[arg(long)]
            release_authority_key: Option<PathBuf>,
            #[arg(long)]
            preview_signed_at: Option<String>,
            #[arg(long)]
            preview_expires_at: Option<String>,
        },
        Linux {
            #[arg(long)]
            candidate: PathBuf,
            #[arg(long)]
            build_input_sha256: String,
            #[arg(long)]
            source_commit: String,
            #[arg(long)]
            output_root: PathBuf,
            #[arg(long)]
            version: String,
        },
    }

    #[derive(Subcommand)]
    pub enum RunnerCommand {
        CaptureVerifyMatrix {
            #[arg(long, value_delimiter = ',')]
            runners: Vec<String>,
            #[arg(long)]
            capture_dir: PathBuf,
            #[arg(long)]
            out: PathBuf,
        },
    }

    #[derive(Subcommand)]
    pub enum ScaffoldCommand {
        Create {
            #[arg(long)]
            fixtures: PathBuf,
            #[arg(long, value_delimiter = ',')]
            contracts: Vec<PathBuf>,
            #[arg(long)]
            xtask: PathBuf,
            #[arg(long)]
            out: PathBuf,
            #[arg(long)]
            lock: PathBuf,
        },
        Verify {
            #[arg(long)]
            repo: PathBuf,
            #[arg(long)]
            lock: PathBuf,
        },
    }

    #[derive(Subcommand)]
    pub enum ReadinessCommand {
        /// Verify an externally-signed readiness attestation against the pinned
        /// independent trusted-verifier key and committed runner lock. Verifies
        /// only; never signs. Fails closed until G004 provisions the verifier key.
        Verify {
            #[arg(long)]
            input: PathBuf,
            #[arg(long)]
            runner_lock: PathBuf,
        },
        /// Re-verify the attestation and publish a create-only canonical copy.
        /// Never signs; `publication_authorized` stays false.
        Publish {
            #[arg(long)]
            input: PathBuf,
            #[arg(long)]
            runner_lock: PathBuf,
            #[arg(long)]
            out: PathBuf,
        },
    }

    /// CLI mirror of `package_smoke::PackageKind`. Defined locally so the parse
    /// layer stays decoupled from whether the module derives clap. Use
    /// [`PackageSmokeKind::to_smoke_kind`] for the 1:1 conversion.
    #[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
    pub enum PackageSmokeKind {
        MacosAppZip,
        MacosDmg,
        LinuxDeb,
        LinuxRpm,
        LinuxTarZst,
    }

    impl PackageSmokeKind {
        /// Convert the CLI-local kind to the engine's `PackageKind`. This is the
        /// single dispatch point — exhaustive, so adding a variant without
        /// updating it is a compile error (fail-closed).
        pub fn to_smoke_kind(self) -> PackageKind {
            match self {
                PackageSmokeKind::MacosAppZip => PackageKind::MacosAppZip,
                PackageSmokeKind::MacosDmg => PackageKind::MacosDmg,
                PackageSmokeKind::LinuxDeb => PackageKind::LinuxDeb,
                PackageSmokeKind::LinuxRpm => PackageKind::LinuxRpm,
                PackageSmokeKind::LinuxTarZst => PackageKind::LinuxTarZst,
            }
        }
    }
}

#[cfg(test)]
mod cli_parse_tests {
    use clap::Parser;

    use super::cli::{
        Cli, Command, EvidenceCommand, PackageCommand, PackageSmokeKind, ReadinessCommand,
    };

    fn parse(args: &[&str]) -> Command {
        Cli::try_parse_from(args).unwrap().command
    }

    fn fail(args: &[&str]) -> String {
        Cli::try_parse_from(args)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_else(|| panic!("expected parse failure: {args:?}"))
    }

    #[test]
    fn readiness_verify_parses_required_inputs_only() {
        let Command::Readiness { command } = parse(&[
            "xtask",
            "readiness",
            "verify",
            "--input",
            "a.json",
            "--runner-lock",
            "lock",
        ]) else {
            panic!("expected Readiness command");
        };
        match command {
            ReadinessCommand::Verify { input, runner_lock } => {
                assert_eq!(input, std::path::Path::new("a.json"));
                assert_eq!(runner_lock, std::path::Path::new("lock"));
            }
            ReadinessCommand::Publish { .. } => panic!("expected Verify variant"),
        }
    }

    #[test]
    fn readiness_publish_parses_with_out() {
        let Command::Readiness { command } = parse(&[
            "xtask",
            "readiness",
            "publish",
            "--input",
            "a.json",
            "--runner-lock",
            "lock",
            "--out",
            "o.json",
        ]) else {
            panic!("expected Readiness command");
        };
        match command {
            ReadinessCommand::Publish {
                input,
                runner_lock,
                out,
            } => {
                assert_eq!(input, std::path::Path::new("a.json"));
                assert_eq!(runner_lock, std::path::Path::new("lock"));
                assert_eq!(out, std::path::Path::new("o.json"));
            }
            ReadinessCommand::Verify { .. } => panic!("expected Publish variant"),
        }
    }

    #[test]
    fn readiness_verify_rejects_missing_runner_lock() {
        let message = fail(&["xtask", "readiness", "verify", "--input", "a.json"]);
        assert!(message.contains("--runner-lock"), "{message}");
    }

    #[test]
    fn package_smoke_row_parses_all_five_kinds() {
        for (index, kind) in [
            "macos-app-zip",
            "macos-dmg",
            "linux-deb",
            "linux-rpm",
            "linux-tar-zst",
        ]
        .iter()
        .enumerate()
        {
            let Command::Package { command } = parse(&[
                "xtask",
                "package",
                "smoke-row",
                "--package",
                "p",
                "--kind",
                kind,
                "--package-sha256",
                "abc123",
                "--source-commit",
                "deadbeef",
                "--install-target",
                "i",
                "--binary-path",
                "b",
                "--expected-executable-sha256",
                "def456",
                "--evidence-dir",
                "e",
            ]) else {
                panic!("expected Package command");
            };
            match command {
                PackageCommand::SmokeRow {
                    kind: parsed_kind,
                    expected_executable_sha256,
                    deadline_secs,
                    term_grace_secs,
                    ..
                } => {
                    let expected = [
                        PackageSmokeKind::MacosAppZip,
                        PackageSmokeKind::MacosDmg,
                        PackageSmokeKind::LinuxDeb,
                        PackageSmokeKind::LinuxRpm,
                        PackageSmokeKind::LinuxTarZst,
                    ][index];
                    assert_eq!(parsed_kind, expected, "kind {kind}");
                    assert_eq!(expected_executable_sha256, "def456");
                    assert_eq!(deadline_secs, None);
                    assert_eq!(term_grace_secs, None);
                }
                _ => panic!("expected SmokeRow variant"),
            }
        }
    }

    #[test]
    fn package_smoke_row_parses_optional_timeouts() {
        let Command::Package { command } = parse(&[
            "xtask",
            "package",
            "smoke-row",
            "--package",
            "p",
            "--kind",
            "linux-tar-zst",
            "--package-sha256",
            "abc123",
            "--source-commit",
            "deadbeef",
            "--install-target",
            "i",
            "--binary-path",
            "b",
            "--expected-executable-sha256",
            "def456",
            "--evidence-dir",
            "e",
            "--deadline-secs",
            "120",
            "--term-grace-secs",
            "10",
        ]) else {
            panic!("expected Package command");
        };
        match command {
            PackageCommand::SmokeRow {
                deadline_secs,
                term_grace_secs,
                ..
            } => {
                assert_eq!(deadline_secs, Some(120));
                assert_eq!(term_grace_secs, Some(10));
            }
            _ => panic!("expected SmokeRow variant"),
        }
    }

    #[test]
    fn package_smoke_row_requires_binary_path() {
        let message = fail(&[
            "xtask",
            "package",
            "smoke-row",
            "--package",
            "p",
            "--kind",
            "linux-tar-zst",
            "--package-sha256",
            "abc123",
            "--source-commit",
            "deadbeef",
            "--install-target",
            "i",
            "--expected-executable-sha256",
            "def456",
            "--evidence-dir",
            "e",
        ]);
        assert!(message.contains("--binary-path"), "{message}");
    }

    #[test]
    fn package_smoke_row_requires_expected_executable_sha256() {
        let message = fail(&[
            "xtask",
            "package",
            "smoke-row",
            "--package",
            "p",
            "--kind",
            "linux-tar-zst",
            "--package-sha256",
            "abc123",
            "--source-commit",
            "deadbeef",
            "--install-target",
            "i",
            "--binary-path",
            "b",
            "--evidence-dir",
            "e",
        ]);
        assert!(
            message.contains("--expected-executable-sha256"),
            "{message}"
        );
    }

    #[test]
    fn package_smoke_row_rejects_unknown_kind() {
        let message = fail(&[
            "xtask",
            "package",
            "smoke-row",
            "--package",
            "p",
            "--kind",
            "windows-msi",
            "--package-sha256",
            "abc123",
            "--source-commit",
            "deadbeef",
            "--install-target",
            "i",
            "--binary-path",
            "b",
            "--expected-executable-sha256",
            "def456",
            "--evidence-dir",
            "e",
        ]);
        assert!(message.contains("invalid value 'windows-msi'"), "{message}");
    }

    #[test]
    fn package_smoke_kind_maps_exhaustively_to_engine_kind() {
        use crate::package_smoke::PackageKind;
        assert_eq!(
            PackageSmokeKind::MacosAppZip.to_smoke_kind(),
            PackageKind::MacosAppZip
        );
        assert_eq!(
            PackageSmokeKind::MacosDmg.to_smoke_kind(),
            PackageKind::MacosDmg
        );
        assert_eq!(
            PackageSmokeKind::LinuxDeb.to_smoke_kind(),
            PackageKind::LinuxDeb
        );
        assert_eq!(
            PackageSmokeKind::LinuxRpm.to_smoke_kind(),
            PackageKind::LinuxRpm
        );
        assert_eq!(
            PackageSmokeKind::LinuxTarZst.to_smoke_kind(),
            PackageKind::LinuxTarZst
        );
    }

    #[test]
    fn evidence_bind_parses_required_args() {
        let Command::Evidence { command } = parse(&[
            "xtask",
            "evidence",
            "bind",
            "--plain-index",
            "plain.json",
            "--provenance",
            "prov.json",
            "--evidence-root",
            "root",
            "--out",
            "out.json",
        ]) else {
            panic!("expected Evidence command");
        };
        match command {
            EvidenceCommand::Bind {
                plain_index,
                provenance,
                evidence_root,
                out,
            } => {
                assert_eq!(plain_index, std::path::Path::new("plain.json"));
                assert_eq!(provenance, std::path::Path::new("prov.json"));
                assert_eq!(evidence_root, std::path::Path::new("root"));
                assert_eq!(out, std::path::Path::new("out.json"));
            }
            EvidenceCommand::Validate { .. } => panic!("expected Bind variant"),
        }
    }

    #[test]
    fn evidence_bind_rejects_missing_out() {
        let message = fail(&[
            "xtask",
            "evidence",
            "bind",
            "--plain-index",
            "plain.json",
            "--provenance",
            "prov.json",
            "--evidence-root",
            "root",
        ]);
        assert!(message.contains("--out"), "{message}");
    }

    #[test]
    fn evidence_validate_still_parses_after_bind_addition() {
        let Command::Evidence { command } = parse(&[
            "xtask",
            "evidence",
            "validate",
            "--input",
            "i.json",
            "--schema",
            "evidence-index",
        ]) else {
            panic!("expected Evidence command");
        };
        match command {
            EvidenceCommand::Validate { input, schema } => {
                assert_eq!(input, std::path::Path::new("i.json"));
                assert_eq!(schema, "evidence-index");
            }
            EvidenceCommand::Bind { .. } => panic!("expected Validate variant"),
        }
    }
}
