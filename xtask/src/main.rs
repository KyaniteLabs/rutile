use std::fs;
use std::time::Duration;

use clap::Parser;
use sha2::Digest;
use xtask::artifact_inspector::{ArtifactInspector, InspectionMode, PolicyPaths};
use xtask::cli::{
    ArtifactCommand, ArtifactInspectionMode, Cli, Command, ComparatorCommand, EvidenceCommand,
    FixtureCommand, GuiCommand, LinuxPackageFormats, LocalPackageCommand, MetricsCommand,
    PackageCommand, ProvenanceCommand, ReadinessCommand, ReleaseCommand, RunnerCommand,
    ScaffoldCommand,
};
use xtask::comparator::{ScaffoldCreate, create_scaffold, verify_scaffold};
use xtask::evidence_bind::{EvidenceBindRequest, bind as bind_evidence};
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
use xtask::package_smoke::{ProductionSmokeExecutor, SmokeRequest, run_smoke};
use xtask::readiness_keystone;
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
                    pinned_release_authority_pubkey: defaults
                        .pinned_release_authority_pubkey
                        .clone(),
                })?;
                let (mode, requires_authorization) = match mode {
                    ArtifactInspectionMode::Candidate => (InspectionMode::Candidate, false),
                    ArtifactInspectionMode::Package => (InspectionMode::Package, true),
                };
                let report = inspector.inspect(&artifact, mode, provenance.as_deref());
                println!("{}", serde_json::to_string_pretty(&report)?);
                if !report.accepted
                    || (requires_authorization
                        && !report.publication_authorized
                        && !report.preview_authorized)
                {
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
                let validation = if xtask::evidence::is_source_bound_kind(&schema) {
                    xtask::evidence::validate_readiness_with_source(&input, &schema)
                } else {
                    xtask::evidence::validate_kind(&input, &schema)
                };
                match validation {
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
            EvidenceCommand::Bind {
                plain_index,
                provenance,
                evidence_root,
                out,
            } => {
                let request = EvidenceBindRequest {
                    plain_index,
                    provenance,
                    evidence_root,
                    out,
                };
                let outcome = bind_evidence(&request)?;
                println!(
                    "evidence-bind out={} source_commit={} provenance_sha256={} records={} durable={}",
                    outcome.out.display(),
                    outcome.source_commit,
                    outcome.production_provenance_sha256,
                    outcome.record_count,
                    outcome.durable,
                );
                for warning in outcome.warnings {
                    eprintln!("xtask: evidence-bind warning: {warning}");
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
                        release_authority_key,
                        preview_signed_at,
                        preview_expires_at,
                    } => LocalPackageCliRequest::Macos(MacPackageRequest {
                        candidate,
                        build_input_sha256,
                        source_commit,
                        output_root,
                        version,
                        release_authority_key,
                        preview_signed_at,
                        preview_expires_at,
                    }),
                    LocalPackageCommand::Linux {
                        candidate,
                        build_input_sha256,
                        source_commit,
                        output_root,
                        version,
                        formats,
                    } => {
                        let request = LinuxPackageRequest {
                            candidate,
                            build_input_sha256,
                            source_commit,
                            output_root,
                            version,
                        };
                        match formats {
                            LinuxPackageFormats::All => LocalPackageCliRequest::Linux(request),
                            LinuxPackageFormats::Ubuntu => {
                                LocalPackageCliRequest::LinuxUbuntu(request)
                            }
                        }
                    }
                };
                let manifests = run_local_package(request, &ProcessCommandExecutor)?;
                println!("{}", serde_json::to_string_pretty(&manifests)?);
            }
            PackageCommand::SmokeRow {
                package,
                kind,
                package_sha256,
                source_commit,
                install_target,
                binary_path,
                expected_executable_sha256,
                evidence_dir,
                deadline_secs,
                term_grace_secs,
            } => {
                let evidence_dir_display = evidence_dir.display().to_string();
                let mut request = SmokeRequest::new(
                    package,
                    package_sha256,
                    kind.to_smoke_kind(),
                    source_commit,
                    install_target,
                    binary_path.clone(),
                    expected_executable_sha256,
                    evidence_dir,
                );
                if let Some(secs) = deadline_secs {
                    request = request.with_deadline(Duration::from_secs(secs));
                }
                if let Some(secs) = term_grace_secs {
                    request = request.with_term_grace(Duration::from_secs(secs));
                }
                let executor = ProductionSmokeExecutor::new(binary_path);
                let execution = run_smoke(request, &executor)?;
                println!(
                    "package-smoke-row passed={} evidence_dir={} receipt={}",
                    execution.receipt.passed,
                    evidence_dir_display,
                    execution.receipt_path.display(),
                );
                println!("{}", serde_json::to_string_pretty(&execution.receipt)?);
                if !execution.receipt.passed {
                    return Err(execution
                        .failure
                        .unwrap_or_else(|| {
                            "package smoke-row failed: install/open/uninstall did not pass clean"
                                .to_string()
                        })
                        .into());
                }
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
        Command::Provenance { command } => match command {
            ProvenanceCommand::Generate {
                candidate,
                repo_root,
                features,
                target_root,
                product,
                product_version,
                out,
            } => {
                let provenance = xtask::provenance::generate_with_reproducible_env(
                    &xtask::provenance::ProvenanceRequest {
                        candidate,
                        repo_root,
                        features,
                        target_root,
                    },
                    &product,
                    &product_version,
                )?;
                let json = provenance.canonical_json()?;
                fs::write(&out, &json)?;
                let sha256 = provenance.provenance_sha256()?;
                println!("provenance {} sha256={}", out.display(), sha256);
            }
        },
        Command::Release { command } => match command {
            ReleaseCommand::Keygen { public_out } => {
                let (secret_hex, public_hex, fingerprint) = xtask::release_authority::keygen()?;
                if let Some(parent) = public_out.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&public_out, &public_hex)?;
                eprintln!(
                    "release-authority public key -> {} (fingerprint {fingerprint})",
                    public_out.display()
                );
                eprintln!(
                    "release-authority SECRET hex below — install privately at 0600, NEVER commit:"
                );
                println!("{secret_hex}");
            }
            ReleaseCommand::PreviewAuthorize {
                artifact,
                provenance,
                product,
                version_label,
                signed_at,
                expires_at,
                key_path,
                out,
            } => {
                let signing = xtask::release_authority::read_signing_key(&key_path)?;
                let artifact_bytes = fs::read(&artifact)?;
                let artifact_sha256 = hex::encode(sha2::Sha256::digest(&artifact_bytes));
                let provenance_bytes = fs::read(&provenance)?;
                let provenance_value: serde_json::Value =
                    serde_json::from_slice(&provenance_bytes)?;
                if provenance_value
                    .get("schema")
                    .and_then(|value| value.as_str())
                    != Some("rutile.production-provenance.v1")
                {
                    return Err(
                        "provenance file is not a rutile.production-provenance.v1 record".into(),
                    );
                }
                let provenance_sha256 = hex::encode(sha2::Sha256::digest(&provenance_bytes));
                let statement = xtask::release_authority::PreviewAuthorizationStatement {
                    artifact_sha256,
                    provenance_sha256,
                    tier: xtask::release_authority::PREVIEW_TIER.to_string(),
                    product,
                    version_label,
                    signed_at,
                    expires_at,
                };
                let record = xtask::release_authority::sign(&statement, &signing)?;
                fs::write(&out, serde_json::to_string_pretty(&record)?)?;
                println!(
                    "preview-authorization {} artifact={} fingerprint={}",
                    out.display(),
                    record.artifact_sha256,
                    record.release_authority_key_fingerprint
                );
            }
        },
        Command::Readiness { command } => match command {
            ReadinessCommand::Verify { input, runner_lock } => {
                let receipt = readiness_keystone::verify_only(&input, &runner_lock)?;
                println!(
                    "readiness-verify ready={} source_commit={} source_tree={} runner_lock_sha256={} verifier_fingerprint={}",
                    receipt.ready,
                    receipt.source.commit,
                    receipt.source.tree,
                    hex::encode(receipt.runner_lock_sha256),
                    receipt.verifier_key_fingerprint,
                );
                if !receipt.ready {
                    return Err("verified but not ready; actionable blockers remain".into());
                }
            }
            ReadinessCommand::Publish {
                input,
                runner_lock,
                out,
            } => {
                let receipt = readiness_keystone::verify_and_publish(&input, &runner_lock, &out)?;
                let out_display = receipt
                    .out_path
                    .as_deref()
                    .unwrap_or(&out)
                    .display()
                    .to_string();
                println!(
                    "readiness-publish ready={} source_commit={} source_tree={} runner_lock_sha256={} verifier_fingerprint={} out={}",
                    receipt.ready,
                    receipt.source.commit,
                    receipt.source.tree,
                    hex::encode(receipt.runner_lock_sha256),
                    receipt.verifier_key_fingerprint,
                    out_display,
                );
            }
        },
    }
    Ok(())
}
