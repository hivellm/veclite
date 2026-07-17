//! Dev-task runner (SPEC-015 §7). `cargo xtask crash` runs the crash-safety
//! gate two ways: the deterministic in-process suite at a high iteration count
//! (via `cargo test`), then a real subprocess kill-9 harness (TST-010) that
//! SIGKILLs / TerminateProcess-es a live writer at random points and verifies
//! that every acknowledged `Full`-durability commit survives the reopen with
//! zero corruption.
//!
//! Usage: `cargo xtask crash [in_process_iters] [kill_iters]`
//! (defaults: 10 000 in-process iterations, 200 kill iterations — the NFR-05
//! gate). `crash-child` is the internal driver the harness spawns.

mod api_freeze;
mod conformance;
mod coverage;
mod graduation;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use veclite::{CollectionOptions, Durability, Metric, OpenOptions, Point, Quantization, VecLite};

/// Deterministic splitmix64.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.max(1))
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

fn time_nonce() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x1234_5678)
}

fn oracle_path(db: &Path) -> PathBuf {
    let mut n = db.file_name().unwrap_or_default().to_os_string();
    n.push(".oracle");
    db.with_file_name(n)
}

fn wal_path(db: &Path) -> PathBuf {
    let mut n = db.file_name().unwrap_or_default().to_os_string();
    n.push("-wal");
    db.with_file_name(n)
}

fn cleanup(db: &Path) {
    let _ = std::fs::remove_file(db);
    let _ = std::fs::remove_file(wal_path(db));
    let _ = std::fs::remove_file(oracle_path(db));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("help");
    let code = match cmd {
        "crash" => cmd_crash(&args[2..]),
        "crash-child" => cmd_crash_child(&args[2..]),
        "conformance" => conformance::run(&args[2..]),
        "coverage" => coverage::run(&args[2..]),
        "api-freeze" => api_freeze::run(&args[2..]),
        "graduation" => graduation::run(&args[2..]),
        _ => {
            eprintln!(
                "usage: cargo xtask <crash [in_process_iters] [kill_iters] | conformance [--bless] [corpus_dir] | coverage | api-freeze [--bless] | graduation [--bless] [--skip-server] [--vectorizer <path>]>"
            );
            2
        }
    };
    std::process::exit(code);
}

fn cmd_crash(args: &[String]) -> i32 {
    let iters: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let kills: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(200);

    // 1) Deterministic in-process suite (torn WAL / torn main / bit-flip / model
    //    equivalence) at the requested iteration count, reusing the tested code.
    eprintln!("[xtask crash] in-process crash suite: {iters} iterations");
    let status = Command::new(env!("CARGO"))
        .args([
            "test",
            "-p",
            "veclite",
            "--release",
            "--test",
            "crash_safety",
        ])
        .env("VECLITE_CRASH_ITERS", iters.to_string())
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("[xtask crash] in-process suite FAILED ({s})");
            return 1;
        }
        Err(e) => {
            eprintln!("[xtask crash] could not launch cargo test: {e}");
            return 1;
        }
    }

    // 2) Real subprocess kill-9 harness (TST-010).
    eprintln!("[xtask crash] subprocess kill-9 harness: {kills} iterations");
    match kill_harness(kills) {
        Ok(()) => {
            eprintln!("[xtask crash] PASS");
            0
        }
        Err(e) => {
            eprintln!("[xtask crash] kill harness FAILED: {e}");
            1
        }
    }
}

/// Spawn a writer, kill it at a random point, reopen, and assert every acked
/// `Full` commit is present with no corruption. Repeat `kills` times.
fn kill_harness(kills: u64) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut rng = Rng::new(time_nonce());
    let tmp = std::env::temp_dir();
    for it in 0..kills {
        let db = tmp.join(format!("veclite-kill-{}-{it}.veclite", std::process::id()));
        cleanup(&db);
        let seed = rng.next_u64();
        let db_arg = db.to_str().ok_or("non-utf8 temp path")?;

        let mut child = Command::new(&exe)
            .args(["crash-child", db_arg, &seed.to_string()])
            .spawn()
            .map_err(|e| e.to_string())?;

        // Let it run, then kill it at a random point mid-workload.
        std::thread::sleep(Duration::from_millis(10 + rng.below(60)));
        child.kill().map_err(|e| e.to_string())?;
        let _ = child.wait();

        verify_after_kill(&db).map_err(|e| format!("iter {it}: {e}"))?;
        cleanup(&db);
        if it % 50 == 49 {
            eprintln!("[xtask crash]   kill iter {}/{kills} clean", it + 1);
        }
    }
    Ok(())
}

/// After a kill: the file (if the child got that far) reopens cleanly and every
/// id the child recorded as acked is present.
fn verify_after_kill(db: &Path) -> Result<(), String> {
    if !db.exists() {
        return Ok(()); // killed before it created the file — nothing acked
    }
    let acked = read_oracle(&oracle_path(db));
    let vdb = VecLite::open(db).map_err(|e| format!("reopen failed: {e}"))?;
    if acked.is_empty() {
        return Ok(());
    }
    let c = vdb
        .collection("docs")
        .map_err(|e| format!("collection missing: {e}"))?;
    for id in &acked {
        let present = c.get(id).map_err(|e| e.to_string())?.is_some();
        if !present {
            return Err(format!("acked id {id} missing after kill"));
        }
    }
    Ok(())
}

/// Read the acked-id oracle, dropping a torn final line (no trailing newline).
fn read_oracle(path: &Path) -> Vec<String> {
    let s = std::fs::read_to_string(path).unwrap_or_default();
    let mut ids: Vec<String> = s.split('\n').map(str::to_string).collect();
    if !s.ends_with('\n') {
        ids.pop(); // partial write torn by the kill
    }
    ids.into_iter().filter(|l| !l.is_empty()).collect()
}

/// Internal driver: open the db in `Full` durability and upsert sequential ids
/// forever, recording each acked id (fsync'd) to the oracle sidecar, until the
/// supervisor kills it.
fn cmd_crash_child(args: &[String]) -> i32 {
    let Some(db_raw) = args.first() else {
        return 2;
    };
    let db = PathBuf::from(db_raw);
    let seed = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1u64);
    let mut rng = Rng::new(seed);

    let vdb = match VecLite::open_with(&db, OpenOptions::new().durability(Durability::Full)) {
        Ok(d) => d,
        Err(_) => return 1,
    };
    let c = match vdb.create_collection(
        "docs",
        CollectionOptions::new(4, Metric::Euclidean).quantization(Quantization::None),
    ) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let mut oracle = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(oracle_path(&db))
    {
        Ok(f) => f,
        Err(_) => return 1,
    };

    let mut i = 0u64;
    loop {
        let id = format!("k{i}");
        let v: Vec<f32> = (0..4).map(|_| rng.next_f32()).collect();
        if c.upsert(Point::new(id.clone(), v)).is_err() {
            return 1;
        }
        // Record only after the upsert is acked (its WAL append fsync'd under
        // Full), then fsync the record: the oracle is a subset of the db.
        if writeln!(oracle, "{id}").is_err() || oracle.sync_all().is_err() {
            return 1;
        }
        i += 1;
    }
}
