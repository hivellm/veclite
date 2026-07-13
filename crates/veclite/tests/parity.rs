//! Server-parity harness (SPEC-015 TST-030, gate G1): the same pinned corpus
//! is loaded into VecLite and a pinned Vectorizer server, and the top-10
//! results for each query must overlap by >= 0.99 (NFR-04).
//!
//! This needs a running Vectorizer server, so it is gated on `VECLITE_PARITY_URL`
//! and is a no-op (prints why) when that is unset — a normal `cargo test` never
//! requires the server. To run it for real against the pinned `3.5.0` image:
//!
//! ```text
//! docker run -d --name vlp -p 15002:15002 hivehub/vectorizer:3.5.0
//! PW=$(docker exec vlp cat /data/.root_credentials | sed -n 's/^password=//p')
//! VECLITE_PARITY_URL=http://localhost:15002 VECLITE_PARITY_PASSWORD="$PW" \
//!   cargo test --test parity -- --nocapture
//! ```
//!
//! The server's default embedding provider fixes every collection dimension at
//! 512, which is also the benchmark reference dimension, so the corpus is 512-d.
//! Vectors are inserted raw (BYO) via `/insert_vectors`, so both systems index
//! byte-identical data; only the two HNSW implementations differ.

use std::io::{Read, Write};
use std::net::TcpStream;

use serde_json::{Value, json};
use veclite::{CollectionOptions, Metric, Point, Quantization, VecLite};

const DIM: usize = 512;
const CORPUS: usize = 1000;
const QUERIES: usize = 50;
const TOP_K: usize = 10;
const EF_SEARCH: usize = 256;
const PARITY_FLOOR: f32 = 0.99;
/// Cluster count / noise for the corpus. Real embeddings are clustered on a
/// sphere, where HNSW recall is high; uniform-random high-dim data is
/// adversarial (both engines lose recall, so mutual overlap can't reach the
/// floor even when both are correct). Clustered data is the realistic case.
const CLUSTERS: usize = 40;
const NOISE: f32 = 0.05;
/// Both engines rank by cosine: VecLite by request, the server because its
/// `optimized_hnsw` hardcodes `DistCosine` regardless of the collection's
/// configured metric (verified against 3.5.0). Parity is therefore a cosine
/// comparison.
const METRIC: &str = "cosine";

/// Deterministic splitmix64 — same generator as the recall gate, so the corpus
/// is reproducible across runs and machines without a `rand` dependency.
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
    fn component(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }
    fn vector(&mut self, dim: usize) -> Vec<f32> {
        (0..dim).map(|_| self.component()).collect()
    }
    /// A point near a randomly chosen cluster center (center + small noise).
    fn clustered(&mut self, centers: &[Vec<f32>], dim: usize) -> Vec<f32> {
        let c = (self.next_u64() as usize) % centers.len();
        (0..dim)
            .map(|j| centers[c][j] + self.component() * NOISE)
            .collect()
    }
}

