//! End-to-end persistence (SPEC-002 §5 + SPEC-003): checkpoint→reopen,
//! WAL replay after a simulated crash, delete/rename durability, and the
//! stale-WAL guard. Native-only — `VecLite::open` does not exist on wasm32
//! (CORE-004), and `cargo test` runs on the host anyway.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use veclite::{CollectionOptions, Metric, Point, Quantization, VecLite, VecLiteError};

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
        db.__test_simulate_crash();
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
            if rng.next_u64().is_multiple_of(3) {
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
        db.__test_simulate_crash();
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
        db.__test_simulate_crash();
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

/// Task 2.2: a second read-write open of the same file fails fast with `Locked`,
/// and a read-only open conflicts with the held exclusive lock too (STG-060).
#[test]
fn second_open_gets_locked() {
    let path = db_path("lock");
    cleanup(&path);
    let db1 = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(VecLite::open(&path), Err(VecLiteError::Locked)));
    assert!(matches!(
        VecLite::open_with(&path, veclite::OpenOptions::new().read_only(true)),
        Err(VecLiteError::Locked)
    ));
    drop(db1); // releases the lock
    let db2 = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    drop(db2);
    cleanup(&path);
}

/// Task 1.5: a read-only database serves reads but rejects every mutation with
/// `ReadOnly` (STG-062).
#[test]
fn read_only_rejects_writes_allows_reads() {
    let path = db_path("ro");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(3))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![1.0, 2.0, 3.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    }
    let db = VecLite::open_with(&path, veclite::OpenOptions::new().read_only(true))
        .unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 1); // reads work
    assert_eq!(
        c.search(&[1.0, 2.0, 3.0], 1)
            .unwrap_or_else(|e| panic!("{e}"))[0]
            .id,
        "a"
    );
    assert!(matches!(
        c.upsert(Point::new("b", vec![4.0, 5.0, 6.0])),
        Err(VecLiteError::ReadOnly)
    ));
    assert!(matches!(
        db.create_collection("x", opts(3)),
        Err(VecLiteError::ReadOnly)
    ));
    drop(db);
    cleanup(&path);
}

/// Task 1.5 / WAL-043: a read-only open over a pending WAL fails with
/// `WalPending`; `read_only_ignore_wal` opens the last checkpoint instead.
#[test]
fn read_only_wal_pending_guard() {
    let path = db_path("ropending");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(2))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![1.0, 2.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.__test_simulate_crash(); // WAL has uncheckpointed entries
    }
    assert!(matches!(
        VecLite::open_with(&path, veclite::OpenOptions::new().read_only(true)),
        Err(VecLiteError::WalPending)
    ));
    // With ignore, it opens the last checkpoint (gen 0, before the WAL writes).
    let db = VecLite::open_with(
        &path,
        veclite::OpenOptions::new()
            .read_only(true)
            .read_only_ignore_wal(true),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(db.list_collections().is_empty());
    drop(db);
    cleanup(&path);
}

/// Task 1.6 / 2.4: damage beyond the committed TOC does not affect opening in
/// either mode — both read the committed state (STG-003).
#[test]
fn damaged_tail_opens_in_both_modes() {
    let path = db_path("damaged");
    cleanup(&path);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(2))
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![1.0, 2.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    }
    // Append garbage past the committed TOC (an uncommitted torn tail).
    {
        let mut f = OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap_or_else(|e| panic!("{e}"));
        std::io::Write::write_all(&mut f, &[0xABu8; 500]).unwrap_or_else(|e| panic!("{e}"));
        f.sync_all().unwrap_or_else(|e| panic!("{e}"));
    }
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}")); // read-write
        assert_eq!(
            db.collection("docs")
                .unwrap_or_else(|e| panic!("{e}"))
                .len(),
            1
        );
    }
    let db = VecLite::open_with(&path, veclite::OpenOptions::new().read_only(true))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        db.collection("docs")
            .unwrap_or_else(|e| panic!("{e}"))
            .len(),
        1
    );
    drop(db);
    cleanup(&path);
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).unwrap_or_else(|e| panic!("{e}")).len()
}

