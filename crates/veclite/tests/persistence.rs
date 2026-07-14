//! End-to-end persistence (SPEC-002 §5 + SPEC-003): checkpoint→reopen,
//! WAL replay after a simulated crash, delete/rename durability, and the
//! stale-WAL guard. Native-only — `VecLite::open` does not exist on wasm32
//! (CORE-004), and `cargo test` runs on the host anyway.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use veclite::{CollectionOptions, Metric, Point, Quantization, VecLite};

/// Deterministic splitmix64 for reproducible op sequences.
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
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn db_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("veclite-it-{}-{name}.veclite", std::process::id()))
}

fn wal_path(db: &Path) -> PathBuf {
    let mut n = db.file_name().unwrap_or_default().to_os_string();
    n.push("-wal");
    db.with_file_name(n)
}

fn cleanup(db: &Path) {
    let _ = std::fs::remove_file(db);
    let _ = std::fs::remove_file(wal_path(db));
}

fn opts(dim: usize) -> CollectionOptions {
    CollectionOptions::new(dim, Metric::Euclidean).quantization(Quantization::None)
}

#[test]
fn checkpoint_then_reopen_preserves_data_and_search() {
    let path = db_path("ckpt");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(3))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![1.0, 2.0, 3.0]).payload(serde_json::json!({"k": 1})))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("b", vec![40.0, 50.0, 60.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    } // drop → close checkpoint (idempotent) + clean header

    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(db.list_collections(), vec!["docs"]);
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 2);
    let a = c
        .get("a")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("a missing"));
    assert_eq!(a.vector, vec![1.0, 2.0, 3.0]);
    assert_eq!(a.payload, Some(serde_json::json!({"k": 1})));
    // The HNSW graph was rebuilt from the loaded vectors (STG-063).
    let hits = c
        .search(&[1.0, 2.0, 3.0], 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "a");
    drop(db);
    cleanup(&path);
}

#[test]
fn wal_replay_recovers_uncheckpointed_writes() {
    let path = db_path("wal");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(3))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("x", vec![1.0, 2.0, 3.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("y", vec![4.0, 5.0, 6.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        c.delete("y").unwrap_or_else(|e| panic!("{e}"));
        // Simulate a crash: leak the handle so Drop's checkpoint never runs and
        // the WAL keeps the CREATE + UPSERT + DELETE entries.
        std::mem::forget(db);
    }

    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 1, "replay should leave one live point");
    assert_eq!(
        c.get("x")
            .unwrap_or_else(|e| panic!("{e}"))
            .map(|p| p.vector),
        Some(vec![1.0, 2.0, 3.0])
    );
    assert!(c.get("y").unwrap_or_else(|e| panic!("{e}")).is_none());
    drop(db);
    cleanup(&path);
}

#[test]
fn delete_and_rename_persist_across_checkpoint() {
    let path = db_path("delrename");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let a = db
            .create_collection("a", opts(2))
            .unwrap_or_else(|e| panic!("{e}"));
        a.upsert(Point::new("p", vec![1.0, 2.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_collection("b", opts(2))
            .unwrap_or_else(|e| panic!("{e}"));
        db.delete_collection("b").unwrap_or_else(|e| panic!("{e}"));
        db.rename_collection("a", "renamed")
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    }

    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(db.list_collections(), vec!["renamed"]);
    assert_eq!(
        db.collection("renamed")
            .unwrap_or_else(|e| panic!("{e}"))
            .len(),
        1
    );
    drop(db);
    cleanup(&path);
}

#[test]
fn stale_wal_is_ignored() {
    let path = db_path("stale");
    cleanup(&path);
    // A clean, empty database (drop checkpoints → WAL truncated to its header).
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    }
    // Drop a foreign WAL (wrong magic / uuid) next to the file.
    std::fs::write(wal_path(&path), b"NOPENOPENOPENOPE").unwrap_or_else(|e| panic!("{e}"));

    // Open still succeeds; the foreign WAL is ignored (WAL-002), no phantom data.
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(db.list_collections().is_empty());
    drop(db);
    cleanup(&path);
}

#[test]
fn reopen_survives_many_checkpoints() {
    let path = db_path("many");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(4))
            .unwrap_or_else(|e| panic!("{e}"));
        for round in 0..5 {
            for i in 0..20 {
                #[allow(clippy::cast_precision_loss)]
                c.upsert(Point::new(
                    format!("v{i}"),
                    vec![i as f32, round as f32, 0.0, 0.0],
                ))
                .unwrap_or_else(|e| panic!("{e}"));
            }
            db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        }
    }
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    // 20 unique ids, last write wins → 20 live.
    assert_eq!(
        db.collection("docs")
            .unwrap_or_else(|e| panic!("{e}"))
            .len(),
        20
    );
    drop(db);
    cleanup(&path);
}

/// Task 2.1/2.3: arbitrary upsert/delete interleavings with periodic
/// checkpoints, then a crash — the recovered state (checkpoint + WAL replay)
/// equals the model, regardless of whether the crash landed before or after a
/// checkpoint (WAL-032/041).
#[test]
fn replay_and_checkpoints_match_model_after_crash() {
    let path = db_path("model");
    cleanup(&path);
    let mut rng = Rng::new(0x5EED_C0DE_1234_0001);
    let mut model: HashMap<String, Vec<f32>> = HashMap::new();
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(4))
            .unwrap_or_else(|e| panic!("{e}"));
        for step in 0..200u32 {
            let id = format!("id{}", rng.next_u64() % 30);
            if rng.next_u64() % 3 == 0 {
                c.delete(&id).unwrap_or_else(|e| panic!("{e}"));
                model.remove(&id);
            } else {
                let v: Vec<f32> = (0..4).map(|_| rng.next_f32()).collect();
                c.upsert(Point::new(id.clone(), v.clone()))
                    .unwrap_or_else(|e| panic!("{e}"));
                model.insert(id, v);
            }
            if step % 17 == 16 {
                db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
            }
        }
        // Crash after the last op — may be mid-WAL or just after a checkpoint.
        std::mem::forget(db);
    }

    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), model.len(), "live count diverged from model");
    for (id, v) in &model {
        assert_eq!(
            c.get(id)
                .unwrap_or_else(|e| panic!("{e}"))
                .map(|p| p.vector)
                .as_ref(),
            Some(v),
            "id {id} diverged"
        );
    }
    drop(db);
    cleanup(&path);
}

/// Task 2.2: a torn last WAL entry is discarded on open; the entries before it
/// stay intact and the database opens cleanly (WAL-011).
#[test]
fn truncated_wal_tail_recovers_prior_entries() {
    let path = db_path("torntail");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(2))
            .unwrap_or_else(|e| panic!("{e}"));
        #[allow(clippy::cast_precision_loss)]
        for i in 0..6 {
            c.upsert(Point::new(format!("v{i}"), vec![i as f32, 0.0]))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        std::mem::forget(db);
    }
    // Tear the last entry by dropping a few bytes off the WAL.
    let wal = wal_path(&path);
    let len = std::fs::metadata(&wal)
        .unwrap_or_else(|e| panic!("{e}"))
        .len();
    let f = OpenOptions::new()
        .write(true)
        .open(&wal)
        .unwrap_or_else(|e| panic!("{e}"));
    f.set_len(len - 3).unwrap_or_else(|e| panic!("{e}"));
    drop(f);

    // Open succeeds; the 5 entries before the torn last one survive.
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        db.collection("docs")
            .unwrap_or_else(|e| panic!("{e}"))
            .len(),
        5
    );
    drop(db);
    cleanup(&path);
}
