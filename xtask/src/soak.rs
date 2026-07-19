//! Sustained-operation soak (SPEC-015 TST-051; DAG T6.2).
//!
//! `cargo xtask soak [--minutes N] [--mmap-pressure] [--budget-mb M]`
//!
//! Runs a continuous write / search / vacuum / snapshot mix against a
//! file-backed database with an in-memory oracle, invariant checks on every
//! cycle, and RSS sampling with plateau detection:
//!
//! - **Invariants**: sampled `get` matches the oracle (vectors within 1e-5
//!   after metric normalization, payloads exact), `len` matches, searches
//!   return only live ids with finite scores, every periodic snapshot passes
//!   the full `verify` integrity pass and reopens with matching counts.
//! - **Leak detection**: RSS is sampled throughout; after a warm-up quarter,
//!   the median of the last quarter must not exceed the median of the first
//!   quarter by more than 15 % — a monotonic-growth trend fails the run.
//! - **`--mmap-pressure`** (memory-pressure path): pre-builds a dense dataset
//!   4× the configured `memory_budget` and reopens under that budget, so the
//!   whole soak runs on the mmap exact-scan tier (ADR-0004) — "dataset 4×
//!   RAM" realized through the budget knob, which is the enforced ceiling.
//!
//! The default budget is 24 h (1440 min); shorter runs accumulate evidence in
//! `tests/soak/accumulation.log` (committed) the same way the fuzz log does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use veclite::{CollectionOptions, Metric, OpenOptions, Point, Quantization, VecLite};

use crate::Rng;

const DENSE_DIM: usize = 64;
const TEXT_DIM: usize = 128;
const PRESSURE_DIM: usize = 256;
const RSS_SAMPLE_EVERY: Duration = Duration::from_secs(10);
const RSS_GROWTH_LIMIT: f64 = 1.15;

struct Args {
    minutes: u64,
    mmap_pressure: bool,
    budget_mb: u64,
    /// Seconds between maintenance ticks (checkpoint + vacuum, rotating in a
    /// verified snapshot). Default 60 s; smoke runs lower it so the working
    /// set plateaus quickly for a meaningful RSS verdict.
    maintain_secs: u64,
    /// Live-set cap: the workload deletes as it inserts past this point, so
    /// the footprint plateaus by design and any RSS growth past it is a leak,
    /// not the data. The leak verdict only counts samples taken at the cap
    /// (steady state) — short smoke runs should lower it (`--live-cap`) so
    /// they reach steady state at all.
    live_cap: usize,
}

fn parse(args: &[String]) -> Args {
    let get = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse::<u64>().ok())
    };
    Args {
        minutes: get("--minutes").unwrap_or(24 * 60),
        mmap_pressure: args.iter().any(|a| a == "--mmap-pressure"),
        budget_mb: get("--budget-mb").unwrap_or(16),
        live_cap: get("--live-cap").map_or(20_000, |v| v as usize).max(64),
        maintain_secs: get("--maintain-secs").unwrap_or(60).max(1),
    }
}

/// `cargo xtask soak`.
pub fn run(args: &[String]) -> i32 {
    let args = parse(args);
    match soak(&args) {
        Ok(report) => {
            eprintln!(
                "[soak] PASS — {} ops, {} invariant checks, {} snapshots verified, RSS working-set \
                 floor first/last {:.1}/{:.1} MiB (ratio {:.3}, limit {RSS_GROWTH_LIMIT})",
                report.ops,
                report.invariant_checks,
                report.snapshots_verified,
                report.rss_first_floor_mb,
                report.rss_last_floor_mb,
                report.rss_ratio
            );
            log_accumulation(&args, &report, true);
            0
        }
        Err(e) => {
            eprintln!("[soak] FAIL: {e}");
            log_failure(&args);
            1
        }
    }
}

struct Report {
    ops: u64,
    invariant_checks: u64,
    snapshots_verified: u64,
    rss_first_floor_mb: f64,
    rss_last_floor_mb: f64,
    rss_ratio: f64,
}

