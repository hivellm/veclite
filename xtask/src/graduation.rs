//! Graduation round-trip gate (SPEC-013 §4, TST-032; DAG T5.6).
//!
//! Drives the shared `.vecdb` conformance corpus (IOP-002) end to end:
//!
//! 1. Builds the standard benchmark corpus deterministically (seeded
//!    splitmix64 — same bytes every run) in a VecLite database.
//! 2. Runs the standard queries and pins them against the committed golden
//!    (`tests/compat/vecdb/golden.json`; `--bless` regenerates it).
//! 3. Exports to the server Compact layout, re-imports, and re-runs the
//!    queries: top-10 overlap ≥ 0.99 and text scores within 1e-5 (§4.1/§4.2).
//! 4. Runs a **second** export→import cycle and requires it stable against
//!    the first (no drift, §4.2).
//! 5. Materializes the corpus into the pinned Vectorizer server repo
//!    (`crates/vectorizer/tests/compat/veclite/`) and runs the server-side
//!    conformance test there — the same golden, asserted by the server's own
//!    `StorageReader` + BM25 provider code (IOP-002: the corpus runs in both
//!    repos' gates). GitHub Actions are disabled for this project, so "both
//!    repos' CI" is the local quality gate on each side; the server check
//!    runs against the pinned server *sources* rather than a docker image —
//!    the same code a dockerized server would run, without the network.
//!
//! Usage: `cargo xtask graduation [--bless] [--skip-server] [--vectorizer <path>]`

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use veclite::interop::{ExportOptions, ImportOptions, export_vecdb, import_vecdb};
use veclite::{CollectionOptions, Metric, Point, VecLite};

const GOLDEN_PATH: &str = "tests/compat/vecdb/golden.json";
const EXPORT_DIR: &str = "target/graduation/export";
const OVERLAP_GATE: f64 = 0.99; // NFR-04
const SCORE_TOL: f32 = 1e-5;
const TOP_K: usize = 10;

// ── Deterministic corpus (the "standard benchmark corpus") ───────────────────

/// Word pool for synthetic documents; queries draw from the same pool so BM25
/// rankings are meaningful, not degenerate.
const WORDS: [&str; 48] = [
    "vector",
    "database",
    "embedded",
    "search",
    "index",
    "hnsw",
    "quantization",
    "payload",
    "filter",
    "hybrid",
    "sparse",
    "dense",
    "cosine",
    "euclidean",
    "segment",
    "checkpoint",
    "wal",
    "durability",
    "crash",
    "recovery",
    "snapshot",
    "vacuum",
    "alias",
    "collection",
    "upsert",
    "query",
    "scroll",
    "batch",
    "tokenizer",
    "vocabulary",
    "bm25",
    "tfidf",
    "provider",
    "graph",
    "server",
    "client",
    "graduation",
    "export",
    "import",
    "archive",
    "portable",
    "image",
    "storage",
    "format",
    "golden",
    "corpus",
    "conformance",
    "parity",
];

/// splitmix64 — the same deterministic generator the crash harness uses.
struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
    #[allow(clippy::cast_precision_loss)]
    fn unit_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn sentence(rng: &mut Rng, words: usize) -> String {
    let mut out = Vec::with_capacity(words);
    for _ in 0..words {
        out.push(WORDS[rng.below(WORDS.len() as u64) as usize]);
    }
    out.join(" ")
}

struct Corpus {
    docs: Vec<(String, String)>,
    text_queries: Vec<String>,
    vecs: Vec<(String, Vec<f32>)>,
    vector_queries: Vec<Vec<f32>>,
}

fn build_corpus() -> Corpus {
    let mut rng = Rng(0x5EC1_17E5_D0C5_2026);
    let docs = (0..120)
        .map(|i| {
            let len = 12 + rng.below(28) as usize;
            (format!("doc-{i:03}"), sentence(&mut rng, len))
        })
        .collect();
    let text_queries = (0..15)
        .map(|_| {
            let len = 3 + rng.below(3) as usize;
            sentence(&mut rng, len)
        })
        .collect();
    let vecs = (0..200)
        .map(|i| {
            let v: Vec<f32> = (0..64).map(|_| rng.unit_f32() * 2.0 - 1.0).collect();
            (format!("vec-{i:03}"), v)
        })
        .collect();
    let vector_queries = (0..10)
        .map(|_| (0..64).map(|_| rng.unit_f32() * 2.0 - 1.0).collect())
        .collect();
    Corpus {
        docs,
        text_queries,
        vecs,
        vector_queries,
    }
}

