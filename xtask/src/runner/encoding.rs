use super::RunnerError;
use super::protocol::{
    NativeProbeChallengeV1, NativeProbeReportV1, ProbePayloadV1, ProbePurpose, ProbeRequestV1,
    RunnerIdentityV1,
};

pub(crate) const MAX_NATIVE_REPORT_BYTES: usize = 64 * 1024;

pub(crate) fn encode_native_challenge(challenge: &NativeProbeChallengeV1) -> Vec<u8> {
    let mut out = Vec::with_capacity(36);
    array(&mut out, 2);
    uint(&mut out, 1);
    bytes(&mut out, &challenge.challenge);
    out
}

pub(crate) fn decode_native_challenge(bytes: &[u8]) -> Result<NativeProbeChallengeV1, RunnerError> {
    let mut reader = Reader { bytes, offset: 0 };
    reader.exact_array(2)?;
    if reader.uint()? != 1 {
        return Err(protocol("unsupported native challenge schema"));
    }
    let challenge = NativeProbeChallengeV1 {
        challenge: reader.hash()?,
    };
    if challenge.challenge == [0; 32]
        || reader.offset != bytes.len()
        || encode_native_challenge(&challenge) != bytes
    {
        return Err(protocol("native challenge is not canonical"));
    }
    Ok(challenge)
}

pub(crate) fn encode_native_report(report: &NativeProbeReportV1) -> Result<Vec<u8>, RunnerError> {
    let mut out = Vec::new();
    array(&mut out, 9);
    uint(&mut out, 1);
    bytes(&mut out, &report.challenge);
    encode_identity_into(&mut out, &report.identity);
    bytes(&mut out, &report.boot_id_sha256);
    bytes(&mut out, &report.graphical_session_id_sha256);
    text(&mut out, &report.snapshot_id);
    text(&mut out, &report.snapshot_provider);
    bytes(&mut out, &report.snapshot_image_sha256);
    uint(&mut out, report.captured_at_unix_ms);
    if out.len() > MAX_NATIVE_REPORT_BYTES {
        return Err(protocol("native report exceeds fixed bound"));
    }
    Ok(out)
}

pub(crate) fn decode_native_report(bytes: &[u8]) -> Result<NativeProbeReportV1, RunnerError> {
    if bytes.len() > MAX_NATIVE_REPORT_BYTES {
        return Err(protocol("native report exceeds fixed bound"));
    }
    let mut reader = Reader { bytes, offset: 0 };
    reader.exact_array(9)?;
    if reader.uint()? != 1 {
        return Err(protocol("unsupported native report schema"));
    }
    let report = NativeProbeReportV1 {
        challenge: reader.hash()?,
        identity: decode_identity_from(&mut reader)?,
        boot_id_sha256: reader.hash()?,
        graphical_session_id_sha256: reader.hash()?,
        snapshot_id: reader.text()?,
        snapshot_provider: reader.text()?,
        snapshot_image_sha256: reader.hash()?,
        captured_at_unix_ms: reader.uint()?,
    };
    if report.challenge == [0; 32]
        || report.boot_id_sha256 == [0; 32]
        || report.graphical_session_id_sha256 == [0; 32]
        || report.snapshot_id.is_empty()
        || report.snapshot_provider.is_empty()
        || report.snapshot_image_sha256 == [0; 32]
        || report.captured_at_unix_ms == 0
        || reader.offset != bytes.len()
        || encode_native_report(&report)? != bytes
    {
        return Err(protocol("native report is not canonical and complete"));
    }
    Ok(report)
}

pub(crate) fn encode_probe_request(request: &ProbeRequestV1) -> Vec<u8> {
    let mut out = Vec::new();
    array(&mut out, 14);
    uint(&mut out, 1);
    uint(&mut out, request.purpose as u64);
    bytes(&mut out, &request.run_id);
    text(&mut out, &request.runner_id);
    bytes(&mut out, &request.challenge);
    uint(&mut out, request.issued_at_unix_ms);
    uint(&mut out, request.not_after_unix_ms);
    text(&mut out, &request.expected_snapshot_id);
    text(&mut out, &request.expected_snapshot_provider);
    bytes(&mut out, &request.expected_image_sha256);
    bytes(&mut out, &request.expected_probe_sha256);
    optional_hash(&mut out, request.enrollment_commitment);
    optional_hash(&mut out, request.final_lock_sha256);
    optional_hash(&mut out, request.candidate_manifest_sha256);
    out
}

