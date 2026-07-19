//! Fuzzing surface (SPEC-015 TST-050) — thin, behavior-free wrappers over
//! every parser that consumes untrusted bytes: the file header, TOC, segment
//! framing, WAL replay, filter documents, the MessagePack CONFIG (options)
//! decoding, and the whole-image open path.
//!
//! The contract each wrapper exercises: **arbitrary input yields a typed
//! error, never a panic, hang, or unbounded allocation.** The same functions
//! back the `fuzz/fuzz_targets/*` cargo-fuzz binaries (coverage-guided,
//! nightly) and the stable `fuzz_regression` test that replays the committed
//! corpus on every gate run.
//!
//! `seed_corpus` builds the deterministic seed inputs (valid artifacts per
//! target) that `cargo xtask fuzz-seed` materializes into `fuzz/corpus/`.

use crate::storage::body::StoredConfig;
use crate::storage::header::Header;
use crate::storage::segment::Segment;
use crate::storage::toc::Toc;

/// File header page (SPEC-002 §2).
pub fn run_header(data: &[u8]) {
    let _ = Header::decode(data);
}

/// TOC document (SPEC-002 §4, MessagePack).
pub fn run_toc(data: &[u8]) {
    let _ = Toc::decode(data);
}

/// Segment framing + body decompression (SPEC-002 §3).
pub fn run_segment(data: &[u8]) {
    let _ = Segment::read(data, 0, 0);
}

/// CONFIG segment body — the MessagePack options decoding (SPEC-002 §3.1).
pub fn run_config(data: &[u8]) {
    let _ = StoredConfig::decode(data);
}

/// Portable filter document parsing (SPEC-006 §JSON).
pub fn run_filter(data: &[u8]) {
    if let Ok(text) = core::str::from_utf8(data)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
    {
        let _ = crate::filter::Filter::from_json(&value);
    }
}

/// WAL replay scan (SPEC-003). The uuid prefix is taken from the input's own
/// header position so the fuzzer can reach past the stale-sidecar guard
/// (WAL-002) into entry parsing.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_wal(data: &[u8]) {
    let mut uuid_prefix = [0u8; 8];
    if data.len() >= 16 {
        uuid_prefix.copy_from_slice(&data[8..16]);
    }
    let _ = crate::storage::wal::Wal::scan(data, uuid_prefix);
}

/// The whole-file open path: header → TOC → every segment body → collection
/// reconstruction (`VecLite::deserialize`) — the SPEC-015 "malformed file
/// never crashes" scenario end to end.
pub fn run_image(data: &[u8]) {
    let _ = crate::database::VecLite::deserialize(data);
}

/// Run the named target's wrapper (shared dispatch for the regression test
/// and seed tooling). Unknown names are a caller bug.
pub fn run_target(target: &str, data: &[u8]) -> bool {
    match target {
        "header" => run_header(data),
        "toc" => run_toc(data),
        "segment" => run_segment(data),
        "config" => run_config(data),
        "filter" => run_filter(data),
        #[cfg(not(target_arch = "wasm32"))]
        "wal" => run_wal(data),
        "image" => run_image(data),
        _ => return false,
    }
    true
}

/// Every fuzz-target name, matching `fuzz/fuzz_targets/*.rs` and the corpus
/// directories `fuzz/corpus/<name>/`.
pub const TARGETS: [&str; 7] = [
    "header", "toc", "segment", "config", "filter", "wal", "image",
];

