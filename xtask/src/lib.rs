//! Deterministic FeatherMark build and evidence driver.

#[allow(dead_code)] // Task 1C wires the capability boundary to real application launchers.
mod app_launch;
pub mod artifact_inspector;
#[allow(dead_code)] // Task 1C wires the capability boundary to real scenario owners.
mod candidate;
pub mod comparator;
pub mod evidence;
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
pub mod provenance;
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
}