/// Task 2.2 / STG-071: after deletes, `vacuum()` shrinks the file in place, the
/// pager stays live for further writes, and reopening preserves the compacted
/// data.
#[test]
fn vacuum_shrinks_file_and_pager_survives() {
    let path = db_path("vacuum");
    cleanup(&path);
    let mut rng = Rng::new(42);
    {
        // Disable auto-vacuum so the explicit vacuum() is what reclaims space
        // (otherwise deleting half crosses the 0.25 default and the second
        // checkpoint already compacts — see auto_vacuum_escalates_at_threshold).
        let db = VecLite::open_with(
            &path,
            veclite::OpenOptions::new().auto_vacuum_threshold(0.0),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(8))
            .unwrap_or_else(|e| panic!("{e}"));
        for i in 0..2000u32 {
            let v: Vec<f32> = (0..8).map(|_| rng.next_f32()).collect();
            c.upsert(Point::new(format!("k{i}"), v))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        // Delete half, then checkpoint again — the file grows (append-only).
        for i in 0..1000u32 {
            c.delete(&format!("k{i}")).unwrap_or_else(|e| panic!("{e}"));
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        let before = file_len(&path);

        db.vacuum().unwrap_or_else(|e| panic!("{e}"));
        let after = file_len(&path);
        assert!(
            after < before,
            "vacuum should shrink the file: before={before}, after={after}"
        );
        assert_eq!(c.len(), 1000);

        // The pager is still live after the in-place close→rename→reopen swap.
        for i in 2000..2100u32 {
            let v: Vec<f32> = (0..8).map(|_| rng.next_f32()).collect();
            c.upsert(Point::new(format!("k{i}"), v))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.len(), 1100);
    }
    // Reopen: compacted data intact, deletes still gone.
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 1100);
    assert!(c.get("k0").unwrap_or_else(|e| panic!("{e}")).is_none());
    assert!(c.get("k1500").unwrap_or_else(|e| panic!("{e}")).is_some());
    drop(db);
    cleanup(&path);
}

/// Task 2.4 / STG-072: a checkpoint escalates to a vacuum once a collection's
/// tombstone ratio crosses the configured threshold — shrinking the file a
/// non-escalating checkpoint would leave bloated.
#[test]
fn auto_vacuum_escalates_at_threshold() {
    let on = db_path("autovac-on"); // threshold 0.25, ratio 0.30 crosses it
    let off = db_path("autovac-off"); // threshold 0.90, ratio 0.30 does not
    cleanup(&on);
    cleanup(&off);

    // Populate 100, checkpoint, delete 30 (ratio 0.30), checkpoint; measure the
    // file before the drop-time checkpoint muddies it. Returns the file size.
    fn churn_and_measure(path: &Path, threshold: f32) -> u64 {
        let db = VecLite::open_with(
            path,
            veclite::OpenOptions::new().auto_vacuum_threshold(threshold),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts(4))
            .unwrap_or_else(|e| panic!("{e}"));
        let mut rng = Rng::new(7);
        for i in 0..100u32 {
            let v: Vec<f32> = (0..4).map(|_| rng.next_f32()).collect();
            c.upsert(Point::new(format!("k{i}"), v))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        for i in 0..30u32 {
            c.delete(&format!("k{i}")).unwrap_or_else(|e| panic!("{e}"));
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.len(), 70);
        let len = file_len(path);
        db.__test_simulate_crash(); // skip the drop checkpoint so `len` is final
        len
    }

    let with_autovac = churn_and_measure(&on, 0.25);
    let without = churn_and_measure(&off, 0.90);
    assert!(
        with_autovac < without,
        "auto-vacuum should shrink the file: on={with_autovac}, off={without}"
    );
    cleanup(&on);
    cleanup(&off);
}

/// Task 2.1 / STG-070: `snapshot` writes a standalone compacted copy that opens
/// independently with a consistent point-in-time state, while a concurrent
/// writer keeps upserting without failure.
#[test]
fn snapshot_is_standalone_and_consistent_under_writes() {
    let path = db_path("snap-src");
    let snap = db_path("snap-out");
    cleanup(&path);
    cleanup(&snap);

    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db
        .create_collection("docs", opts(4))
        .unwrap_or_else(|e| panic!("{e}"));
    let mut rng = Rng::new(99);
    for i in 0..500u32 {
        let v: Vec<f32> = (0..4).map(|_| rng.next_f32()).collect();
        c.upsert(Point::new(format!("k{i}"), v))
            .unwrap_or_else(|e| panic!("{e}"));
    }

    // A concurrent writer keeps adding points during the snapshot.
    let writer_db = db.clone();
    let handle = std::thread::spawn(move || {
        let wc = writer_db
            .collection("docs")
            .unwrap_or_else(|e| panic!("{e}"));
        let mut r = Rng::new(1234);
        for i in 500..1500u32 {
            let v: Vec<f32> = (0..4).map(|_| r.next_f32()).collect();
            wc.upsert(Point::new(format!("k{i}"), v))
                .unwrap_or_else(|e| panic!("{e}"));
        }
    });

    db.snapshot(&snap).unwrap_or_else(|e| panic!("{e}"));
    assert!(handle.join().is_ok(), "concurrent writer must not fail");

    // The original received every write.
    assert_eq!(c.len(), 1500);
    drop(db);

    // The snapshot opens standalone with a consistent point-in-time subset
    // (>= the 500 present before the snapshot, <= the full 1500).
    let sdb = VecLite::open(&snap).unwrap_or_else(|e| panic!("{e}"));
    let sc = sdb.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    let n = sc.len();
    assert!((500..=1500).contains(&n), "snapshot count {n} out of range");
    assert!(sc.get("k0").unwrap_or_else(|e| panic!("{e}")).is_some());
    let hits = sc
        .search(&[0.1, 0.2, 0.3, 0.4], 5)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits.len(), 5);
    drop(sdb);

    cleanup(&path);
    cleanup(&snap);
}

/// A checkpoint that has nothing to persist must not grow the file (SPEC-002
/// STG-052). The reuse path exists — the pager references committed segments
/// in place — but was reachable only on the mmap tier, so an ordinary
/// collection resealed and rewrote every segment on every checkpoint, and a
/// database that was merely opened and closed grew by a full copy each time.
#[test]
fn idle_checkpoints_do_not_grow_the_file() {
    let path = db_path("idle-checkpoint");
    cleanup(&path);

    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);

    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("v", CollectionOptions::new(8, Metric::Cosine))
            .unwrap_or_else(|e| panic!("{e}"));
        for i in 0..500u32 {
            let v: Vec<f32> = (0..8)
                .map(|j| f32::from(((i * 7 + j) % 11) as u8) + 1.0)
                .collect();
            c.upsert(Point::new(format!("v{i}"), v))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        let after_first = size(&path);

        // Nothing is written between these, so there is nothing to seal.
        for _ in 0..5 {
            db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        }
        assert_eq!(
            size(&path),
            after_first,
            "five no-op checkpoints grew the file"
        );
    }

    // Closing checkpoints too, so a reopen cycle that writes nothing is the
    // same property seen from the outside — and it is what every process that
    // merely reads the database does on every run.
    let after_close = size(&path);
    for _ in 0..5 {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        drop(db);
    }
    assert_eq!(
        size(&path),
        after_close,
        "five open/close cycles with no writes grew the file"
    );

    cleanup(&path);
}

