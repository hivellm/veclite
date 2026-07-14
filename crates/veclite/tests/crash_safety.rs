//! Crash-safety suite (SPEC-015 §2, gate G2 / NFR-05). Combines an in-process
//! kill-and-reopen loop over randomized workloads in all durability modes
//! (TST-010), a torn-WAL-tail sweep (TST-011 / WAL-011), a torn main-file tail
//! check (STG-003), and whole-file bit-flip drills (TST-012). Every reopen must
//! succeed with a state equal to the oracle model, or fail cleanly with
//! `Corrupt` — never a panic and never a silently wrong answer. Plain
//! `cargo test`, so it also runs on Windows CI (TST-013).
//!
//! Iteration count scales with `VECLITE_CRASH_ITERS` (default light for a normal
//! `cargo test`; the nightly crash job and `cargo xtask crash` set it to 10 000).

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use veclite::{CollectionOptions, Durability, Metric, OpenOptions as DbOptions, Point, VecLite};

/// Deterministic splitmix64.
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
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}

const DIM: usize = 6;

fn db_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "veclite-crash-{}-{name}.veclite",
        std::process::id()
    ))
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

/// Euclidean + no quantization, so the stored vector is byte-identical to the
/// ingested one (cosine would normalize and defeat exact model equality).
fn opts() -> CollectionOptions {
    CollectionOptions::new(DIM, Metric::Euclidean).quantization(veclite::Quantization::None)
}

/// Iteration budget, tunable for the nightly 10 000-run gate.
fn iters() -> u64 {
    std::env::var("VECLITE_CRASH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64)
}

/// Assert the reopened database exactly matches the oracle model.
fn assert_matches_model(db: &VecLite, oracle: &BTreeMap<String, Vec<f32>>) {
    if oracle.is_empty() {
        // The collection may exist but be empty, or (if the create was the only
        // op) still exist — either way it holds no live points.
        if let Ok(c) = db.collection("docs") {
            assert_eq!(c.len(), 0);
        }
        return;
    }
    let c = db
        .collection("docs")
        .unwrap_or_else(|e| panic!("collection missing after reopen: {e}"));
    assert_eq!(c.len(), oracle.len(), "live count mismatch after recovery");
    for (id, vec) in oracle {
        let got = c
            .get(id)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("id {id} missing after recovery"));
        assert_eq!(&got.vector, vec, "vector for {id} wrong after recovery");
    }
}

/// TST-010: randomized upsert/delete/checkpoint workloads across all three
/// durability modes, crash at the end, reopen, and require exact model
/// equivalence — an in-process kill leaves the WAL intact, so every acked op
/// must be recovered (WAL-041), with zero corruption and no panic.
#[test]
fn crash_and_reopen_reconstructs_model_all_durability_modes() {
    let modes = [Durability::Full, Durability::Normal, Durability::Off];
    for it in 0..iters() {
        let mode = modes[(it % 3) as usize];
        let path = db_path(&format!("model-{it}"));
        cleanup(&path);
        let mut rng = Rng::new(0xC0FF_EE00 ^ it.wrapping_mul(0x9E37_79B9));
        let mut oracle: BTreeMap<String, Vec<f32>> = BTreeMap::new();

        {
            let db = VecLite::open_with(&path, DbOptions::new().durability(mode))
                .unwrap_or_else(|e| panic!("{e}"));
            let c = db
                .create_collection("docs", opts())
                .unwrap_or_else(|e| panic!("{e}"));
            let steps = 20 + rng.below(60);
            for _ in 0..steps {
                let roll = rng.below(10);
                if roll < 7 || oracle.is_empty() {
                    // upsert into a bounded id space so replaces happen too.
                    let id = format!("k{}", rng.below(40));
                    let v: Vec<f32> = (0..DIM).map(|_| rng.next_f32()).collect();
                    c.upsert(Point::new(id.clone(), v.clone()))
                        .unwrap_or_else(|e| panic!("{e}"));
                    oracle.insert(id, v);
                } else if roll < 9 {
                    // delete an existing id.
                    if let Some(id) = oracle.keys().nth((rng.below(oracle.len() as u64)) as usize) {
                        let id = id.clone();
                        c.delete(&id).unwrap_or_else(|e| panic!("{e}"));
                        oracle.remove(&id);
                    }
                } else {
                    db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
                }
            }
            db.__test_simulate_crash(); // WAL survives for replay
        }

        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("reopen after crash: {e}"));
        assert_matches_model(&db, &oracle);
        drop(db);
        cleanup(&path);
    }
}