#[allow(dead_code)] // Used by the root launcher framing implementation and deterministic tests.
pub(crate) fn decode_probe_request(bytes: &[u8]) -> Result<ProbeRequestV1, RunnerError> {
    let mut reader = Reader { bytes, offset: 0 };
    reader.exact_array(14)?;
    if reader.uint()? != 1 {
        return Err(protocol("unsupported request schema"));
    }
    let purpose = ProbePurpose::try_from(reader.uint()?)
        .map_err(|_| protocol("unsupported probe purpose"))?;
    let request = ProbeRequestV1 {
        purpose,
        run_id: reader.hash()?,
        runner_id: reader.text()?,
        challenge: reader.hash()?,
        issued_at_unix_ms: reader.uint()?,
        not_after_unix_ms: reader.uint()?,
        expected_snapshot_id: reader.text()?,
        expected_snapshot_provider: reader.text()?,
        expected_image_sha256: reader.hash()?,
        expected_probe_sha256: reader.hash()?,
        enrollment_commitment: reader.optional_hash()?,
        final_lock_sha256: reader.optional_hash()?,
        candidate_manifest_sha256: reader.optional_hash()?,
    };
    if reader.offset != bytes.len() || encode_probe_request(&request) != bytes {
        return Err(protocol("request CBOR is not canonical"));
    }
    Ok(request)
}

pub(crate) fn encode_probe_payload(payload: &ProbePayloadV1) -> Vec<u8> {
    let mut out = Vec::new();
    array(&mut out, 9);
    uint(&mut out, 1);
    out.extend_from_slice(&encode_probe_request(&payload.request));
    encode_identity_into(&mut out, &payload.identity);
    bytes(&mut out, &payload.boot_id_sha256);
    bytes(&mut out, &payload.graphical_session_id_sha256);
    uint(&mut out, payload.captured_at_unix_ms);
    uint(&mut out, payload.elapsed_ms);
    uint(&mut out, payload.launcher_protocol_version as u64);
    bytes(&mut out, &payload.measured_probe_sha256);
    out
}

pub(crate) fn decode_probe_payload(bytes: &[u8]) -> Result<ProbePayloadV1, RunnerError> {
    let mut reader = Reader { bytes, offset: 0 };
    reader.exact_array(9)?;
    if reader.uint()? != 1 {
        return Err(protocol("unsupported payload schema"));
    }
    let request = decode_request_from(&mut reader)?;
    let identity = decode_identity_from(&mut reader)?;
    let payload = ProbePayloadV1 {
        request,
        identity,
        boot_id_sha256: reader.hash()?,
        graphical_session_id_sha256: reader.hash()?,
        captured_at_unix_ms: reader.uint()?,
        elapsed_ms: reader.uint()?,
        launcher_protocol_version: reader
            .uint()?
            .try_into()
            .map_err(|_| protocol("launcher version overflow"))?,
        measured_probe_sha256: reader.hash()?,
    };
    if reader.offset != bytes.len() || encode_probe_payload(&payload) != bytes {
        return Err(protocol("payload CBOR is not canonical"));
    }
    Ok(payload)
}

pub(crate) fn encode_identity(identity: &RunnerIdentityV1) -> Vec<u8> {
    let mut out = Vec::new();
    encode_identity_into(&mut out, identity);
    out
}

fn encode_identity_into(out: &mut Vec<u8>, identity: &RunnerIdentityV1) {
    array(out, 25);
    uint(out, 1);
    text(out, &identity.runner_id);
    bytes(out, &identity.machine_id_sha256);
    text(out, &identity.hardware_model);
    text(out, &identity.cpu_model);
    uint(out, identity.cpu_cores as u64);
    uint(out, identity.ram_bytes);
    text(out, &identity.arch);
    text(out, &identity.os_product);
    text(out, &identity.os_version);
    text(out, &identity.os_build);
    text(out, &identity.os_image);
    text(out, &identity.kernel);
    text(out, &identity.display_session);
    optional_text(out, identity.display_socket.as_deref());
    uint(out, identity.monitor_width_px as u64);
    uint(out, identity.monitor_height_px as u64);
    uint(out, identity.monitor_scale_milli as u64);
    uint(out, identity.monitor_refresh_millihz as u64);
    optional_text(out, identity.gtk_version.as_deref());
    optional_text(out, identity.webkitgtk_version.as_deref());
    optional_text(out, identity.wkwebview_version.as_deref());
    uint(out, u64::from(identity.virtualized));
    optional_hash(out, identity.virtualization_image_sha256);
    text(out, &identity.snapshot_provider);
}