/// End-to-end cover for the path segment reuse makes riskiest: seal, delete,
/// vacuum, checkpoint again, reopen. A vacuum rewrites the file wholesale, so
/// any surviving carried-forward ref would point the next TOC at bytes that no
/// longer mean anything (STG-071).
///
/// Two things guard that today, and only the first is load-bearing: the vacuum
/// rebase sets `dirty`, which short-circuits `clean_reuse` before it consults
/// the sealed refs, and it also clears those refs. Verified by deleting the
/// second guard — this test still passes — so treat it as defence in depth, not
/// as the thing under test here. What this test does verify is that the whole
/// sequence round-trips to the exact surviving point set.
#[test]
fn vacuum_invalidates_carried_forward_segment_refs() {
    let path = db_path("vacuum-invalidates-reuse");
    cleanup(&path);

    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("v", CollectionOptions::new(4, Metric::Cosine))
            .unwrap_or_else(|e| panic!("{e}"));
        for i in 0..200u32 {
            let v: Vec<f32> = (0..4)
                .map(|j| f32::from(((i * 3 + j) % 7) as u8) + 1.0)
                .collect();
            c.upsert(Point::new(format!("v{i}"), v))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        // Seal, so the collection is carrying refs into the pre-vacuum file.
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        for i in 0..100u32 {
            c.delete(&format!("v{i}")).unwrap_or_else(|e| panic!("{e}"));
        }
        db.vacuum().unwrap_or_else(|e| panic!("{e}"));
        // The refs recorded before the vacuum are now meaningless; a checkpoint
        // that reused them would commit a TOC pointing into the old layout.
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    }

    // The file must still be readable and hold exactly the surviving points.
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let c = db.collection("v").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 100);
    for i in 100..200u32 {
        let id = format!("v{i}");
        assert!(
            c.get(&id).unwrap_or_else(|e| panic!("{e}")).is_some(),
            "{id} should have survived the vacuum"
        );
    }
    for i in 0..100u32 {
        let id = format!("v{i}");
        assert!(
            c.get(&id).unwrap_or_else(|e| panic!("{e}")).is_none(),
            "{id} was deleted before the vacuum"
        );
    }
    drop(db);
    cleanup(&path);
}

