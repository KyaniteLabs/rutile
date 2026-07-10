//! Closed owner for non-application tools. Candidate/application paths are not accepted here.

use std::path::Path;
use std::process::{Command, Output};

pub(crate) fn git(
    repo: &Path,
    args: &[&str],
    environment: &[(&str, &str)],
) -> std::io::Result<Output> {
    let mut command = Command::new("git");
    command.args(args).current_dir(repo);
    for (name, value) in environment {
        command.env(name, value);
    }
    #[allow(clippy::disallowed_methods)]
    command.output()
}