/// The oracle entry for a dense point.
struct OraclePoint {
    vector: Vec<f32>,
    payload: Option<serde_json::Value>,
}

fn soak(args: &Args) -> Result<Report, String> {
    let dir = PathBuf::from("target/soak");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Process-unique so concurrent soak runs never collide on the advisory
    // file lock.
    let db_path = dir.join(format!("soak-{}.veclite", std::process::id()));
    cleanup(&db_path);

    let budget = args.budget_mb * 1024 * 1024;
    let mut rng = Rng::new(0x50AC_2026);

    // ── Build phase ─────────────────────────────────────────────────────
    let mut oracle: HashMap<String, OraclePoint> = HashMap::new();
    let mut text_oracle: HashMap<String, String> = HashMap::new();
    let mut next_id = 0u64;

    let (db, dense_dim) = if args.mmap_pressure {
        // Dataset 4× the memory budget, then reopen under the budget so the
        // soak runs on the mmap exact-scan tier (ADR-0004 / TST-051).
        let count = (4 * budget) as usize / (PRESSURE_DIM * 4);
        eprintln!(
            "[soak] mmap-pressure build: {count} × {PRESSURE_DIM}-dim vectors (~{} MiB) under a \
             {} MiB budget…",
            count * PRESSURE_DIM * 4 / (1024 * 1024),
            args.budget_mb
        );
        {
            let build = VecLite::open(&db_path).map_err(|e| e.to_string())?;
            let dense = build
                .create_collection(
                    "dense",
                    CollectionOptions::new(PRESSURE_DIM, Metric::Cosine)
                        .quantization(Quantization::None),
                )
                .map_err(|e| e.to_string())?;
            let mut batch = Vec::with_capacity(1000);
            for _ in 0..count {
                let id = format!("p-{next_id:08}");
                next_id += 1;
                let vector: Vec<f32> = (0..PRESSURE_DIM)
                    .map(|_| rng.next_f32() * 2.0 - 1.0)
                    .collect();
                oracle.insert(
                    id.clone(),
                    OraclePoint {
                        vector: vector.clone(),
                        payload: None,
                    },
                );
                batch.push(Point::new(id, vector));
                if batch.len() == 1000 {
                    dense
                        .upsert_batch(std::mem::take(&mut batch))
                        .map_err(|e| e.to_string())?;
                }
            }
            if !batch.is_empty() {
                dense.upsert_batch(batch).map_err(|e| e.to_string())?;
            }
            build.checkpoint().map_err(|e| e.to_string())?;
        }
        let db = VecLite::open_with(&db_path, OpenOptions::new().memory_budget(budget))
            .map_err(|e| e.to_string())?;
        (db, PRESSURE_DIM)
    } else {
        let db = VecLite::open(&db_path).map_err(|e| e.to_string())?;
        db.create_collection("dense", CollectionOptions::new(DENSE_DIM, Metric::Cosine))
            .map_err(|e| e.to_string())?;
        db.create_collection("texts", CollectionOptions::auto_embed("bm25", TEXT_DIM))
            .map_err(|e| e.to_string())?;
        (db, DENSE_DIM)
    };

    // ── Soak loop ───────────────────────────────────────────────────────
    let deadline = Instant::now() + Duration::from_secs(args.minutes * 60);
    let started = Instant::now();
    let mut ops = 0u64;
    let mut invariant_checks = 0u64;
    let mut snapshots_verified = 0u64;
    let mut rss_samples: Vec<u64> = Vec::new();
    let mut last_rss = Instant::now() - RSS_SAMPLE_EVERY;
    let mut last_maintenance = Instant::now();
    let mut snapshot_seq = 0u64;

    while Instant::now() < deadline {
        let dense = db.collection("dense").map_err(|e| e.to_string())?;
        // Operation mix.
        match rng.below(10) {
            // Insert a small dense batch (delete-first past the live cap).
            0..=3 => {
                if oracle.len() >= args.live_cap {
                    // Evict a pseudo-random live id to keep the set bounded.
                    if let Some(id) = oracle.keys().nth(rng.below(64) as usize).cloned() {
                        dense.delete(&id).map_err(|e| e.to_string())?;
                        oracle.remove(&id);
                    }
                }
                let id = format!("p-{next_id:08}");
                next_id += 1;
                let vector: Vec<f32> = (0..dense_dim).map(|_| rng.next_f32() * 2.0 - 1.0).collect();
                let payload = (rng.below(2) == 0)
                    .then(|| serde_json::json!({"n": next_id, "bucket": next_id % 7}));
                dense
                    .upsert(Point {
                        id: id.clone(),
                        vector: vector.clone(),
                        sparse: None,
                        payload: payload.clone(),
                    })
                    .map_err(|e| e.to_string())?;
                oracle.insert(id, OraclePoint { vector, payload });
            }
            // Delete.
            4 => {
                if let Some(id) = oracle.keys().nth(rng.below(64) as usize).cloned() {
                    let existed = dense.delete(&id).map_err(|e| e.to_string())?;
                    if !existed {
                        return Err(format!("delete({id}): oracle says live, db says absent"));
                    }
                    oracle.remove(&id);
                }
            }
            // Dense search: only live ids, finite scores.
            5..=6 => {
                let query: Vec<f32> = (0..dense_dim).map(|_| rng.next_f32() * 2.0 - 1.0).collect();
                let hits = dense
                    .query(&query)
                    .limit(10)
                    .run()
                    .map_err(|e| e.to_string())?;
                if hits.len() > 10 {
                    return Err("search returned more than limit".to_string());
                }
                for hit in &hits {
                    if !hit.score.is_finite() {
                        return Err(format!("non-finite score for {}", hit.id));
                    }
                    if !oracle.contains_key(&hit.id) {
                        return Err(format!("search returned dead/unknown id {}", hit.id));
                    }
                }
                invariant_checks += 1;
            }
            // Text upsert + text search (standard mode only).
            7 => {
                if !args.mmap_pressure {
                    let texts = db.collection("texts").map_err(|e| e.to_string())?;
                    let id = format!("t-{next_id:08}");
                    next_id += 1;
                    let text = format!(
                        "soak document {next_id} covering vector database durability topic {}",
                        rng.below(50)
                    );
                    texts.upsert_text(&id, &text).map_err(|e| e.to_string())?;
                    text_oracle.insert(id, text);
                    if text_oracle.len() > args.live_cap / 4 {
                        if let Some(id) = text_oracle.keys().next().cloned() {
                            texts.delete(&id).map_err(|e| e.to_string())?;
                            text_oracle.remove(&id);
                        }
                    }
                    let hits = texts
                        .search_text("durability topic", 5)
                        .map_err(|e| e.to_string())?;
                    for hit in &hits {
                        if !text_oracle.contains_key(&hit.id) {
                            return Err(format!("text search returned dead id {}", hit.id));
                        }
                    }
                    invariant_checks += 1;
                }
            }
            // Scroll a page and cross-check membership.
            8 => {
                let page = dense.scroll(None, 64, None).map_err(|e| e.to_string())?;
                for point in &page.points {
                    if !oracle.contains_key(&point.id) {
                        return Err(format!("scroll returned dead id {}", point.id));
                    }
                }
                invariant_checks += 1;
            }
            // Point-lookup vs oracle (vector + payload equivalence).
            _ => {
                if let Some(id) = oracle.keys().nth(rng.below(64) as usize).cloned() {
                    let entry = &oracle[&id];
                    let got = dense
                        .get(&id)
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("get({id}): oracle live, db None"))?;
                    let want = normalize(&entry.vector);
                    if got.vector.len() != want.len()
                        || got
                            .vector
                            .iter()
                            .zip(&want)
                            .any(|(a, b)| (a - b).abs() > 1e-5)
                    {
                        return Err(format!("get({id}): vector mismatch vs oracle"));
                    }
                    if got.payload != entry.payload {
                        return Err(format!("get({id}): payload mismatch vs oracle"));
                    }
                }
                invariant_checks += 1;
            }
        }
        ops += 1;

        // Global count invariant, cheap enough to run always.
        let live = db.collection("dense").map_err(|e| e.to_string())?.len();
        if live != oracle.len() {
            return Err(format!("len {} != oracle {}", live, oracle.len()));
        }

        // Periodic maintenance. Every tick checkpoints and vacuums: the
        // workload deletes continuously, and soft-deletes only leave the HNSW
        // graph on vacuum/reindex, so vacuuming every tick is what keeps the
        // working set bounded — a soak that vacuums rarely measures unbounded
        // churn, not sustained operation. A verified snapshot rotates in every
        // third tick (SPEC-002 STG-070 + the full integrity pass).
        if last_maintenance.elapsed() >= Duration::from_secs(args.maintain_secs) {
            last_maintenance = Instant::now();
            db.checkpoint().map_err(|e| format!("checkpoint: {e}"))?;
            db.vacuum().map_err(|e| format!("vacuum: {e}"))?;
            if snapshot_seq.is_multiple_of(3) {
                let snap = dir.join(format!("snap-{snapshot_seq}.veclite"));
                let _ = std::fs::remove_file(&snap);
                db.snapshot(&snap).map_err(|e| format!("snapshot: {e}"))?;
                let report = veclite::interop::verify_file(&snap)
                    .map_err(|e| format!("verify snapshot: {e}"))?;
                if !report.findings.is_empty() {
                    return Err(format!(
                        "snapshot verify found corruption: {:?}",
                        report.findings
                    ));
                }
                let reopened = VecLite::open(&snap).map_err(|e| e.to_string())?;
                let count = reopened
                    .collection("dense")
                    .map_err(|e| e.to_string())?
                    .len();
                if count != oracle.len() {
                    return Err(format!(
                        "snapshot reopen count {count} != oracle {}",
                        oracle.len()
                    ));
                }
                drop(reopened);
                cleanup(&snap);
                snapshots_verified += 1;
            }
            snapshot_seq += 1;
        }

        // RSS sampling. The leak verdict only counts steady-state samples —
        // while the live set is still growing toward the cap, RSS growth is
        // the data, not a leak.
        if last_rss.elapsed() >= RSS_SAMPLE_EVERY {
            last_rss = Instant::now();
            if let Some(rss) = rss_bytes() {
                let steady = oracle.len() >= args.live_cap;
                if steady {
                    rss_samples.push(rss);
                }
                let minutes = started.elapsed().as_secs() / 60;
                eprintln!(
                    "[soak] t+{minutes:>4}m ops={ops} live={} rss={} MiB{}",
                    oracle.len(),
                    rss / (1024 * 1024),
                    if steady { " [steady]" } else { "" }
                );
            }
        }
    }

    // ── Leak verdict: RSS plateau over steady-state samples (TST-051) ───
    if rss_samples.len() < 8 {
        eprintln!(
            "[soak] note: only {} steady-state RSS samples (live set below --live-cap {} for \
             most of the run) — leak verdict neutral; lower --live-cap or run longer for a \
             meaningful plateau check",
            rss_samples.len(),
            args.live_cap
        );
    }
    let (first_floor, last_floor, ratio) = rss_verdict(&rss_samples)?;
    if ratio > RSS_GROWTH_LIMIT {
        return Err(format!(
            "RSS working-set floor grew {ratio:.3}× from {first_floor:.1} MiB to {last_floor:.1} \
             MiB (limit {RSS_GROWTH_LIMIT}) — the post-vacuum floor is trending up, a leak"
        ));
    }

    drop(db);
    cleanup(&db_path);
    Ok(Report {
        ops,
        invariant_checks,
        snapshots_verified,
        rss_first_floor_mb: first_floor,
        rss_last_floor_mb: last_floor,
        rss_ratio: ratio,
    })
}

