//! WAL-replay and database-management coverage: seal the `CreateColl` with a
//! checkpoint, then journal further ops and simulate a crash so the reopen
//! *replays* them (SPEC-003 WAL-042). Exercises the per-op replay arms and the
//! alias/rename/registered-embedder management paths that only run off disk.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use veclite::{
    CollectionOptions, Durability, Embedder, Metric, OpenOptions, Point, Quantization, VecLite,
    VecLiteError,
};

static N: AtomicU64 = AtomicU64::new(0);

fn tmp() -> PathBuf {
    let n = N.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("veclite-replay-{}-{n}.veclite", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let mut wal = p.file_name().unwrap_or_default().to_os_string();
    wal.push("-wal");
    let _ = std::fs::remove_file(p.with_file_name(wal));
    p
}

fn opts(dim: usize) -> CollectionOptions {
    CollectionOptions::new(dim, Metric::Euclidean).quantization(Quantization::None)
}

fn open(path: &PathBuf) -> VecLite {
    VecLite::open_with(path, OpenOptions::new().durability(Durability::Full))
        .unwrap_or_else(|e| panic!("open: {e}"))
}

#[test]
fn replays_upsert_delete_alias_and_pidx_after_crash() {
    let path = tmp();
    {
        let db = open(&path);
        let c = db
            .create_collection("docs", opts(2))
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}")); // seal CreateColl into the base

        // These land in the WAL only (no further checkpoint):
        c.upsert(Point::new("a", vec![0.0, 0.0]).payload(serde_json::json!({"lang": "en"})))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("b", vec![1.0, 0.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("c", vec![2.0, 0.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        c.delete("c").unwrap_or_else(|e| panic!("{e}"));
        c.create_payload_index("lang", veclite::PayloadIndexKind::Keyword)
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_alias("alias", "docs")
            .unwrap_or_else(|e| panic!("{e}"));
        db.__test_simulate_crash(); // WAL survives for replay
    }

    // Reopen → replays UpsertBatch / DeleteBatch / PidxDeclare / Alias.
    let db = open(&path);
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 2, "a,b survive; c deleted");
    assert!(c.get("a").unwrap_or_else(|e| panic!("{e}")).is_some());
    assert!(c.get("c").unwrap_or_else(|e| panic!("{e}")).is_none());
    // The alias replayed and resolves to the collection.
    assert_eq!(
        db.collection("alias")
            .unwrap_or_else(|e| panic!("{e}"))
            .len(),
        2
    );
    drop(db);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn replays_rename_and_drop_after_crash() {
    let path = tmp();
    {
        let db = open(&path);
        db.create_collection("keep", opts(1))
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_collection("gone", opts(1))
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));

        db.rename_collection("keep", "renamed")
            .unwrap_or_else(|e| panic!("{e}"));
        db.delete_collection("gone")
            .unwrap_or_else(|e| panic!("{e}"));
        db.__test_simulate_crash();
    }
    let db = open(&path);
    let mut names = db.list_collections();
    names.sort();
    assert_eq!(names, vec!["renamed".to_owned()], "rename + drop replayed");
    drop(db);
    let _ = std::fs::remove_file(&path);
}

/// A trivial deterministic embedder for the registered-provider reopen path.
struct ConstEmbedder(usize);
impl Embedder for ConstEmbedder {
    fn embed(&self, _text: &str) -> veclite::Result<Vec<f32>> {
        Ok(vec![0.5; self.0])
    }
    fn dimension(&self) -> usize {
        self.0
    }
    fn fit(&mut self, _corpus: &[&str]) -> veclite::Result<()> {
        Ok(())
    }
    fn export_state(&self) -> veclite::Result<Vec<u8>> {
        Ok(self.0.to_le_bytes().to_vec())
    }
    fn import_state(&mut self, _bytes: &[u8]) -> veclite::Result<()> {
        Ok(())
    }
}

#[test]
fn reopens_collection_backed_by_a_registered_embedder() {
    let path = tmp();
    {
        let db = open(&path);
        db.register_embedder("custom", Box::new(ConstEmbedder(4)))
            .unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("t", CollectionOptions::auto_embed("custom", 4))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert_text("d1", "hello world")
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        drop(db);
    }
    // Reopen and re-register: build_provider("custom") fails (not builtin), so
    // the restore resolves it from the registered embedders.
    let db = open(&path);
    db.register_embedder("custom", Box::new(ConstEmbedder(4)))
        .unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("t").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 1);
    drop(db);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn unsupported_provider_error_lists_registered_names() {
    let db = VecLite::memory();
    db.register_embedder("myprov", Box::new(ConstEmbedder(4)))
        .unwrap_or_else(|e| panic!("{e}"));
    // An unknown builtin provider error is enriched with the registered names.
    let err = db
        .create_collection("t", CollectionOptions::auto_embed("nope", 4))
        .err()
        .unwrap_or_else(|| panic!("expected error"));
    match err {
        VecLiteError::UnsupportedProvider { available, .. } => {
            assert!(available.contains(&"myprov".to_owned()), "{available:?}");
        }
        other => panic!("unexpected: {other}"),
    }
}

#[test]
fn delete_alias_and_rename_persist_on_disk() {
    let path = tmp();
    {
        let db = open(&path);
        db.create_collection("c", opts(1))
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_alias("a1", "c").unwrap_or_else(|e| panic!("{e}"));
        db.delete_alias("a1").unwrap_or_else(|e| panic!("{e}"));
        // A now-deleted alias no longer resolves.
        assert!(matches!(
            db.collection("a1"),
            Err(VecLiteError::CollectionNotFound(_))
        ));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        drop(db);
    }
    let db = open(&path);
    assert!(db.collection("a1").is_err(), "deleted alias stays gone");
    drop(db);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn replays_vocab_update_from_refit() {
    let path = tmp();
    {
        let db = open(&path);
        let c = db
            .create_collection("docs", CollectionOptions::auto_embed("bm25", 64))
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        c.upsert_text("d1", "the quick brown fox")
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert_text("d2", "a lazy dog sleeps")
            .unwrap_or_else(|e| panic!("{e}"));
        c.refit().unwrap_or_else(|e| panic!("{e}")); // journals a VocabUpdate
        db.__test_simulate_crash();
    }
    let db = open(&path);
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 2);
    // The reopened vocabulary searches identically (deterministic after replay).
    let hits = c
        .search_text("quick fox", 2)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!hits.is_empty());
    drop(db);
    let _ = std::fs::remove_file(&path);
}