fn build_db(corpus: &Corpus) -> Result<VecLite, String> {
    let db = VecLite::memory();
    let docs = db
        .create_collection("docs", CollectionOptions::auto_embed("bm25", 256))
        .map_err(|e| e.to_string())?;
    for (id, text) in &corpus.docs {
        docs.upsert_text(id, text).map_err(|e| e.to_string())?;
    }
    // Settle the vocabulary exactly as a search would, so golden, export, and
    // server all see the same fitted state.
    docs.refit().map_err(|e| e.to_string())?;
    let vecs = db
        .create_collection("vecs", CollectionOptions::new(64, Metric::Cosine))
        .map_err(|e| e.to_string())?;
    for (id, vector) in &corpus.vecs {
        vecs.upsert(Point::new(id.clone(), vector.clone()))
            .map_err(|e| e.to_string())?;
    }
    db.create_alias("latest-docs", "docs")
        .map_err(|e| e.to_string())?;
    Ok(db)
}

// ── Golden schema (shared with the server-side test) ─────────────────────────

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct Hit {
    id: String,
    score: f32,
}

#[derive(Serialize, Deserialize)]
struct TextQueryGolden {
    query: String,
    /// The fitted BM25 embedding of the query — the server-side test asserts
    /// its own provider reproduces it within 1e-5 from the exported tokenizer.
    embedding: Vec<f32>,
    top: Vec<Hit>,
}

#[derive(Serialize, Deserialize)]
struct VectorQueryGolden {
    vector: Vec<f32>,
    top: Vec<Hit>,
}

#[derive(Serialize, Deserialize)]
struct Golden {
    doc_count: usize,
    vec_count: usize,
    text_queries: Vec<TextQueryGolden>,
    vector_queries: Vec<VectorQueryGolden>,
}

struct Observed {
    text: Vec<Vec<Hit>>,
    vectors: Vec<Vec<Hit>>,
}

fn run_queries(db: &VecLite, corpus: &Corpus) -> Result<Observed, String> {
    let docs = db.collection("docs").map_err(|e| e.to_string())?;
    let vecs = db.collection("vecs").map_err(|e| e.to_string())?;
    let mut text = Vec::new();
    for query in &corpus.text_queries {
        let hits = docs.search_text(query, TOP_K).map_err(|e| e.to_string())?;
        text.push(
            hits.into_iter()
                .map(|h| Hit {
                    id: h.id,
                    score: h.score,
                })
                .collect(),
        );
    }
    let mut vectors = Vec::new();
    for query in &corpus.vector_queries {
        let hits = vecs
            .query(query)
            .limit(TOP_K)
            .run()
            .map_err(|e| e.to_string())?;
        vectors.push(
            hits.into_iter()
                .map(|h| Hit {
                    id: h.id,
                    score: h.score,
                })
                .collect(),
        );
    }
    Ok(Observed { text, vectors })
}

/// The fitted BM25 query embeddings, from a standalone provider fitted on the
/// same ordered corpus (`Collection::refit` fits on live `_text` in slot
/// order, so the states coincide — pinned by the golden comparison below).
fn query_embeddings(corpus: &Corpus) -> Result<Vec<Vec<f32>>, String> {
    let mut provider = veclite::build_provider("bm25", 256).map_err(|e| e.to_string())?;
    let texts: Vec<&str> = corpus.docs.iter().map(|(_, t)| t.as_str()).collect();
    provider.fit(&texts).map_err(|e| e.to_string())?;
    corpus
        .text_queries
        .iter()
        .map(|q| provider.embed(q).map_err(|e| e.to_string()))
        .collect()
}

// ── Comparisons ──────────────────────────────────────────────────────────────

fn overlap(a: &[Hit], b: &[Hit]) -> f64 {
    let sa: BTreeSet<&str> = a.iter().map(|h| h.id.as_str()).collect();
    let sb: BTreeSet<&str> = b.iter().map(|h| h.id.as_str()).collect();
    let shared = sa.intersection(&sb).count();
    #[allow(clippy::cast_precision_loss)]
    if sa.is_empty() && sb.is_empty() {
        1.0
    } else {
        shared as f64 / sa.len().max(sb.len()).max(1) as f64
    }
}

