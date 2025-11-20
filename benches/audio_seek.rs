use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::sync::Arc;

/// Simulates the OLD approach: cloning remaining samples
fn seek_with_vec_clone(samples: &[f32], position: usize) -> Vec<f32> {
    samples[position..].to_vec()
}

/// Simulates the NEW approach: Arc + offset (just metadata)
fn seek_with_arc_offset(samples: Arc<[f32]>, position: usize) -> (Arc<[f32]>, usize) {
    (samples, position)
}

fn benchmark_seek_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("audio_seek");

    // Test with different file sizes
    let sizes = vec![
        ("1MB", 250_000),      // ~1MB: 250k samples * 4 bytes/sample
        ("10MB", 2_500_000),   // ~10MB
        ("100MB", 25_000_000), // ~100MB: 10 seconds @ 44.1kHz stereo
    ];

    for (name, num_samples) in sizes {
        // Create test data
        let samples_vec: Vec<f32> = (0..num_samples)
            .map(|i| (i as f32 / num_samples as f32) * 2.0 - 1.0)
            .collect();
        let samples_arc: Arc<[f32]> = samples_vec.clone().into();

        // Test seeking to middle (50% position)
        let seek_position = num_samples / 2;

        group.throughput(Throughput::Bytes(
            (num_samples * std::mem::size_of::<f32>()) as u64,
        ));

        // Benchmark OLD approach (Vec clone)
        group.bench_with_input(
            BenchmarkId::new("vec_clone", name),
            &(&samples_vec, seek_position),
            |b, (samples, pos)| {
                b.iter(|| {
                    let result = seek_with_vec_clone(black_box(samples), black_box(*pos));
                    black_box(result.len());
                });
            },
        );

        // Benchmark NEW approach (Arc + offset)
        group.bench_with_input(
            BenchmarkId::new("arc_offset", name),
            &(samples_arc.clone(), seek_position),
            |b, (samples, pos)| {
                b.iter(|| {
                    let result =
                        seek_with_arc_offset(black_box(Arc::clone(samples)), black_box(*pos));
                    black_box(result.0.len());
                });
            },
        );
    }

    group.finish();
}

fn benchmark_multiple_seeks(c: &mut Criterion) {
    let mut group = c.benchmark_group("multiple_seeks");

    // Simulate realistic playback scenario: 10 seeks during a 10-second file
    let num_samples = 25_000_000; // ~100MB: 10 seconds @ 44.1kHz stereo
    let samples_vec: Vec<f32> = (0..num_samples)
        .map(|i| (i as f32 / num_samples as f32) * 2.0 - 1.0)
        .collect();
    let samples_arc: Arc<[f32]> = samples_vec.clone().into();

    let seek_positions = vec![
        0,
        num_samples / 10,
        num_samples / 5,
        num_samples / 3,
        num_samples / 2,
        num_samples * 2 / 3,
        num_samples * 3 / 4,
        num_samples * 4 / 5,
        num_samples * 9 / 10,
        num_samples - 1000,
    ];

    // Benchmark OLD approach: 10 seeks with Vec cloning
    group.bench_function("vec_clone_10_seeks", |b| {
        b.iter(|| {
            for pos in &seek_positions {
                let result = seek_with_vec_clone(black_box(&samples_vec), black_box(*pos));
                black_box(result.len());
            }
        });
    });

    // Benchmark NEW approach: 10 seeks with Arc
    group.bench_function("arc_offset_10_seeks", |b| {
        b.iter(|| {
            for pos in &seek_positions {
                let result =
                    seek_with_arc_offset(black_box(Arc::clone(&samples_arc)), black_box(*pos));
                black_box(result.0.len());
            }
        });
    });

    group.finish();
}

fn benchmark_memory_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_pressure");

    // Simulate high-frequency seeking (e.g., scrubbing through timeline)
    let num_samples = 25_000_000; // ~100MB
    let samples_vec: Vec<f32> = (0..num_samples)
        .map(|i| (i as f32 / num_samples as f32) * 2.0 - 1.0)
        .collect();
    let samples_arc: Arc<[f32]> = samples_vec.clone().into();

    // 100 random seeks
    let mut seed = 12345u64;
    let mut next_random = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed % num_samples as u64) as usize
    };
    let seek_positions: Vec<usize> = (0..100).map(|_| next_random()).collect();

    // OLD: Vec cloning creates massive memory pressure
    group.bench_function("vec_100_random_seeks", |b| {
        b.iter(|| {
            for pos in &seek_positions {
                let result = seek_with_vec_clone(black_box(&samples_vec), black_box(*pos));
                black_box(result.len());
            }
        });
    });

    // NEW: Arc has minimal memory impact
    group.bench_function("arc_100_random_seeks", |b| {
        b.iter(|| {
            for pos in &seek_positions {
                let result =
                    seek_with_arc_offset(black_box(Arc::clone(&samples_arc)), black_box(*pos));
                black_box(result.0.len());
            }
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_seek_operations,
    benchmark_multiple_seeks,
    benchmark_memory_pressure
);
criterion_main!(benches);