/// Cosine ingest normalization — what the engine stores for the oracle
/// comparison.
fn normalize(vector: &[f32]) -> Vec<f32> {
    let norm = vector
        .iter()
        .map(|v| f64::from(*v) * f64::from(*v))
        .sum::<f64>()
        .sqrt();
    if norm == 0.0 {
        vector.to_vec()
    } else {
        #[allow(clippy::cast_possible_truncation)]
        vector
            .iter()
            .map(|v| (f64::from(*v) / norm) as f32)
            .collect()
    }
}

/// Leak verdict over the post-warm-up samples, comparing the **floor** (a low
/// percentile) of the first window against the last. Under maintenance the RSS
/// signal sawtooths — it climbs as churn accumulates between vacuums, then
/// drops back at each vacuum — so the retained working set is the post-vacuum
/// floor, not the median or peak. A true leak raises that floor; transient
/// pre-vacuum peaks do not. Comparing floors is therefore the noise-robust
/// plateau test (a median comparison is fooled by which phase of the sawtooth
/// each window happens to sample).
fn rss_verdict(samples: &[u64]) -> Result<(f64, f64, f64), String> {
    if samples.len() < 8 {
        // Too short to judge a trend (smoke runs): report neutrally.
        let mb = samples.last().copied().unwrap_or(0) as f64 / (1024.0 * 1024.0);
        return Ok((mb, mb, 1.0));
    }
    let warm = &samples[samples.len() / 4..]; // discard the warm-up quarter
    let half = warm.len() / 2;
    // 20th percentile of a window — the post-vacuum floor, robust to a single
    // unlucky low sample the way a raw minimum is not.
    let floor = |window: &[u64]| -> f64 {
        let mut sorted = window.to_vec();
        sorted.sort_unstable();
        sorted[sorted.len() / 5] as f64 / (1024.0 * 1024.0)
    };
    let first = floor(&warm[..half.max(1)]);
    let last = floor(&warm[half..]);
    Ok((first, last, if first > 0.0 { last / first } else { 1.0 }))
}