fn decode_identity_from(reader: &mut Reader<'_>) -> Result<RunnerIdentityV1, RunnerError> {
    reader.exact_array(25)?;
    if reader.uint()? != 1 {
        return Err(protocol("unsupported identity schema"));
    }
    Ok(RunnerIdentityV1 {
        runner_id: reader.text()?,
        machine_id_sha256: reader.hash()?,
        hardware_model: reader.text()?,
        cpu_model: reader.text()?,
        cpu_cores: reader
            .uint()?
            .try_into()
            .map_err(|_| protocol("core count overflow"))?,
        ram_bytes: reader.uint()?,
        arch: reader.text()?,
        os_product: reader.text()?,
        os_version: reader.text()?,
        os_build: reader.text()?,
        os_image: reader.text()?,
        kernel: reader.text()?,
        display_session: reader.text()?,
        display_socket: reader.optional_text()?,
        monitor_width_px: reader
            .uint()?
            .try_into()
            .map_err(|_| protocol("width overflow"))?,
        monitor_height_px: reader
            .uint()?
            .try_into()
            .map_err(|_| protocol("height overflow"))?,
        monitor_scale_milli: reader
            .uint()?
            .try_into()
            .map_err(|_| protocol("scale overflow"))?,
        monitor_refresh_millihz: reader
            .uint()?
            .try_into()
            .map_err(|_| protocol("refresh overflow"))?,
        gtk_version: reader.optional_text()?,
        webkitgtk_version: reader.optional_text()?,
        wkwebview_version: reader.optional_text()?,
        virtualized: match reader.uint()? {
            0 => false,
            1 => true,
            _ => return Err(protocol("invalid boolean")),
        },
        virtualization_image_sha256: reader.optional_hash()?,
        snapshot_provider: reader.text()?,
    })
}

fn decode_request_from(reader: &mut Reader<'_>) -> Result<ProbeRequestV1, RunnerError> {
    reader.exact_array(14)?;
    if reader.uint()? != 1 {
        return Err(protocol("unsupported request schema"));
    }
    Ok(ProbeRequestV1 {
        purpose: ProbePurpose::try_from(reader.uint()?)
            .map_err(|_| protocol("unsupported probe purpose"))?,
        run_id: reader.hash()?,
        runner_id: reader.text()?,
        challenge: reader.hash()?,
        issued_at_unix_ms: reader.uint()?,
        not_after_unix_ms: reader.uint()?,
        expected_snapshot_id: reader.text()?,
        expected_snapshot_provider: reader.text()?,
        expected_image_sha256: reader.hash()?,
        expected_probe_sha256: reader.hash()?,
        enrollment_commitment: reader.optional_hash()?,
        final_lock_sha256: reader.optional_hash()?,
        candidate_manifest_sha256: reader.optional_hash()?,
    })
}

pub(crate) fn array(out: &mut Vec<u8>, len: usize) {
    major(out, 4, len as u64);
}

pub(crate) fn uint(out: &mut Vec<u8>, value: u64) {
    major(out, 0, value);
}

pub(crate) fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    major(out, 2, value.len() as u64);
    out.extend_from_slice(value);
}