/// Carry-forward makes a mixed commit the common case: some collections
/// referenced in place, others freshly sealed in the same generation. That mix
/// existed before only for mmap'd collections, so it is worth pinning that the
/// committed TOC stays self-consistent — the untouched collection must not be
/// disturbed by its neighbour's reseal, and vice versa.
#[test]
fn a_commit_mixing_carried_forward_and_resealed_collections_stays_consistent() {
    let path = db_path("mixed-carry-forward");
    cleanup(&path);

    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let stable = db
            .create_collection("stable", CollectionOptions::new(4, Metric::Cosine))
            .unwrap_or_else(|e| panic!("{e}"));
        // Euclidean here so the stored vector is the one written: a cosine
        // collection normalizes on ingest (CORE-014), which would obscure the
        // freshness check below.
        let churn = db
            .create_collection("churn", CollectionOptions::new(4, Metric::Euclidean))
            .unwrap_or_else(|e| panic!("{e}"));
        for i in 0..100u32 {
            let v: Vec<f32> = (0..4)
                .map(|j| f32::from(((i + j) % 6) as u8) + 1.0)
                .collect();
            stable
                .upsert(Point::new(format!("s{i}"), v.clone()))
                .unwrap_or_else(|e| panic!("{e}"));
            churn
                .upsert(Point::new(format!("c{i}"), v))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));

        // Only `churn` moves from here on, so every later checkpoint carries
        // `stable` forward while resealing `churn`.
        for round in 0..3u32 {
            for i in 0..20u32 {
                let v = vec![
                    f32::from((round + 1) as u8),
                    2.0,
                    3.0,
                    f32::from((i % 4) as u8) + 1.0,
                ];
                churn
                    .upsert(Point::new(format!("c{i}"), v))
                    .unwrap_or_else(|e| panic!("{e}"));
            }
            db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        }
    }

    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    let stable = db.collection("stable").unwrap_or_else(|e| panic!("{e}"));
    let churn = db.collection("churn").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(stable.len(), 100, "carried-forward collection lost points");
    assert_eq!(churn.len(), 100, "resealed collection lost points");
    for i in 0..100u32 {
        assert!(
            stable
                .get(&format!("s{i}"))
                .unwrap_or_else(|e| panic!("{e}"))
                .is_some(),
            "s{i} missing from the carried-forward collection"
        );
    }
    // The last round's values are the ones that survived in the churned one.
    let p = churn
        .get("c0")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("c0 missing"));
    assert!(
        (p.vector[0] - 3.0).abs() < 1e-6,
        "stale value: {:?}",
        p.vector
    );

    drop(db);
    cleanup(&path);
}
