use std::{fs, path::PathBuf};

use clap::{Parser, Subcommand};
use xtask::comparator::{ScaffoldCreate, create_scaffold, verify_scaffold};
use xtask::fixtures::{generate_fixtures, verify_fixtures};
use xtask::gui::validate_transcript;
use xtask::metrics::{MetricAssertion, assert_metric_record};
use xtask::package::assert_file;
use xtask::runner::capture_verify_matrix;

#[derive(Parser)]
#[command(
    name = "xtask",
    version,
    about = "FeatherMark build and evidence driver"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Fixtures {
        #[command(subcommand)]
        command: FixtureCommand,
    },
    Comparator {
        #[command(subcommand)]
        command: ComparatorCommand,
    },
    Gui {
        #[command(subcommand)]
        command: GuiCommand,
    },
    Metrics {
        #[command(subcommand)]
        command: MetricsCommand,
    },
    Package {
        #[command(subcommand)]
        command: PackageCommand,
    },
    Runner {
        #[command(subcommand)]
        command: RunnerCommand,
    },
}

#[derive(Subcommand)]
enum FixtureCommand {
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
enum ComparatorCommand {
    Scaffold {
        #[command(subcommand)]
        command: ScaffoldCommand,
    },
}

#[derive(Subcommand)]
enum GuiCommand {
    ValidateTranscript {
        #[arg(long)]
        commands: PathBuf,
        #[arg(long)]
        events: PathBuf,
    },
}

#[derive(Subcommand)]
enum MetricsCommand {
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
enum PackageCommand {
    AssertFile {
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        sha256: String,
        #[arg(long)]
        maximum_bytes: u64,
    },
}

#[derive(Subcommand)]
enum RunnerCommand {
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
enum ScaffoldCommand {
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

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("xtask: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Fixtures { command } => match command {
            FixtureCommand::Generate { out } => {
                let manifest = generate_fixtures(&out)?;
                println!("generated {} fixtures", manifest.fixtures.len());
            }
            FixtureCommand::Verify { dir } => {
                let manifest = verify_fixtures(&dir)?;
                println!("verified {} fixtures", manifest.fixtures.len());
            }
        },
        Command::Comparator { command } => match command {
            ComparatorCommand::Scaffold { command } => match command {
                ScaffoldCommand::Create {
                    fixtures,
                    contracts,
                    xtask,
                    out,
                    lock,
                } => {
                    let created = create_scaffold(&ScaffoldCreate {
                        fixtures,
                        contracts,
                        xtask,
                        out,
                        lock,
                    })?;
                    println!("{}", created.commit_sha);
                }
                ScaffoldCommand::Verify { repo, lock } => {
                    let verified = verify_scaffold(&repo, &lock)?;
                    println!("{}", verified.commit_sha);
                }
            },
        },
        Command::Gui { command } => match command {
            GuiCommand::ValidateTranscript { commands, events } => {
                let summary = validate_transcript(&fs::read(commands)?, &fs::read(events)?)?;
                println!("commands={} events={}", summary.commands, summary.events);
            }
        },
        Command::Metrics { command } => match command {
            MetricsCommand::AssertRecord {
                input,
                minimum_samples,
                maximum_p95,
            } => {
                let result = assert_metric_record(
                    &fs::read(input)?,
                    &MetricAssertion {
                        minimum_samples,
                        maximum_p95,
                    },
                )?;
                println!("samples={} p95={}", result.samples, result.p95);
            }
        },
        Command::Package { command } => match command {
            PackageCommand::AssertFile {
                path,
                sha256,
                maximum_bytes,
            } => {
                let result = assert_file(&path, &sha256, maximum_bytes)?;
                println!("bytes={} sha256={}", result.bytes, result.sha256);
            }
        },
        Command::Runner { command } => match command {
            RunnerCommand::CaptureVerifyMatrix {
                runners,
                capture_dir,
                out,
            } => {
                let summary = capture_verify_matrix(&runners, &capture_dir, &out)?;
                println!("verified {} runners", summary.runners);
            }
        },
    }
    Ok(())
}
