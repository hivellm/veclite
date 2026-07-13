//! Criterion benches (SPEC-015 TST-040, gate G1): search p50, index build
//! time, and batch-insert throughput. These run at a modest scale for a quick
//! smoke; the reference-profile targets (search p50 < 3 ms at 1M×512 SQ-8) are
//! measured on the reference runner and recorded in docs/benchmarks.md.
//!
//! Cosine + `Quantization::None` matches the parity reference. Run with
//! `cargo bench -p veclite`.

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use veclite::{Collection, CollectionOptions, Metric, Point, Quantization, VecLite};

const DIM: usize = 512;
const CORPUS: usize = 2_000;
const BUILD_CORPUS: usize = 1_000;
const BATCH: usize = 500;

/// Deterministic splitmix64 — same generator as the recall/parity gates.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    #[allow(clippy::cast_precision_loss)]
    fn vector(&mut self, dim: usize) -> Vec<f32> {
        (0..dim)
            .map(|_| (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0)
            .collect()
    }
}

fn corpus(rng: &mut Rng, n: usize) -> Vec<Vec<f32>> {
    (0..n).map(|_| rng.vector(DIM)).collect()
}

fn collection() -> Collection {
    VecLite::memory()
        .create_collection(
            "bench",
            CollectionOptions::new(DIM, Metric::Cosine).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"))
}

fn seeded(vectors: &[Vec<f32>]) -> Collection {
    let c = collection();
    for (i, v) in vectors.iter().enumerate() {
        c.upsert(Point::new(format!("v{i}"), v.clone()))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    c
}

fn bench_search(c: &mut Criterion) {
    let mut rng = Rng::new(1);
    let vectors = corpus(&mut rng, CORPUS);
    let coll = seeded(&vectors);
    let query = rng.vector(DIM);
    c.bench_function("search/top10_2k_512_cosine", |b| {
        b.iter(|| {
            black_box(
                coll.search(black_box(&query), 10)
                    .unwrap_or_else(|e| panic!("{e}")),
            )
        });
    });
}

fn bench_index_build(c: &mut Criterion) {
    let mut rng = Rng::new(2);
    let vectors = corpus(&mut rng, BUILD_CORPUS);
    c.bench_function("index_build/1k_512_cosine", |b| {
        b.iter_batched(
            || vectors.clone(),
            |vs| black_box(seeded(&vs).len()),
            BatchSize::SmallInput,
        );
    });
}

fn bench_batch_insert(c: &mut Criterion) {
    let mut rng = Rng::new(3);
    let batch: Vec<Point> = (0..BATCH)
        .map(|i| Point::new(format!("v{i}"), rng.vector(DIM)))
        .collect();
    c.bench_function("batch_insert/500_512_cosine", |b| {
        b.iter_batched(
            || (collection(), batch.clone()),
            |(coll, pts)| {
                coll.upsert_batch(black_box(pts))
                    .unwrap_or_else(|e| panic!("{e}"));
                black_box(coll.len())
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(20);
    targets = bench_search, bench_index_build, bench_batch_insert
}
criterion_main!(benches);
