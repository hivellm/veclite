//! Fuzzing orchestration (SPEC-015 TST-050; DAG T6.1).
//!
//! Two subcommands:
//!
//! - `cargo xtask fuzz-seed` — materialize the deterministic seed corpus
//!   (valid artifacts per parser, built by `veclite::fuzz_api::seed_corpus`)
//!   into `fuzz/corpus/<target>/seed-NNN`. Idempotent; seeds are committed.
//! - `cargo xtask fuzz [--seconds N] [--target <name>]` — run every
//!   cargo-fuzz target (nightly + libFuzzer) for a time budget each, then
//!   append the outcome to `fuzz/accumulation.log` — the committed evidence
//!   trail for the 72 h pre-1.0 accumulation gate. Any crash artifact fails
//!   the run and is left in `fuzz/artifacts/<target>/` for triage; once
//!   fixed, the reproducer is committed under `fuzz/regressions/<target>/`
//!   where the stable `fuzz_regression` test replays it forever.
//!
//! The coverage-guided runs need `cargo +nightly fuzz`; the committed corpus
//! regression runs on stable in the normal gate (`cargo test --all-features`,
//! test `fuzz_regression`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// `cargo xtask fuzz-seed`.
pub fn run_seed(_args: &[String]) -> i32 {
    let seeds = match veclite::fuzz_api::seed_corpus() {
        Ok(seeds) => seeds,
        Err(e) => {
            eprintln!("[fuzz-seed] building seeds: {e}");
            return 1;
        }
    };
    let mut written = 0usize;
    for (target, inputs) in seeds {
        let dir = PathBuf::from("fuzz/corpus").join(target);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("[fuzz-seed] mkdir {}: {e}", dir.display());
            return 1;
        }
        for (i, bytes) in inputs.iter().enumerate() {
            let path = dir.join(format!("seed-{i:03}"));
            if let Err(e) = std::fs::write(&path, bytes) {
                eprintln!("[fuzz-seed] write {}: {e}", path.display());
                return 1;
            }
            written += 1;
        }
    }
    eprintln!("[fuzz-seed] wrote {written} seed inputs under fuzz/corpus/");
    0
}

/// `cargo xtask fuzz [--seconds N] [--target <name>] [--docker|--native]`.
///
/// Coverage-guided libFuzzer needs SanitizerCoverage section symbols the MSVC
/// linker does not synthesize, so on Windows the runs happen inside a
/// `rustlang/rust:nightly` container by default (`--native` forces a local
/// attempt); on Linux/macOS they run natively. Either way the corpus,
/// artifacts, and accumulation log land in the working tree.
pub fn run_fuzz(args: &[String]) -> i32 {
    let seconds: u64 = args
        .iter()
        .position(|a| a == "--seconds")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let only: Option<&String> = args
        .iter()
        .position(|a| a == "--target")
        .and_then(|i| args.get(i + 1));
    let docker = if args.iter().any(|a| a == "--native") {
        false
    } else {
        args.iter().any(|a| a == "--docker") || cfg!(windows)
    };

    // Seeds anchor coverage — refuse to fuzz an empty corpus.
    if !Path::new("fuzz/corpus").exists() {
        eprintln!("[fuzz] fuzz/corpus missing — run `cargo xtask fuzz-seed` first");
        return 1;
    }
    if !docker {
        let probe = Command::new("cargo")
            .args(["+nightly", "fuzz", "--version"])
            .output();
        if !matches!(probe, Ok(ref out) if out.status.success()) {
            eprintln!(
                "[fuzz] `cargo +nightly fuzz` unavailable — install with `cargo install \
                 cargo-fuzz` and `rustup toolchain install nightly`"
            );
            return 1;
        }
    }

    let libfuzzer_args = [
        format!("-max_total_time={seconds}"),
        "-timeout=30".to_string(),
        "-rss_limit_mb=4096".to_string(),
        "-print_final_stats=1".to_string(),
    ];

    let mut failed = Vec::new();
    for target in veclite::fuzz_api::TARGETS {
        if only.is_some_and(|t| t != target) {
            continue;
        }
        eprintln!(
            "[fuzz] {target}: {seconds}s budget ({})…",
            if docker { "docker" } else { "native" }
        );
        let status = if docker {
            let repo = std::env::current_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            // Named volume over /usr/local/cargo persists the cargo-fuzz
            // install + registry across runs; fuzz/target volume keeps the
            // instrumented build cache off the Windows bind mount (fast).
            let script = format!(
                "command -v cargo-fuzz >/dev/null || cargo install cargo-fuzz --locked; \
                 cargo fuzz run {target} -- {}",
                libfuzzer_args.join(" ")
            );
            Command::new("docker")
                .args([
                    "run",
                    "--rm",
                    "-v",
                    &format!("{repo}:/work"),
                    "-v",
                    "veclite-fuzz-cargo:/usr/local/cargo",
                    "-v",
                    "veclite-fuzz-target:/work/fuzz/target",
                    "-w",
                    "/work",
                    "rustlang/rust:nightly",
                    "bash",
                    "-ceu",
                    &script,
                ])
                .status()
        } else {
            Command::new("cargo")
                .args(["+nightly", "fuzz", "run", target, "--"])
                .args(&libfuzzer_args)
                .status()
        };
        let clean = matches!(status, Ok(s) if s.success());
        // Distinguish a real finding (libFuzzer wrote a reproducer) from an
        // environment/build failure — only the former is a crash in the log.
        let has_artifact = std::fs::read_dir(format!("fuzz/artifacts/{target}"))
            .map(|entries| entries.flatten().any(|e| e.path().is_file()))
            .unwrap_or(false);
        let outcome = if clean {
            "clean"
        } else if has_artifact {
            "CRASH"
        } else {
            "error"
        };
        log_accumulation(target, seconds, outcome);
        if !clean {
            eprintln!(
                "[fuzz] {target} FAILED ({outcome}) — a reproducer, if any, is under \
                 fuzz/artifacts/{target}/ (triage, fix, then commit it to \
                 fuzz/regressions/{target}/)"
            );
            failed.push(target);
        }
    }
    if failed.is_empty() {
        eprintln!("[fuzz] all targets clean ({seconds}s each) — logged to fuzz/accumulation.log");
        0
    } else {
        eprintln!("[fuzz] FAILED targets: {failed:?}");
        1
    }
}

/// Append one line of the committed accumulation evidence (TST-050: 72 h
/// accumulated clean before 1.0).
fn log_accumulation(target: &str, seconds: u64, outcome: &str) {
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("epoch={epoch} target={target} seconds={seconds} result={outcome}\n");
    let path = Path::new("fuzz/accumulation.log");
    let previous = std::fs::read_to_string(path).unwrap_or_default();
    if let Err(e) = std::fs::write(path, previous + &line) {
        eprintln!("[fuzz] warning: could not append {}: {e}", path.display());
    }
}