pub(crate) fn text(out: &mut Vec<u8>, value: &str) {
    major(out, 3, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

pub(crate) fn optional_hash(out: &mut Vec<u8>, value: Option<[u8; 32]>) {
    match value {
        None => {
            array(out, 1);
            uint(out, 0);
        }
        Some(hash) => {
            array(out, 2);
            uint(out, 1);
            bytes(out, &hash);
        }
    }
}

pub(crate) fn optional_text(out: &mut Vec<u8>, value: Option<&str>) {
    match value {
        None => {
            array(out, 1);
            uint(out, 0);
        }
        Some(value) => {
            array(out, 2);
            uint(out, 1);
            text(out, value);
        }
    }
}

fn major(out: &mut Vec<u8>, major: u8, value: u64) {
    let prefix = major << 5;
    match value {
        0..=23 => out.push(prefix | value as u8),
        24..=0xff => out.extend_from_slice(&[prefix | 24, value as u8]),
        0x100..=0xffff => {
            out.push(prefix | 25);
            out.extend_from_slice(&(value as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(prefix | 26);
            out.extend_from_slice(&(value as u32).to_be_bytes());
        }
        _ => {
            out.push(prefix | 27);
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Reader<'_> {
    fn exact_array(&mut self, expected: u64) -> Result<(), RunnerError> {
        let (major, len) = self.head()?;
        if major == 4 && len == expected {
            Ok(())
        } else {
            Err(protocol("unexpected CBOR array"))
        }
    }

    fn uint(&mut self) -> Result<u64, RunnerError> {
        let (major, value) = self.head()?;
        if major == 0 {
            Ok(value)
        } else {
            Err(protocol("expected unsigned integer"))
        }
    }

    fn hash(&mut self) -> Result<[u8; 32], RunnerError> {
        let value = self.byte_string()?;
        value
            .try_into()
            .map_err(|_| protocol("expected 32-byte byte string"))
    }

    fn text(&mut self) -> Result<String, RunnerError> {
        let (major, len) = self.head()?;
        if major != 3 {
            return Err(protocol("expected text"));
        }
        let value = self.take(len as usize)?;
        std::str::from_utf8(value)
            .map(str::to_owned)
            .map_err(|_| protocol("text is not UTF-8"))
    }

    fn optional_hash(&mut self) -> Result<Option<[u8; 32]>, RunnerError> {
        let (major, len) = self.head()?;
        if major != 4 {
            return Err(protocol("expected optional array"));
        }
        match (len, self.uint()?) {
            (1, 0) => Ok(None),
            (2, 1) => self.hash().map(Some),
            _ => Err(protocol("invalid optional discriminant")),
        }
    }

    fn optional_text(&mut self) -> Result<Option<String>, RunnerError> {
        let (major, len) = self.head()?;
        if major != 4 {
            return Err(protocol("expected optional array"));
        }
        match (len, self.uint()?) {
            (1, 0) => Ok(None),
            (2, 1) => self.text().map(Some),
            _ => Err(protocol("invalid optional discriminant")),
        }
    }

    fn byte_string(&mut self) -> Result<&[u8], RunnerError> {
        let (major, len) = self.head()?;
        if major != 2 {
            return Err(protocol("expected byte string"));
        }
        self.take(len as usize)
    }

    fn head(&mut self) -> Result<(u8, u64), RunnerError> {
        let initial = *self.take(1)?.first().expect("one byte");
        let major = initial >> 5;
        let info = initial & 0x1f;
        let value = match info {
            0..=23 => info as u64,
            24 => {
                let value = self.take(1)?[0] as u64;
                if value < 24 {
                    return Err(protocol("non-shortest CBOR integer"));
                }
                value
            }
            25 => {
                let value = u16::from_be_bytes(self.take(2)?.try_into().expect("two bytes")) as u64;
                if value <= 0xff {
                    return Err(protocol("non-shortest CBOR integer"));
                }
                value
            }
            26 => {
                let value =
                    u32::from_be_bytes(self.take(4)?.try_into().expect("four bytes")) as u64;
                if value <= 0xffff {
                    return Err(protocol("non-shortest CBOR integer"));
                }
                value
            }
            27 => {
                let value = u64::from_be_bytes(self.take(8)?.try_into().expect("eight bytes"));
                if value <= 0xffff_ffff {
                    return Err(protocol("non-shortest CBOR integer"));
                }
                value
            }
            _ => return Err(protocol("indefinite/reserved CBOR is forbidden")),
        };
        Ok((major, value))
    }

    fn take(&mut self, len: usize) -> Result<&[u8], RunnerError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| protocol("CBOR length overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| protocol("truncated CBOR"))?;
        self.offset = end;
        Ok(value)
    }
}

fn protocol(message: &str) -> RunnerError {
    RunnerError::Protocol(message.into())
}