/// Average top-K overlap across query sets, plus score parity on shared ids.
fn compare(
    label: &str,
    reference: &[Vec<Hit>],
    observed: &[Vec<Hit>],
    score_tol: f32,
) -> Result<f64, String> {
    if reference.len() != observed.len() {
        return Err(format!(
            "{label}: query count mismatch ({} vs {})",
            reference.len(),
            observed.len()
        ));
    }
    let mut total = 0.0;
    for (i, (want, got)) in reference.iter().zip(observed).enumerate() {
        total += overlap(want, got);
        for hit in got {
            if let Some(expected) = want.iter().find(|w| w.id == hit.id)
                && (expected.score - hit.score).abs() > score_tol
            {
                return Err(format!(
                    "{label}: query {i}, id {:?}: score {} vs {} (tol {score_tol})",
                    hit.id, hit.score, expected.score
                ));
            }
        }
    }
    #[allow(clippy::cast_precision_loss)]
    Ok(total / reference.len().max(1) as f64)
}

// ── Entry point ──────────────────────────────────────────────────────────────

/// `cargo xtask graduation [--bless] [--skip-server] [--vectorizer <path>]`.
pub fn run(args: &[String]) -> i32 {
    match run_inner(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("[graduation] FAIL: {e}");
            1
        }
    }
}

