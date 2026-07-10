//! Sole owner of application/candidate process creation.

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::runner::current::{VerifiedRunner, recheck_current_session};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppLaunchError {
    #[error("application launch capability expired or does not match the closed spec")]
    Capability,
    #[error("application executable does not match the verified manifest")]
    Executable,
    #[error("application launch I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

pub(crate) struct AppLaunchSpec {
    executable: PathBuf,
    runner_id: String,
    manifest_sha256: [u8; 32],
    snapshot_id: String,
    executable_sha256: [u8; 32],
    arguments: Vec<String>,
}

pub(crate) struct AuthorizedAppLaunch {
    spec: AppLaunchSpec,
    capability: VerifiedRunner,
}

pub(crate) fn authorize(
    capability: VerifiedRunner,
    spec: AppLaunchSpec,
) -> Result<AuthorizedAppLaunch, AppLaunchError> {
    if Instant::now() > capability.expires_at
        || capability.runner_id != spec.runner_id
        || capability.manifest_sha256 != spec.manifest_sha256
        || capability.snapshot_id != spec.snapshot_id
        || capability.executable_sha256 != spec.executable_sha256
    {
        return Err(AppLaunchError::Capability);
    }
    Ok(AuthorizedAppLaunch { spec, capability })
}

pub(crate) fn spawn(authorized: AuthorizedAppLaunch) -> Result<Child, AppLaunchError> {
    if Instant::now() > authorized.capability.expires_at {
        return Err(AppLaunchError::Capability);
    }
    recheck_current_session(&authorized.capability).map_err(|_| AppLaunchError::Capability)?;
    if Instant::now() > authorized.capability.expires_at {
        return Err(AppLaunchError::Capability);
    }
    let bytes = fs::read(&authorized.spec.executable)?;
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    if digest != authorized.spec.executable_sha256 {
        return Err(AppLaunchError::Executable);
    }
    let mut command = Command::new(&authorized.spec.executable);
    command.args(&authorized.spec.arguments);
    #[allow(clippy::disallowed_methods)]
    Ok(command.spawn()?)
}
