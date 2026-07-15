use std::fs;

use clap::Parser;
use xtask::artifact_inspector::{ArtifactInspector, InspectionMode, PolicyPaths};
use xtask::cli::{
    ArtifactCommand, ArtifactInspectionMode, Cli, Command, ComparatorCommand, EvidenceCommand,
    FixtureCommand, GuiCommand, LocalPackageCommand, MetricsCommand, PackageCommand, RunnerCommand,
    ScaffoldCommand,
};
use xtask::comparator::{ScaffoldCreate, create_scaffold, verify_scaffold};
use xtask::fixtures::{generate_fixtures, verify_fixtures};
use xtask::gui::validate_transcript;
#[cfg(unix)]
use xtask::linux_gate::{LinuxGateRequest, run_gate as run_linux_gate};
use xtask::local_package::{LinuxPackageRequest, MacPackageRequest};
use xtask::local_package_cli::{LocalPackageCliRequest, ProcessCommandExecutor, run_local_package};
use xtask::metrics::{MetricAssertion, assert_metric_record};
#[cfg(unix)]
use xtask::native_smoke::{NativeSmokeGateRequest, run_gate};
use xtask::package::assert_file;
use xtask::release_preflight::{load_and_validate, publish_create_only};
use xtask::reproducible_build::{ReproducibleBuildRequest, run as run_reproducible_build};
use xtask::runner::capture_verify_matrix;

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("xtask: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Artifact { command } => match command {
            ArtifactCommand::Inspect {
                artifact,
                quarantine,
                policy,
                mode,
                provenance,
            } => {
                let defaults = PolicyPaths::repository_defaults();
                let inspector = ArtifactInspector::load(&PolicyPaths {
                    quarantine: quarantine.unwrap_or(defaults.quarantine),
                    policy: policy.unwrap_or(defaults.policy),
                })?;
                let (mode, requires_authorization) = match mode {
                    ArtifactInspectionMode::Candidate => (InspectionMode::Candidate, false),
                    ArtifactInspectionMode::Package => (InspectionMode::Package, true),
                };
                let report = inspector.inspect(&artifact, mode, provenance.as_deref());
                println!("{}", serde_json::to_string_pretty(&report)?);
                if !report.accepted || (requires_authorization && !report.publication_authorized) {
                    return Err("artifact inspection rejected candidate".into());
                }
            }
        },
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
        Command::Evidence { command } => match command {
            EvidenceCommand::Validate { input, schema } => {
                match xtask::evidence::validate_kind(&input, &schema) {
                    Ok(()) => {
                        println!(
                            "PASS: {} validates against rutile.{}.v1",
                            input.display(),
                            schema
                        );
                    }
                    Err(error) => {
                        eprintln!("FAIL: {error}");
                        return Err(format!(
                            "validation failed: {} does not validate against rutile.{}.v1",
                            input.display(),
                            schema
                        )
                        .into());
                    }
                }
            }
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
        Command::ReleasePreflight { input, out } => {
            let report = load_and_validate(&input)?;
            let publication = publish_create_only(&out, &report)?;
            println!(
                "release-prerequisite-preflight run_id={} ready={} durable={}",
                report.run_id, report.result.ready, publication.durable
            );
            for warning in publication.warnings {
                eprintln!("xtask: release-preflight warning: {warning}");
            }
            if !report.result.ready {
                return Err("release prerequisite preflight blocked".into());
            }
        }
        #[cfg(unix)]
        Command::NativeSmoke {
            binary,
            profile,
            repeat,
            evidence_dir,
        } => {
            let execution = run_gate(NativeSmokeGateRequest {
                binary,
                profile,
                repeat: repeat.map(|value| value.get()),
                evidence_dir,
            })?;
            println!(
                "native-smoke-gate-result={} runs={} passed={}",
                execution.report_path.display(),
                execution.repeats,
                execution.passed,
            );
            if !execution.passed {
                return Err(std::io::Error::other(format!(
                    "native smoke gate failed; report retained at {}: {}",
                    execution.report_path.display(),
                    execution
                        .failure
                        .as_deref()
                        .unwrap_or("required run did not pass")
                ))
                .into());
            }
        }
        #[cfg(unix)]
        Command::LinuxGate {
            binary,
            profile,
            cycles,
            exit_code,
            started_ms,
            ended_ms,
            stdout_log,
            stderr_log,
            evidence_dir,
        } => {
            let execution = run_linux_gate(LinuxGateRequest {
                binary,
                profile,
                cycles: cycles.get(),
                exit_code,
                started_unix_ms: started_ms,
                ended_unix_ms: ended_ms,
                stdout_log,
                stderr_log,
                evidence_dir,
            })?;
            println!(
                "linux-gate-result={} passed={}",
                execution.report_path.display(),
                execution.passed,
            );
            if !execution.passed {
                return Err(std::io::Error::other(format!(
                    "linux native gate failed; report retained at {}: {}",
                    execution.report_path.display(),
                    execution
                        .failure
                        .as_deref()
                        .unwrap_or("required run did not pass")
                ))
                .into());
            }
        }
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
        Command::ReproducibleBuild {
            package,
            bin,
            features,
        } => {
            let result = run_reproducible_build(ReproducibleBuildRequest {
                package,
                bin,
                features,
            })?;
            println!(
                "reproducible-build binary={} source_date_epoch={}",
                result.binary_path.display(),
                result.source_date_epoch,
            );
        }
    }
    Ok(())
}
