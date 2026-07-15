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