fn run_inner(args: &[String]) -> Result<(), String> {
    let bless = args.iter().any(|a| a == "--bless");
    let skip_server = args.iter().any(|a| a == "--skip-server");
    let vectorizer_repo = args
        .iter()
        .position(|a| a == "--vectorizer")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../Vectorizer"));

    let corpus = build_corpus();
    let db = build_db(&corpus)?;
    let observed = run_queries(&db, &corpus)?;
    let embeddings = query_embeddings(&corpus)?;

    // 1. Golden: bless or assert.
    let golden = Golden {
        doc_count: corpus.docs.len(),
        vec_count: corpus.vecs.len(),
        text_queries: corpus
            .text_queries
            .iter()
            .zip(&embeddings)
            .zip(&observed.text)
            .map(|((query, embedding), top)| TextQueryGolden {
                query: query.clone(),
                embedding: embedding.clone(),
                top: top.clone(),
            })
            .collect(),
        vector_queries: corpus
            .vector_queries
            .iter()
            .zip(&observed.vectors)
            .map(|(vector, top)| VectorQueryGolden {
                vector: vector.clone(),
                top: top.clone(),
            })
            .collect(),
    };
    let golden_path = PathBuf::from(GOLDEN_PATH);
    if bless {
        if let Some(parent) = golden_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = serde_json::to_string_pretty(&golden).map_err(|e| e.to_string())?;
        out.push('\n');
        std::fs::write(&golden_path, out).map_err(|e| e.to_string())?;
        eprintln!("[graduation] blessed golden → {GOLDEN_PATH}");
    } else {
        let text = std::fs::read_to_string(&golden_path)
            .map_err(|e| format!("read {GOLDEN_PATH}: {e} (run `--bless` to create it)"))?;
        let committed: Golden = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let text_ref: Vec<Vec<Hit>> = committed
            .text_queries
            .iter()
            .map(|q| q.top.clone())
            .collect();
        let vec_ref: Vec<Vec<Hit>> = committed
            .vector_queries
            .iter()
            .map(|q| q.top.clone())
            .collect();
        compare("golden text", &text_ref, &observed.text, SCORE_TOL)?;
        compare("golden vectors", &vec_ref, &observed.vectors, SCORE_TOL)?;
        for (i, (committed_q, fresh)) in committed.text_queries.iter().zip(&embeddings).enumerate()
        {
            if committed_q.embedding.len() != fresh.len()
                || committed_q
                    .embedding
                    .iter()
                    .zip(fresh)
                    .any(|(a, b)| (a - b).abs() > SCORE_TOL)
            {
                return Err(format!("golden: query {i} embedding drifted"));
            }
        }
        eprintln!("[graduation] golden PASS ({GOLDEN_PATH})");
    }

    // 2. Cycle 1: export → import → same queries (SPEC-013 §4.1 VecLite-side).
    let export_dir = PathBuf::from(EXPORT_DIR);
    let _ = std::fs::remove_dir_all(&export_dir);
    let report = export_vecdb(&db, &export_dir, &ExportOptions::default())
        .map_err(|e| format!("export: {e}"))?;
    for warning in &report.warnings {
        eprintln!("[graduation] export warning: {warning}");
    }
    let imported = VecLite::memory();
    import_vecdb(&export_dir, &imported, &ImportOptions::default())
        .map_err(|e| format!("import: {e}"))?;
    let cycle1 = run_queries(&imported, &corpus)?;
    let text_overlap = compare("cycle-1 text", &observed.text, &cycle1.text, SCORE_TOL)?;
    let vec_overlap = compare(
        "cycle-1 vectors",
        &observed.vectors,
        &cycle1.vectors,
        SCORE_TOL,
    )?;
    if text_overlap < OVERLAP_GATE || vec_overlap < OVERLAP_GATE {
        return Err(format!(
            "cycle-1 overlap below gate {OVERLAP_GATE}: text {text_overlap:.4}, vectors {vec_overlap:.4}"
        ));
    }
    eprintln!(
        "[graduation] cycle-1 PASS (overlap text {text_overlap:.4}, vectors {vec_overlap:.4})"
    );

    // 3. Cycle 2: re-export the imported db → import → stable (§4.2 no drift).
    let export_dir2 = PathBuf::from(format!("{EXPORT_DIR}2"));
    let _ = std::fs::remove_dir_all(&export_dir2);
    export_vecdb(&imported, &export_dir2, &ExportOptions::default())
        .map_err(|e| format!("re-export: {e}"))?;
    let imported2 = VecLite::memory();
    import_vecdb(&export_dir2, &imported2, &ImportOptions::default())
        .map_err(|e| format!("re-import: {e}"))?;
    let cycle2 = run_queries(&imported2, &corpus)?;
    let text_overlap2 = compare("cycle-2 text", &cycle1.text, &cycle2.text, SCORE_TOL)?;
    let vec_overlap2 = compare(
        "cycle-2 vectors",
        &cycle1.vectors,
        &cycle2.vectors,
        SCORE_TOL,
    )?;
    if text_overlap2 < OVERLAP_GATE || vec_overlap2 < OVERLAP_GATE {
        return Err(format!(
            "cycle-2 drift: overlap text {text_overlap2:.4}, vectors {vec_overlap2:.4}"
        ));
    }
    eprintln!(
        "[graduation] cycle-2 PASS (overlap text {text_overlap2:.4}, vectors {vec_overlap2:.4})"
    );

    // 4. Server side: materialize the corpus into the pinned server repo and
    // run its conformance test (IOP-002 / TST-032).
    if skip_server {
        eprintln!("[graduation] server step skipped (--skip-server)");
        return Ok(());
    }
    if !vectorizer_repo.join("Cargo.toml").exists() {
        return Err(format!(
            "Vectorizer repo not found at {} — pass --vectorizer <path> or --skip-server",
            vectorizer_repo.display()
        ));
    }
    let fixture_dir = vectorizer_repo.join("crates/vectorizer/tests/compat/veclite");
    std::fs::create_dir_all(&fixture_dir).map_err(|e| e.to_string())?;
    for name in ["vectorizer.vecdb", "vectorizer.vecidx"] {
        std::fs::copy(export_dir.join(name), fixture_dir.join(name))
            .map_err(|e| format!("copy {name}: {e}"))?;
    }
    let golden_bytes = std::fs::read(&golden_path).map_err(|e| e.to_string())?;
    std::fs::write(fixture_dir.join("golden.json"), golden_bytes).map_err(|e| e.to_string())?;
    eprintln!(
        "[graduation] corpus materialized → {}",
        fixture_dir.display()
    );

    eprintln!(
        "[graduation] running server-side conformance (cargo test -p vectorizer --test veclite_compat)…"
    );
    let status = Command::new(env!("CARGO"))
        .current_dir(&vectorizer_repo)
        .args([
            "test",
            "-p",
            "vectorizer",
            "--test",
            "veclite_compat",
            "--",
            "--nocapture",
        ])
        .status()
        .map_err(|e| format!("spawn cargo in {}: {e}", vectorizer_repo.display()))?;
    if !status.success() {
        return Err(format!("server-side conformance FAILED ({status})"));
    }
    eprintln!("[graduation] server-side conformance PASS");
    Ok(())
}