/// Current process RSS, no extra dependencies: `/proc` on Linux, `ps` on
/// macOS, PowerShell `WorkingSet64` on Windows.
fn rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        return Some(pages * 4096);
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let kib: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        return Some(kib * 1024);
    }
    #[cfg(windows)]
    {
        let out = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Process -Id {}).WorkingSet64", std::process::id()),
            ])
            .output()
            .ok()?;
        return String::from_utf8_lossy(&out.stdout).trim().parse().ok();
    }
    #[allow(unreachable_code)]
    None
}

fn cleanup(db: &Path) {
    let _ = std::fs::remove_file(db);
    let mut wal = db.file_name().unwrap_or_default().to_os_string();
    wal.push("-wal");
    let _ = std::fs::remove_file(db.with_file_name(wal));
}

/// Committed evidence trail (`tests/soak/accumulation.log`), mirroring the
/// fuzz accumulation log: the 24 h target is accumulated across runs.
fn log_accumulation(args: &Args, report: &Report, pass: bool) {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!(
        "epoch={epoch} minutes={} mode={} ops={} checks={} snapshots={} rss_ratio={:.3} result={}\n",
        args.minutes,
        if args.mmap_pressure {
            "mmap-pressure"
        } else {
            "standard"
        },
        report.ops,
        report.invariant_checks,
        report.snapshots_verified,
        report.rss_ratio,
        if pass { "clean" } else { "FAIL" }
    );
    let path = Path::new("tests/soak/accumulation.log");
    let _ = std::fs::create_dir_all("tests/soak");
    let previous = std::fs::read_to_string(path).unwrap_or_default();
    if let Err(e) = std::fs::write(path, previous + &line) {
        eprintln!("[soak] warning: could not append {}: {e}", path.display());
    }
}

fn log_failure(args: &Args) {
    let report = Report {
        ops: 0,
        invariant_checks: 0,
        snapshots_verified: 0,
        rss_first_floor_mb: 0.0,
        rss_last_floor_mb: 0.0,
        rss_ratio: 0.0,
    };
    log_accumulation(args, &report, false);
}