/// One target's seed inputs: `(target name, inputs)`.
pub type SeedSet = (&'static str, Vec<Vec<u8>>);

/// Deterministic seed inputs per target: valid artifacts produced by the real
/// encoders, so coverage-guided mutation starts inside the interesting state
/// space instead of at the magic-byte cliff (TST-050 corpus seeding).
#[cfg(not(target_arch = "wasm32"))]
pub fn seed_corpus() -> crate::error::Result<Vec<SeedSet>> {
    use crate::options::{CollectionOptions, Metric, PayloadIndexKind};
    use crate::point::Point;

    // A small database touching every segment type: auto-embed text (VECTORS,
    // SPARSE, VOCAB, PAYLOAD), BYO vectors with a payload index (PIDX), a
    // tombstone, aliases, and the always-present CONFIG + IDDIR.
    let db = crate::database::VecLite::memory();
    let docs = db.create_collection("docs", CollectionOptions::auto_embed("bm25", 32))?;
    docs.upsert_text("a", "seed corpus for the fuzz targets")?;
    docs.upsert_text("b", "arbitrary bytes must never crash the parsers")?;
    let vecs = db.create_collection(
        "vecs",
        CollectionOptions::new(4, Metric::Euclidean)
            .payload_index("lang", PayloadIndexKind::Keyword),
    )?;
    vecs.upsert(
        Point::new("v1", vec![1.0, 2.0, 3.0, 4.0]).payload(serde_json::json!({"lang": "en"})),
    )?;
    vecs.upsert(Point::new("v2", vec![4.0, 3.0, 2.0, 1.0]))?;
    vecs.delete("v2")?;
    db.create_alias("latest", "docs")?;
    let image = db.serialize()?;

    // Header page + committed TOC slice, straight from the image.
    let header_page = image
        .get(..crate::storage::header::HEADER_SIZE)
        .unwrap_or(&image)
        .to_vec();
    let header = Header::decode(&image)?;
    let toc_start = usize::try_from(header.toc_offset).unwrap_or(0);
    let toc_end = toc_start.saturating_add(usize::try_from(header.toc_len).unwrap_or(0));
    let toc_bytes = image.get(toc_start..toc_end).unwrap_or(&[]).to_vec();
    let toc = Toc::decode(&toc_bytes)?;

    // One seed per live segment — covers every segment type the writer emits.
    let mut segment_seeds = Vec::new();
    for entry in &toc.collections {
        for seg_ref in &entry.live_segments {
            let start = usize::try_from(seg_ref.offset).unwrap_or(0);
            let end = start.saturating_add(usize::try_from(seg_ref.len).unwrap_or(0));
            if let Some(bytes) = image.get(start..end) {
                segment_seeds.push(bytes.to_vec());
            }
        }
    }

    let config_seed = crate::persist::config::to_stored(
        &CollectionOptions::auto_embed("bm25", 256).hnsw(24, 300, 150),
        1_752_000_000,
    )
    .encode()?;

    let filter_seeds: Vec<Vec<u8>> = [
        r#"{"must":[{"key":"lang","match":{"value":"en"}}]}"#,
        r#"{"should":[{"key":"n","range":{"gte":1,"lt":10}}],"must_not":[{"key":"tag","match":{"any":["a","b"]}}]}"#,
        r#"{"must":[{"key":"deep.path","match":{"value":42}},{"key":"flag","match":{"value":true}}]}"#,
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();

    // A real WAL file: open a file-backed db, write without checkpointing,
    // and take the sidecar bytes (torn-tail variants come from mutation).
    let wal_seed = {
        // Unique per call — concurrent test threads in one process must not
        // collide on the advisory-locked temp file.
        static SEED_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEED_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "veclite-fuzz-seed-{}-{seq}.veclite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let wal_path = crate::persist::wal_path(&path);
        let _ = std::fs::remove_file(&wal_path);
        let file_db = crate::database::VecLite::open(&path)?;
        let coll = file_db.create_collection("w", CollectionOptions::new(4, Metric::Cosine))?;
        coll.upsert(
            Point::new("x", vec![0.1, 0.2, 0.3, 0.4]).payload(serde_json::json!({"k": 1})),
        )?;
        coll.upsert(Point::new("y", vec![0.4, 0.3, 0.2, 0.1]))?;
        coll.delete("y")?;
        // Simulate a crash so the WAL keeps its entries instead of being
        // truncated by the close-time checkpoint.
        file_db.__test_simulate_crash();
        let bytes = std::fs::read(&wal_path)?;
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&wal_path);
        bytes
    };

    // Tiny image too — small inputs mutate faster than the full one.
    let tiny = crate::database::VecLite::memory();
    tiny.create_collection("t", CollectionOptions::new(2, Metric::Cosine))?;
    let tiny_image = tiny.serialize()?;

    Ok(vec![
        ("header", vec![header_page]),
        ("toc", vec![toc_bytes]),
        ("segment", segment_seeds),
        ("config", vec![config_seed]),
        ("filter", filter_seeds),
        ("wal", vec![wal_seed]),
        ("image", vec![tiny_image, image]),
    ])
}