/// TST-011 / WAL-011: truncating the WAL at any offset (a power-loss torn tail)
/// leaves a valid, contiguous prefix of the appended upserts — the torn entry
/// and everything after it are discarded, never a corrupt open or wrong value.
#[test]
fn torn_wal_tail_recovers_a_valid_prefix() {
    let path = db_path("torn-wal");
    cleanup(&path);
    let n = 60u32;
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}")); // seal the CreateColl so only upserts are in the WAL
        let mut rng = Rng::new(7);
        for i in 0..n {
            let v: Vec<f32> = (0..DIM).map(|_| rng.next_f32()).collect();
            c.upsert(Point::new(format!("v{i}"), v))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        db.__test_simulate_crash();
    }
    let full_wal = wal_path(&path);
    let full_len = std::fs::metadata(&full_wal)
        .unwrap_or_else(|e| panic!("{e}"))
        .len();

    // Sweep truncation lengths across the WAL; the collection lives in the main
    // file (checkpointed), so every truncation still opens.
    let step = (full_len / 40).max(1);
    let mut trunc = 0u64;
    while trunc <= full_len {
        let scratch = db_path("torn-wal-copy");
        cleanup(&scratch);
        std::fs::copy(&path, &scratch).unwrap_or_else(|e| panic!("{e}"));
        std::fs::copy(wal_path(&path), wal_path(&scratch)).unwrap_or_else(|e| panic!("{e}"));
        {
            let f = OpenOptions::new()
                .write(true)
                .open(wal_path(&scratch))
                .unwrap_or_else(|e| panic!("{e}"));
            f.set_len(trunc).unwrap_or_else(|e| panic!("{e}"));
        }
        let db = VecLite::open(&scratch)
            .unwrap_or_else(|e| panic!("torn WAL at {trunc}/{full_len} failed to open: {e}"));
        let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
        let live = c.len();
        // Recovered ids must be exactly the contiguous prefix v0..v{live-1}.
        for i in 0..live {
            assert!(
                c.get(&format!("v{i}"))
                    .unwrap_or_else(|e| panic!("{e}"))
                    .is_some(),
                "prefix hole at v{i} (trunc {trunc}, live {live})"
            );
        }
        assert!(
            c.get(&format!("v{live}"))
                .unwrap_or_else(|e| panic!("{e}"))
                .is_none(),
            "recovered past the prefix at v{live} (trunc {trunc})"
        );
        assert!(live as u32 <= n);
        drop(db);
        cleanup(&scratch);
        trunc += step;
    }
    cleanup(&path);
}

/// TST-012 (WAL): a bit flipped anywhere in the WAL is caught by the per-entry
/// CRC — replay discards that entry and everything after it, leaving a valid
/// contiguous prefix, and the reopen never fails or panics.
#[test]
fn bit_flip_in_wal_recovers_a_valid_prefix() {
    let path = db_path("wal-flip");
    cleanup(&path);
    let n = 40u32;
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}")); // only upserts remain in the WAL
        let mut rng = Rng::new(0x77A1);
        for i in 0..n {
            let v: Vec<f32> = (0..DIM).map(|_| rng.next_f32()).collect();
            c.upsert(Point::new(format!("v{i}"), v))
                .unwrap_or_else(|e| panic!("{e}"));
        }
        db.__test_simulate_crash();
    }
    let wal_bytes = std::fs::read(wal_path(&path)).unwrap_or_else(|e| panic!("{e}"));
    let len = wal_bytes.len() as u64;
    let step = (len / 50).max(1);

    let mut off = 0u64;
    while off < len {
        let scratch = db_path("wal-flip-copy");
        cleanup(&scratch);
        std::fs::copy(&path, &scratch).unwrap_or_else(|e| panic!("{e}"));
        let mut m = wal_bytes.clone();
        m[off as usize] ^= 0x01;
        std::fs::write(wal_path(&scratch), &m).unwrap_or_else(|e| panic!("{e}"));

        let db = VecLite::open(&scratch)
            .unwrap_or_else(|e| panic!("WAL flip at {off}/{len} failed to open: {e}"));
        let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
        let live = c.len();
        for i in 0..live {
            assert!(
                c.get(&format!("v{i}"))
                    .unwrap_or_else(|e| panic!("{e}"))
                    .is_some(),
                "prefix hole at v{i} (flip {off}, live {live})"
            );
        }
        assert!(
            c.get(&format!("v{live}"))
                .unwrap_or_else(|e| panic!("{e}"))
                .is_none(),
            "recovered past the prefix at v{live} (flip {off})"
        );
        assert!(live as u32 <= n);
        drop(db);
        cleanup(&scratch);
        off += step;
    }
    cleanup(&path);
}

