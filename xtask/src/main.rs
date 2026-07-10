use std::path::PathBuf;

use clap::{Parser, Subcommand};
use xtask::comparator::{ScaffoldCreate, create_scaffold, verify_scaffold};
use xtask::fixtures::{generate_fixtures, verify_fixtures};

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
    }
    Ok(())
}
