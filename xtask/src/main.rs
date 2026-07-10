use std::fs;

use clap::Parser;
use xtask::cli::{
    Cli, Command, ComparatorCommand, FixtureCommand, GuiCommand, LocalPackageCommand,
    MetricsCommand, PackageCommand, RunnerCommand, ScaffoldCommand,
};
use xtask::comparator::{ScaffoldCreate, create_scaffold, verify_scaffold};
use xtask::fixtures::{generate_fixtures, verify_fixtures};
use xtask::gui::validate_transcript;
use xtask::local_package::{LinuxPackageRequest, MacPackageRequest};
use xtask::local_package_cli::{LocalPackageCliRequest, ProcessCommandExecutor, run_local_package};
use xtask::metrics::{MetricAssertion, assert_metric_record};
use xtask::package::assert_file;
use xtask::runner::capture_verify_matrix;

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
            PackageCommand::Local { command } => {
                let request = match command {
                    LocalPackageCommand::Macos {
                        candidate,
                        build_input_sha256,
                        source_commit,
                        output_root,
                        version,
                    } => LocalPackageCliRequest::Macos(MacPackageRequest {
                        candidate,
                        build_input_sha256,
                        source_commit,
                        output_root,
                        version,
                    }),
                    LocalPackageCommand::Linux {
                        candidate,
                        build_input_sha256,
                        source_commit,
                        output_root,
                        version,
                    } => LocalPackageCliRequest::Linux(LinuxPackageRequest {
                        candidate,
                        build_input_sha256,
                        source_commit,
                        output_root,
                        version,
                    }),
                };
                let manifests = run_local_package(request, &ProcessCommandExecutor)?;
                println!("{}", serde_json::to_string_pretty(&manifests)?);
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