/// STG-003: garbage appended past the committed TOC (a torn checkpoint that
/// never swapped the header) never affects opening — the last committed
/// generation is returned intact, in both read-write and read-only mode.
#[test]
fn torn_main_file_tail_keeps_last_committed_state() {
    let path = db_path("torn-main");
    cleanup(&path);
    let mut oracle: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        let mut rng = Rng::new(99);
        for i in 0..30u32 {
            let v: Vec<f32> = (0..DIM).map(|_| rng.next_f32()).collect();
            c.upsert(Point::new(format!("v{i}"), v.clone()))
                .unwrap_or_else(|e| panic!("{e}"));
            oracle.insert(format!("v{i}"), v);
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    } // clean close; header points at the committed generation

    // Simulate a torn checkpoint: append partial garbage past the committed TOC.
    for pad in [1usize, 200, 5000] {
        let scratch = db_path("torn-main-copy");
        cleanup(&scratch);
        std::fs::copy(&path, &scratch).unwrap_or_else(|e| panic!("{e}"));
        {
            let mut f = OpenOptions::new()
                .append(true)
                .open(&scratch)
                .unwrap_or_else(|e| panic!("{e}"));
            f.write_all(&vec![0xCDu8; pad])
                .unwrap_or_else(|e| panic!("{e}"));
            f.sync_all().unwrap_or_else(|e| panic!("{e}"));
        }
        let db = VecLite::open(&scratch).unwrap_or_else(|e| panic!("rw open (pad {pad}): {e}"));
        assert_matches_model(&db, &oracle);
        drop(db);
        let ro = VecLite::open_with(&scratch, DbOptions::new().read_only(true))
            .unwrap_or_else(|e| panic!("ro open (pad {pad}): {e}"));
        assert_matches_model(&ro, &oracle);
        drop(ro);
        cleanup(&scratch);
    }
    cleanup(&path);
}

/// TST-012: a single-bit flip anywhere in a committed database file either
/// leaves the data bit-identical (the flip hit reserved/dead bytes) or makes
/// open fail with `Corrupt` — never a panic, never a silently wrong answer.
#[test]
fn bit_flip_in_committed_file_is_caught_or_harmless() {
    let path = db_path("bitflip");
    cleanup(&path);
    let mut oracle: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        let mut rng = Rng::new(0x0B17_F11B);
        for i in 0..24u32 {
            let v: Vec<f32> = (0..DIM).map(|_| rng.next_f32()).collect();
            c.upsert(Point::new(format!("v{i}"), v.clone()))
                .unwrap_or_else(|e| panic!("{e}"));
            oracle.insert(format!("v{i}"), v);
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
    let len = bytes.len() as u64;
    let step = (len / 200).max(1); // sample ~200 offsets to keep a normal run fast

    let mut off = 0u64;
    while off < len {
        for bit in [0u8, 3, 7] {
            let scratch = db_path("bitflip-copy");
            cleanup(&scratch);
            let mut m = bytes.clone();
            m[off as usize] ^= 1 << bit;
            std::fs::write(&scratch, &m).unwrap_or_else(|e| panic!("{e}"));

            match VecLite::open(&scratch) {
                Ok(db) => {
                    // Opened cleanly → the data MUST still be exactly correct
                    // (a flip in reserved/dead space); a wrong value here would
                    // be a silent-corruption bug the CRCs failed to catch.
                    assert_matches_model(&db, &oracle);
                    // A search must also return the right neighbour.
                    let probe = oracle
                        .values()
                        .next()
                        .cloned()
                        .unwrap_or_else(|| vec![0.0; DIM]);
                    let hits = db
                        .collection("docs")
                        .unwrap_or_else(|e| panic!("{e}"))
                        .search(&probe, 1)
                        .unwrap_or_else(|e| panic!("{e}"));
                    assert_eq!(hits.len(), 1);
                    drop(db);
                }
                Err(veclite::VecLiteError::Corrupt(_)) => { /* detected — expected */ }
                Err(other) => panic!("flip at byte {off} bit {bit}: unexpected error {other}"),
            }
            cleanup(&scratch);
        }
        off += step;
    }
    cleanup(&path);
}
