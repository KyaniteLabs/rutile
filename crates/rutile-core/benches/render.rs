use std::hint::black_box;
use std::time::{Duration, Instant};

use rutile_core::render_markdown;

fn main() {
    measure(1024 * 1024, Duration::from_secs(2));
    measure(5 * 1024 * 1024, Duration::from_secs(10));
}

fn measure(bytes: usize, gate: Duration) {
    let source = format!("{}\n", "a".repeat(bytes - 1));
    let mut samples = Vec::with_capacity(5);
    for revision in 0..5 {
        let started = Instant::now();
        black_box(render_markdown(black_box(&source), revision).unwrap());
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let p95 = samples[4];
    eprintln!(
        "render {:.0} MiB p95 {:?} (5 measured, gate {:?})",
        bytes as f64 / (1024.0 * 1024.0),
        p95,
        gate
    );
    assert!(p95 <= gate, "render p95 {p95:?} exceeded {gate:?}");
}
