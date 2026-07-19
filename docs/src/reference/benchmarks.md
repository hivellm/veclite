# Benchmarks

Two things are measured, both reproducibly and both with losses published
alongside wins:

1. **Server parity** — VecLite vs the Vectorizer server (top-10 overlap ≥ 0.99).
2. **Embedded peers** — VecLite vs sqlite-vec, LanceDB, and Chroma.

The harness, the pinned datasets, and the hardware profile are all disclosed, so
a third party can reproduce every number on equivalent hardware (SPEC-015
TST-042).

## Embedded-peer comparison

**Harness**: [`bench/harness.py`](https://github.com/hivellm/veclite/blob/main/bench/harness.py)
— see [`bench/README.md`](https://github.com/hivellm/veclite/blob/main/bench/README.md)
for how to run it and the full methodology. **Dataset**: a clustered mixture of
Gaussians (representative of real embeddings, not the uniform-random ANN worst
case), seeded so a rerun reproduces the recall exactly.

**Reference run** — 20 000 × 256, cosine, 64 clusters, 200 queries, top-10, all
stores at full precision:

| store | build (s) | query p50 (ms) | query p95 (ms) | recall@10 | queries/s |
|---|---|---|---|---|---|
| **veclite** | 16.92 | **0.55** | **0.80** | 0.988 | **1737** |
| sqlite-vec | **0.16** | 6.72 | 8.00 | 1.000 | 147 |
| lancedb | 0.50 | 35.76 | 42.58 | 1.000 | 28 |
| chroma | 2.38 | 0.74 | 1.03 | 1.000 | 1129 |

**Hardware**: AMD Ryzen (AMD64 Family 25), Windows 10, Python 3.13. Reproduce
with `python bench/harness.py --vectors 20000 --queries 200 --dim 256 --k 10`.

### Reading the table honestly

- **VecLite has the fastest queries** (0.55 ms p50, ~1.7 k queries/s) and a
  single-file format, at high recall (0.988) from its HNSW index.
- **VecLite has the slowest build.** Building the HNSW graph costs 16.9 s here —
  far more than the flat-index peers (sqlite-vec, LanceDB build in < 0.5 s) and
  ~7× Chroma's HNSW build (2.4 s). This is the real trade: VecLite pays at build
  time to be fast and durable at query time. If your workload rebuilds
  constantly or is tiny, a flat store may suit you better.
- **sqlite-vec and LanceDB are exact** (recall 1.000) — they scan rather than
  approximate. That is ideal for small or exactness-critical collections;
  sqlite-vec's query latency (6.7 ms) and LanceDB's (35.8 ms, no ANN index by
  default) grow linearly with the collection, where VecLite's and Chroma's HNSW
  stay roughly flat.
- **Chroma is the closest peer** — also HNSW, comparable query latency, a faster
  build, but a client/collection model rather than a single portable file.

The recall column is the honest key: `1.000` marks an exact store, `< 1.000` an
approximate ANN index. Comparing an approximate store's latency to an exact
store's is only fair with recall in view — which is why it is in the table.

### Caveats (fairness)

- Peers run **out of the box** with default settings. LanceDB can add an IVF/HNSW
  index that would trade its exact recall for far faster queries; the number
  above is its default flat scan. Chroma and VecLite use default HNSW
  parameters.
- VecLite is measured through its **release** Python wheel (a debug wheel is
  ~10× slower and would misrepresent it — see the harness README).
- Numbers are single-run on one machine; treat them as order-of-magnitude, and
  rerun on your own hardware for decisions. The dataset and harness are fixed, so
  the *shape* of the result (VecLite fastest queries / slowest build, exact vs
  approximate peers) reproduces; absolute milliseconds vary by machine.

## Server parity & the reference profile

The VecLite-vs-server parity result (top-10 overlap ≥ 0.99) and the pinned
reference hardware profiles (desktop and cloud) are recorded in
[`docs/benchmarks.md`](../../benchmarks.md), which closes gate G1. The
graduation round-trip that guarantees scoring survives the move to the server is
in the [graduation guide](../guides/graduation.md).
