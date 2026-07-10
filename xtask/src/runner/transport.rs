//! Private, sealed authenticated launcher transport. There is no runtime selector.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::process::{Command, Stdio};

use super::RunnerError;
use super::config::RunnerDispatchConfig;
use super::encoding::encode_probe_request;
use super::protocol::{ProbeRequestV1, SignedRunnerProbeV1};

const MAGIC: &[u8] = b"FMRP\0v1\0";
const MAX_RECEIPT_BYTES: usize = 256 * 1024;

mod sealed {
    pub trait Sealed {}
}

#[cfg(test)]
impl sealed::Sealed for super::test_support::FakeLauncherTransport {}

#[cfg(test)]
impl LauncherTransport for super::test_support::FakeLauncherTransport {
    fn exchange(
        &self,
        _row: &RunnerDispatchConfig,
        request: &ProbeRequestV1,
    ) -> Result<SignedRunnerProbeV1, RunnerError> {
        self.exchange_impl(request)
    }
}

pub(crate) trait LauncherTransport: sealed::Sealed {
    fn exchange(
        &self,
        row: &RunnerDispatchConfig,
        request: &ProbeRequestV1,
    ) -> Result<SignedRunnerProbeV1, RunnerError>;
}

pub(crate) struct ProductionLauncherTransport;

impl sealed::Sealed for ProductionLauncherTransport {}

impl LauncherTransport for ProductionLauncherTransport {
    fn exchange(
        &self,
        row: &RunnerDispatchConfig,
        request: &ProbeRequestV1,
    ) -> Result<SignedRunnerProbeV1, RunnerError> {
        let (host, port) = endpoint_parts(row.endpoint)?;
        let known_hosts = ssh_known_hosts(row.endpoint, row.ssh_host_ed25519_public_key)?;
        let mut pipe_fds = [0_i32; 2];
        if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: pipe returned two newly owned descriptors.
        let known_reader = unsafe { File::from_raw_fd(pipe_fds[0]) };
        let mut known_writer = unsafe { File::from_raw_fd(pipe_fds[1]) };
        known_writer.write_all(known_hosts.as_bytes())?;
        drop(known_writer);
        let known_path = format!("/dev/fd/{}", known_reader.as_raw_fd());

        #[allow(clippy::disallowed_methods)]
        // This is the audited authenticated runner transport owner.
        let mut child = Command::new("/usr/bin/ssh")
            .args([
                "-F",
                "/dev/null",
                "-T",
                "-oBatchMode=yes",
                "-oIdentitiesOnly=yes",
                "-oStrictHostKeyChecking=yes",
                &format!("-oUserKnownHostsFile={known_path}"),
                "-oGlobalKnownHostsFile=/dev/null",
                "-oHostKeyAlgorithms=ssh-ed25519",
                "-oConnectTimeout=30",
                "-oServerAliveInterval=5",
                "-oServerAliveCountMax=6",
                "-p",
                port,
                &format!("feathermark-runner@{host}"),
                "feathermark-runner-launcher",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| transport(format!("{} authenticated ssh: {error}", row.runner_id)))?;
        // Keep the pinned known-host descriptor open until ssh has consumed it.
        let request = encode_probe_request(request);
        let request_len: u32 = request
            .len()
            .try_into()
            .map_err(|_| transport("request exceeds u32 framing".into()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| transport("ssh stdin was not piped".into()))?;
        stdin.write_all(MAGIC)?;
        stdin.write_all(&request_len.to_be_bytes())?;
        stdin.write_all(&request)?;
        drop(stdin);
        let output = child.wait_with_output()?;
        drop(known_reader);
        if !output.status.success() {
            return Err(transport(format!(
                "{} authenticated ssh exited {}: {}",
                row.runner_id,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        if !output.stderr.is_empty() {
            return Err(transport(format!(
                "{} authenticated ssh produced stderr",
                row.runner_id
            )));
        }
        let mut bytes = output.stdout.as_slice();
        let mut length = [0; 4];
        bytes.read_exact(&mut length)?;
        let length = u32::from_be_bytes(length) as usize;
        if length == 0 || length > MAX_RECEIPT_BYTES || bytes.len() != length {
            return Err(transport(format!(
                "{} receipt length is invalid",
                row.runner_id
            )));
        }
        serde_json::from_slice(bytes).map_err(RunnerError::from)
    }
}

fn endpoint_parts(endpoint: &str) -> Result<(&str, &str), RunnerError> {
    let (host, port) = endpoint
        .rsplit_once(':')
        .ok_or_else(|| transport("ssh endpoint has no port".into()))?;
    if host.is_empty() || port.parse::<u16>().is_err() {
        return Err(transport("ssh endpoint is malformed".into()));
    }
    Ok((host, port))
}

fn ssh_known_hosts(endpoint: &str, public_key: [u8; 32]) -> Result<String, RunnerError> {
    if public_key == [0; 32] {
        return Err(transport("ssh host public key is all zero".into()));
    }
    let (host, port) = endpoint_parts(endpoint)?;
    let mut blob = Vec::with_capacity(4 + 11 + 4 + public_key.len());
    blob.extend_from_slice(&11_u32.to_be_bytes());
    blob.extend_from_slice(b"ssh-ed25519");
    blob.extend_from_slice(&32_u32.to_be_bytes());
    blob.extend_from_slice(&public_key);
    Ok(format!("[{host}]:{port} ssh-ed25519 {}\n", base64(&blob)))
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[((bits >> 18) & 63) as usize] as char);
        out.push(TABLE[((bits >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((bits >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(bits & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn transport(message: String) -> RunnerError {
    RunnerError::Transport(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_transport_pin_is_an_independent_openssh_host_key() {
        let known_hosts = ssh_known_hosts("runner.internal:2222", [7; 32]).unwrap();
        assert!(known_hosts.starts_with("[runner.internal]:2222 ssh-ed25519 "));
        assert!(!known_hosts.contains(&hex::encode([7; 32])));
        assert!(ssh_known_hosts("runner.internal:2222", [0; 32]).is_err());
    }

    #[test]
    fn production_transport_has_no_peer_echo_acceptance_path() {
        let source = include_str!("transport.rs");
        let production = source.rsplit_once("#[cfg(test)]\nmod tests").unwrap().0;
        assert!(!production.contains("TcpStream"));
        assert!(!production.contains("read_exact(&mut fingerprint)"));
        assert!(production.contains("StrictHostKeyChecking=yes"));
        assert!(production.contains("UserKnownHostsFile={known_path}"));
    }
}
