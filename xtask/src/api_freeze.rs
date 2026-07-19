//! Rust public-API freeze gate (`cargo xtask api-freeze`), SPEC-004 §8 / accept-3
//! (API-062, the T4.1 freeze). Regenerates the `veclite` crate's public API with
//! `cargo public-api` and diffs it against the committed snapshot
//! `crates/veclite/public-api.txt`. Within 1.x the API is additive-only, so any
//! removal, rename, or signature change fails this gate; a purely additive change
//! is re-blessed with `--bless` in the same PR that adds it.
//!
//! Usage: `cargo xtask api-freeze [--bless]`.
//!
//! `cargo public-api` needs a nightly rustdoc; when neither the tool nor nightly
//! is present the gate warns and skips (exit 0) rather than blocking a build that
//! has no way to run it — the committed snapshot remains the source of truth.

use std::path::{Path, PathBuf};
use std::process::Command;

const SNAPSHOT: &str = "crates/veclite/public-api.txt";

pub fn run(args: &[String]) -> i32 {
    let bless = args.iter().any(|a| a == "--bless");

    let generated = match generate() {
        Ok(s) => s,
        Err(Skip(msg)) => {
            eprintln!("[api-freeze] SKIP — {msg}");
            eprintln!("[api-freeze] install with: cargo install cargo-public-api --locked");
            return 0;
        }
        Err(Fail(msg)) => {
            eprintln!("[api-freeze] FAIL — {msg}");
            return 1;
        }
    };

    // Canonicalize before writing so the committed snapshot is the same bytes
    // no matter which host blessed it.
    let generated = canonicalize(&generated);

    let path = repo_root().join(SNAPSHOT);
    if bless {
        if let Err(e) = std::fs::write(&path, &generated) {
            eprintln!("[api-freeze] could not write snapshot: {e}");
            return 1;
        }
        eprintln!(
            "[api-freeze] blessed {SNAPSHOT} ({} lines)",
            generated.lines().count()
        );
        return 0;
    }

    let committed = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[api-freeze] cannot read {SNAPSHOT}: {e}");
            eprintln!("[api-freeze] create it with: cargo xtask api-freeze --bless");
            return 1;
        }
    };

    let gen_lines: Vec<String> = generated.lines().map(str::to_owned).collect();
    let com_lines: Vec<String> = normalize(&committed);
    if gen_lines == com_lines {
        eprintln!(
            "[api-freeze] PASS — public API matches the frozen snapshot ({} items)",
            gen_lines.len()
        );
        return 0;
    }

    eprintln!("[api-freeze] FAIL — the public API drifted from the frozen snapshot.");
    report_diff(&com_lines, &gen_lines);
    eprintln!(
        "[api-freeze] Post-freeze (1.x) the API is additive-only (SPEC-004 API-061).\n\
         [api-freeze] Removed/changed items require a major bump. If the change is\n\
         [api-freeze] purely additive, re-bless: cargo xtask api-freeze --bless"
    );
    1
}

/// Distinguish "cannot run the tool" (skip) from "tool ran and something is
/// wrong" (fail).
enum GenErr {
    Skip(String),
    Fail(String),
}
use GenErr::{Fail, Skip};

fn generate() -> Result<String, GenErr> {
    let output = Command::new("cargo")
        .args(["public-api", "-p", "hivellm-veclite", "--simplified"])
        .output()
        .map_err(|e| Skip(format!("could not launch cargo public-api: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // `no such command: public-api` → the subcommand isn't installed.
        if stderr.contains("no such command") {
            return Err(Skip("cargo-public-api is not installed".to_owned()));
        }
        if stderr.contains("nightly") || stderr.contains("rustdoc") {
            return Err(Skip(format!(
                "cargo public-api needs a nightly rustdoc: {}",
                stderr.lines().last().unwrap_or("").trim()
            )));
        }
        return Err(Fail(format!(
            "cargo public-api exited {}: {}",
            output.status,
            stderr.lines().last().unwrap_or("").trim()
        )));
    }
    String::from_utf8(output.stdout).map_err(|e| Fail(format!("non-UTF-8 output: {e}")))
}

/// Rewrite renderings that vary by host without the API itself changing.
///
/// `std::io::Error` is re-exported from `core`, and rustdoc resolves it to
/// `core::io::error::Error` on some hosts and `std::io::error::Error` on
/// others — the CI runner and a Windows checkout disagree. Left alone, the
/// snapshot can only ever match the host it was blessed on, so a developer
/// re-blessing locally breaks CI and vice versa. Canonicalizing both sides on
/// the way in makes the snapshot host-independent.
fn canonicalize(s: &str) -> String {
    s.replace("core::io::", "std::io::")
}

/// Split into lines, tolerating a CRLF checkout of the committed snapshot and
/// host-dependent path rendering.
fn normalize(s: &str) -> Vec<String> {
    canonicalize(s)
        .lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
        .collect()
}

/// Print the added (+) and removed (−) items so a reviewer sees the surface delta
/// without leaving the log. Order-independent set difference — the snapshot is
/// already sorted by `cargo public-api`.
fn report_diff(committed: &[String], generated: &[String]) {
    let old: std::collections::BTreeSet<&str> = committed.iter().map(String::as_str).collect();
    let new: std::collections::BTreeSet<&str> = generated.iter().map(String::as_str).collect();
    for removed in old.difference(&new) {
        eprintln!("  - {removed}");
    }
    for added in new.difference(&old) {
        eprintln!("  + {added}");
    }
}

/// Repo root = the workspace dir two levels up from this crate's manifest.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
