//! Vocabulary lifecycle (SPEC-005 EMB-030/031/032) and custom providers
//! (EMB-011): incremental ingestion, VOCAB persistence across reopen and
//! crash, refit snapshots, and register_embedder with deferred binding.

#![cfg(not(target_arch = "wasm32"))]

use veclite::{CollectionOptions, Embedder, Point, VecLite, VecLiteError};

fn tmp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "veclite-emblife-{}-{name}.veclite",
        std::process::id()
    ))
}

fn cleanup(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let _ = std::fs::remove_file(std::path::PathBuf::from(wal));
}

fn docs() -> Vec<(String, String, Option<serde_json::Value>)> {
    [
        ("cats", "cats are small furry animals that meow"),
        ("dogs", "dogs are loyal furry animals that bark"),
        ("cars", "cars are fast vehicles with engines"),
        ("rails", "trains are long vehicles on steel rails"),
    ]
    .into_iter()
    .map(|(id, t)| (id.to_owned(), t.to_owned(), None))
    .collect()
}

#[test]
fn vocab_persists_across_reopen_without_reembedding() {
    let path = tmp("reopen");
    cleanup(&path);
    let (hits_before, vec_before) = {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("t", CollectionOptions::auto_embed("bm25", 64))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert_text_batch(docs())
            .unwrap_or_else(|e| panic!("{e}"));
        let hits = c
            .search_text("furry animals that meow", 2)
            .unwrap_or_else(|e| panic!("{e}"));
        let v = c
            .get("cats")
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("cats"))
            .vector;
        (hits, v)
        // close: checkpoint seals the VOCAB segment
    };
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("t").unwrap_or_else(|e| panic!("{e}"));
    // The imported VOCAB serves searches immediately: identical results and
    // identical scores (EMB-020 / acceptance 2)...
    let hits_after = c
        .search_text("furry animals that meow", 2)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits_before.len(), hits_after.len());
    for (b, a) in hits_before.iter().zip(&hits_after) {
        assert_eq!(b.id, a.id);
        assert!((b.score - a.score).abs() < 1e-6);
    }
    // ...and — unlike the pre-3f rebuild-on-first-search — nothing was
    // re-embedded: stored vectors are byte-identical and no tombstone churn.
    let vec_after = c
        .get("cats")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("cats"))
        .vector;
    assert_eq!(vec_before, vec_after);
    assert_eq!(c.stats().tombstones, 0, "reopen+search must not re-embed");
    cleanup(&path);
}

#[test]
fn incremental_state_replays_exactly_after_a_crash() {
    let path = tmp("crash");
    cleanup(&path);
    let hits_before = {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("t", CollectionOptions::auto_embed("bm25", 64))
            .unwrap_or_else(|e| panic!("{e}"));
        // Two batches, a refit (journals a VOCAB snapshot), then one more
        // incremental batch — the WAL now holds upserts + snapshot + upserts.
        c.upsert_text_batch(docs())
            .unwrap_or_else(|e| panic!("{e}"));
        c.refit().unwrap_or_else(|e| panic!("{e}"));
        c.upsert_text("planes", "planes are fast aircraft with jet engines")
            .unwrap_or_else(|e| panic!("{e}"));
        let hits = c
            .search_text("fast engines", 3)
            .unwrap_or_else(|e| panic!("{e}"));
        db.__test_simulate_crash();
        hits
    };
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("t").unwrap_or_else(|e| panic!("{e}"));
    let hits_after = c
        .search_text("fast engines", 3)
        .unwrap_or_else(|e| panic!("{e}"));
    let before: Vec<(String, String)> = hits_before
        .iter()
        .map(|h| (h.id.clone(), format!("{:.6}", h.score)))
        .collect();
    let after: Vec<(String, String)> = hits_after
        .iter()
        .map(|h| (h.id.clone(), format!("{:.6}", h.score)))
        .collect();
    assert_eq!(before, after, "recovery must reproduce the exact state");
    cleanup(&path);
}

