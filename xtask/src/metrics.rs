use feathermark_protocol::{MetricRecordV1, ProtocolError, decode_metric_record};
use thiserror::Error;

pub const NEAREST_RANK_PERCENTILE: f64 = 0.95;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricAssertion {
    pub minimum_samples: usize,
    pub maximum_p95: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricAssertionResult {
    pub p95: u64,
    pub samples: usize,
}

#[derive(Debug, Error)]
pub enum MetricDriverError {
    #[error("metric protocol failed: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("metric has {actual} samples; at least {minimum} are required")]
    MissingSamples { actual: usize, minimum: usize },
    #[error("metric p95 {actual} exceeds {maximum}")]
    Threshold { actual: u64, maximum: u64 },
}

pub fn assert_metric_record(
    bytes: &[u8],
    assertion: &MetricAssertion,
) -> Result<MetricAssertionResult, MetricDriverError> {
    let record = decode_metric_record(bytes)?;
    assert_metric(&record, assertion)
}

fn assert_metric(
    record: &MetricRecordV1,
    assertion: &MetricAssertion,
) -> Result<MetricAssertionResult, MetricDriverError> {
    if record.samples.len() < assertion.minimum_samples || record.samples.is_empty() {
        return Err(MetricDriverError::MissingSamples {
            actual: record.samples.len(),
            minimum: assertion.minimum_samples.max(1),
        });
    }
    let mut samples = record.samples.clone();
    samples.sort_unstable();
    let rank = (95 * samples.len()).div_ceil(100);
    let p95 = samples[rank - 1];
    if p95 > assertion.maximum_p95 {
        return Err(MetricDriverError::Threshold {
            actual: p95,
            maximum: assertion.maximum_p95,
        });
    }
    Ok(MetricAssertionResult {
        p95,
        samples: samples.len(),
    })
}
