//! Private, sealed launcher transport. There is no runtime selector.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::RunnerError;
use super::config::RunnerDispatchConfig;
use super::encoding::encode_probe_request;
use super::protocol::{ProbeRequestV1, SignedRunnerProbeV1};

const MAGIC: &[u8] = b"FMRP\0v1\0";
const MAX_RECEIPT_BYTES: usize = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

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
        let address = row
            .endpoint
            .to_socket_addrs()
            .map_err(|error| transport(format!("{} address: {error}", row.runner_id)))?
            .next()
            .ok_or_else(|| transport(format!("{} has no resolved address", row.runner_id)))?;
        let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)
            .map_err(|error| transport(format!("{} connect: {error}", row.runner_id)))?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        let request = encode_probe_request(request);
        let request_len: u32 = request
            .len()
            .try_into()
            .map_err(|_| transport("request exceeds u32 framing".into()))?;
        stream.write_all(MAGIC)?;
        stream.write_all(&request_len.to_be_bytes())?;
        stream.write_all(&request)?;
        stream.flush()?;

        let mut fingerprint = [0; 32];
        stream.read_exact(&mut fingerprint)?;
        if fingerprint != row.transport_fingerprint {
            return Err(transport(format!(
                "{} transport fingerprint mismatch",
                row.runner_id
            )));
        }
        let mut length = [0; 4];
        stream.read_exact(&mut length)?;
        let length = u32::from_be_bytes(length) as usize;
        if length == 0 || length > MAX_RECEIPT_BYTES {
            return Err(transport(format!(
                "{} receipt length is invalid",
                row.runner_id
            )));
        }
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes)?;
        serde_json::from_slice(&bytes).map_err(RunnerError::from)
    }
}

fn transport(message: String) -> RunnerError {
    RunnerError::Transport(message)
}