#[test]
fn incremental_then_refit_equals_scratch_fit() {
    // EMB-031: refit is the exact recompute — after it, the state must equal
    // a from-scratch fit on the same corpus regardless of ingestion order.
    let db = VecLite::memory();
    let a = db
        .create_collection("a", CollectionOptions::auto_embed("bm25", 64))
        .unwrap_or_else(|e| panic!("{e}"));
    for (id, text, _) in docs() {
        a.upsert_text(id, text).unwrap_or_else(|e| panic!("{e}"));
    }
    a.refit().unwrap_or_else(|e| panic!("{e}"));

    let b = db
        .create_collection("b", CollectionOptions::auto_embed("bm25", 64))
        .unwrap_or_else(|e| panic!("{e}"));
    b.upsert_text_batch(docs())
        .unwrap_or_else(|e| panic!("{e}"));
    b.refit().unwrap_or_else(|e| panic!("{e}"));

    for query in ["furry animals", "fast vehicles engines", "steel rails"] {
        let ha = a.search_text(query, 4).unwrap_or_else(|e| panic!("{e}"));
        let hb = b.search_text(query, 4).unwrap_or_else(|e| panic!("{e}"));
        let pa: Vec<(String, String)> = ha
            .iter()
            .map(|h| (h.id.clone(), format!("{:.6}", h.score)))
            .collect();
        let pb: Vec<(String, String)> = hb
            .iter()
            .map(|h| (h.id.clone(), format!("{:.6}", h.score)))
            .collect();
        assert_eq!(pa, pb, "query {query:?}");
    }
}

/// A trivial deterministic custom embedder: hashes chars into buckets.
struct CharBuckets {
    dimension: usize,
}
impl Embedder for CharBuckets {
    fn embed(&self, text: &str) -> veclite::Result<Vec<f32>> {
        let mut v = vec![0.0f32; self.dimension];
        for ch in text.chars() {
            v[(ch as usize) % self.dimension] += 1.0;
        }
        Ok(v)
    }
    fn dimension(&self) -> usize {
        self.dimension
    }
    fn fit(&mut self, _corpus: &[&str]) -> veclite::Result<()> {
        Ok(())
    }
    fn export_state(&self) -> veclite::Result<Vec<u8>> {
        Ok(Vec::new())
    }
    fn import_state(&mut self, _state: &[u8]) -> veclite::Result<()> {
        Ok(())
    }
}

#[test]
fn register_embedder_works_and_reopen_without_it_defers() {
    let path = tmp("register");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        // Built-in and duplicate names are rejected (EMB-011).
        assert!(matches!(
            db.register_embedder("bm25", Box::new(CharBuckets { dimension: 8 })),
            Err(VecLiteError::AlreadyExists(_))
        ));
        db.register_embedder("charbuckets", Box::new(CharBuckets { dimension: 8 }))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            db.register_embedder("charbuckets", Box::new(CharBuckets { dimension: 8 })),
            Err(VecLiteError::AlreadyExists(_))
        ));

        let c = db
            .create_collection("t", CollectionOptions::auto_embed("charbuckets", 8))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert_text("abc", "abcabc")
            .unwrap_or_else(|e| panic!("{e}"));
        let hits = c.search_text("abc", 1).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(hits[0].id, "abc");
    }
    // Reopen WITHOUT registering: open succeeds; vector ops work; text ops
    // fail with UnsupportedProvider naming the remedy (EMB-011/023).
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("t").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 1);
    let got = c
        .get("abc")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("abc"));
    let hits = c.search(&got.vector, 1).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "abc");
    let Err(VecLiteError::UnsupportedProvider { requested, .. }) = c.search_text("abc", 1) else {
        panic!("text op on a missing provider must be UnsupportedProvider")
    };
    assert!(
        requested.contains("register_embedder"),
        "message must name the remedy, got {requested:?}"
    );
    // Registering now binds the deferred collection: text ops work again.
    db.register_embedder("charbuckets", Box::new(CharBuckets { dimension: 8 }))
        .unwrap_or_else(|e| panic!("{e}"));
    let hits = c.search_text("abc", 1).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "abc");
    cleanup(&path);
}

#[test]
fn create_with_unknown_provider_still_fails_fast() {
    let db = VecLite::memory();
    // EMB-021: unknown at create is an error listing what is available —
    // including any registered names.
    db.register_embedder("mine", Box::new(CharBuckets { dimension: 4 }))
        .unwrap_or_else(|e| panic!("{e}"));
    let Err(VecLiteError::UnsupportedProvider { available, .. }) =
        db.create_collection("t", CollectionOptions::auto_embed("bm52", 8))
    else {
        panic!("unknown provider must fail fast")
    };
    assert!(available.iter().any(|p| p == "mine"));
    assert!(available.iter().any(|p| p == "bm25"));
    // BYO upserts on an auto-embed collection remain rejected implicitly by
    // dimension rules; text ops on BYO collections stay InvalidArgument.
    let byo = db
        .create_collection("byo", CollectionOptions::new(2, veclite::Metric::Euclidean))
        .unwrap_or_else(|e| panic!("{e}"));
    byo.upsert(Point::new("x", vec![0.0, 0.0]))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(
        byo.search_text("q", 1),
        Err(VecLiteError::InvalidArgument(_))
    ));
}