/// Minimal blocking HTTP/1.0 JSON client — enough to drive the server's REST
/// API, with no networking crate in the dependency tree (NFR-08). HTTP/1.0 +
/// `Connection: close` makes the body close-delimited, so a single read to EOF
/// returns it without chunked decoding.
fn http(
    authority: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> (u16, String) {
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let mut request = format!(
        "{method} {path} HTTP/1.0\r\nHost: {authority}\r\nConnection: close\r\nAccept: */*\r\n"
    );
    if let Some(t) = token {
        request.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    if body.is_some() {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", payload.len()));
    }
    request.push_str("\r\n");
    request.push_str(&payload);

    let mut stream =
        TcpStream::connect(authority).unwrap_or_else(|e| panic!("connect {authority}: {e}"));
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|e| panic!("write: {e}"));
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .unwrap_or_else(|e| panic!("read: {e}"));
    let text = String::from_utf8_lossy(&raw);
    let (head, resp_body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, resp_body.to_string())
}

fn json_body(status: u16, body: &str, ctx: &str) -> Value {
    assert!((200..300).contains(&status), "{ctx}: HTTP {status}: {body}");
    serde_json::from_str(body).unwrap_or_else(|e| panic!("{ctx}: bad JSON ({e}): {body}"))
}

/// Top-`k` ids the server returns for `query` (already ranked by the server).
fn server_top_k(authority: &str, token: &str, coll: &str, query: &[f32], k: usize) -> Vec<String> {
    let (st, body) = http(
        authority,
        "POST",
        &format!("/collections/{coll}/search"),
        Some(token),
        Some(&json!({ "vector": query, "limit": k })),
    );
    let v = json_body(st, &body, "search");
    v["results"]
        .as_array()
        .unwrap_or_else(|| panic!("search: no results array: {body}"))
        .iter()
        .filter_map(|r| r["id"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn top10_overlap_with_server_is_at_least_0_99() {
    let Ok(url) = std::env::var("VECLITE_PARITY_URL") else {
        eprintln!("parity: VECLITE_PARITY_URL unset — skipping (no server). See the module docs.");
        return;
    };
    let password = std::env::var("VECLITE_PARITY_PASSWORD")
        .unwrap_or_else(|_| panic!("VECLITE_PARITY_URL set but VECLITE_PARITY_PASSWORD is not"));
    let authority = url
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned();
    let coll = "veclite_parity";

    // ── Corpus (identical vectors for both engines) ──────────────────────
    let mut rng = Rng::new(0x5EED_0000_C0DE_0001);
    let centers: Vec<Vec<f32>> = (0..CLUSTERS).map(|_| rng.vector(DIM)).collect();
    let corpus: Vec<Vec<f32>> = (0..CORPUS).map(|_| rng.clustered(&centers, DIM)).collect();
    let queries: Vec<Vec<f32>> = (0..QUERIES).map(|_| rng.clustered(&centers, DIM)).collect();

    // ── Authenticate ─────────────────────────────────────────────────────
    let (st, body) = http(
        &authority,
        "POST",
        "/auth/login",
        None,
        Some(&json!({ "username": "admin", "password": password })),
    );
    let token = json_body(st, &body, "login")["access_token"]
        .as_str()
        .unwrap_or_else(|| panic!("login: no access_token: {body}"))
        .to_owned();

    // ── Server side: fresh collection + raw BYO insert ───────────────────
    http(
        &authority,
        "DELETE",
        &format!("/collections/{coll}"),
        Some(&token),
        None,
    );
    let (st, body) = http(
        &authority,
        "POST",
        "/collections",
        Some(&token),
        Some(&json!({ "name": coll, "dimension": DIM, "metric": METRIC })),
    );
    json_body(st, &body, "create collection");
    let vectors: Vec<Value> = corpus
        .iter()
        .enumerate()
        .map(|(i, v)| json!({ "id": format!("v{i}"), "embedding": v }))
        .collect();
    let (st, body) = http(
        &authority,
        "POST",
        "/insert_vectors",
        Some(&token),
        Some(&json!({ "collection": coll, "vectors": vectors })),
    );
    json_body(st, &body, "insert_vectors");

    // ── VecLite side: same corpus ────────────────────────────────────────
    let db = VecLite::memory();
    let vl = db
        .create_collection(
            coll,
            CollectionOptions::new(DIM, Metric::Cosine).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    for (i, v) in corpus.iter().enumerate() {
        vl.upsert(Point::new(format!("v{i}"), v.clone()))
            .unwrap_or_else(|e| panic!("{e}"));
    }

    // ── Compare top-10 per query ─────────────────────────────────────────
    let mut total = 0.0f32;
    for query in &queries {
        let server = server_top_k(&authority, &token, coll, query, TOP_K);
        let mine: std::collections::HashSet<String> = vl
            .query(query)
            .limit(TOP_K)
            .ef_search(EF_SEARCH)
            .run()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .map(|h| h.id)
            .collect();
        let hits = server.iter().filter(|id| mine.contains(*id)).count();
        #[allow(clippy::cast_precision_loss)]
        {
            total += hits as f32 / TOP_K as f32;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let overlap = total / QUERIES as f32;
    http(
        &authority,
        "DELETE",
        &format!("/collections/{coll}"),
        Some(&token),
        None,
    );

    println!(
        "parity: mean top-{TOP_K} overlap = {overlap:.4} over {QUERIES} queries (floor {PARITY_FLOOR})"
    );
    assert!(
        overlap >= PARITY_FLOOR,
        "server-parity overlap {overlap:.4} < {PARITY_FLOOR}"
    );
}
