//! This benchmark is to test the pure speed of running the algorithm across some sample data.
//!
//! The following is tested:
//! - normal (1.0) speed
//! - double (2.0) speed
//! - half (0.5) speed
//!
//! Additionally, this bench can be run with profile "bench-debug" to get the debug target performance.

use std::{
    num::{NonZeroU16, NonZeroU32},
    time::Duration,
};

use criterion::{BatchSize, Criterion, SamplingMode, Throughput, criterion_group, criterion_main};
use rodio::source::Pink;
use wsola::TimeStretch;

const SAMPLE_RATE: NonZeroU32 = NonZeroU32::new(48_000).unwrap();
const CHANNELS: NonZeroU16 = NonZeroU16::new(2).unwrap();
const LENGTH: Duration = Duration::from_secs(15);

fn speed(c: &mut Criterion) {
    let samples = SAMPLE_RATE.get() as u64 * CHANNELS.get() as u64 * LENGTH.as_secs();
    let data: Vec<f32> = Pink::new(SAMPLE_RATE).take(samples as usize).collect();

    let mut group = c.benchmark_group("speed");
    group.throughput(Throughput::Elements(samples));

    // The following values are configured to even work on the unoptimized debug build (bench-debug) in a reasonable amount of time
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(40));

    group.bench_function("normal", |b| {
        b.iter_batched(
            || {
                let mut ts = TimeStretch::new(SAMPLE_RATE, CHANNELS).unwrap();
                ts.set_tempo(1.0);
                ts.push(&data);

                ts
            },
            |mut ts| {
                // Test the step convertion; dont test the output speed.
                let _ = ts.pull(usize::MAX);
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("double", |b| {
        b.iter_batched(
            || {
                let mut ts = TimeStretch::new(SAMPLE_RATE, CHANNELS).unwrap();
                ts.set_tempo(2.0);
                ts.push(&data);

                ts
            },
            |mut ts| {
                // Test the step convertion; dont test the output speed.
                let _ = ts.pull(usize::MAX);
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("half", |b| {
        b.iter_batched(
            || {
                let mut ts = TimeStretch::new(SAMPLE_RATE, CHANNELS).unwrap();
                ts.set_tempo(0.5);
                ts.push(&data);

                ts
            },
            |mut ts| {
                // Test the step convertion; dont test the output speed.
                let _ = ts.pull(usize::MAX);
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

criterion_group!(benches, speed);
criterion_main!(benches);
