//! Coverage gate (`cargo xtask coverage`). Runs `cargo llvm-cov` for the core
//! and FFI crates and fails if line coverage drops below the floors documented
//! in `docs/COVERAGE.md`. See that doc for why the floors are below 100 % (the
//! residual is justified-unreachable code, not untested behavior).

use std::process::Command;

use serde_json::Value;

/// Per-crate line-coverage floor: `(package, own-source-file suffix, min %)`.
/// For the FFI crate the `veclite` dependency source is compiled in, so the
/// gate checks the crate's *own* `lib.rs` rather than the mixed total.
const GATES: &[(&str, Option<&str>, f64)] = &[
    ("hivellm-veclite", None, 93.0),
    ("hivellm-veclite-ffi", Some("veclite-ffi/src/lib.rs"), 95.0),
];

pub fn run(_args: &[String]) -> i32 {
    let mut failed = false;
    for &(package, file_suffix, floor) in GATES {
        match measure(package) {
            Ok(json) => match line_percent(&json, file_suffix) {
                Some(pct) => {
                    let ok = pct + 1e-9 >= floor;
                    failed |= !ok;
                    eprintln!(
                        "[coverage] {package:<14} {pct:6.2}% line  (floor {floor:.0}%)  {}",
                        if ok { "PASS" } else { "FAIL" }
                    );
                }
                None => {
                    eprintln!("[coverage] {package}: could not find coverage for {file_suffix:?}");
                    failed = true;
                }
            },
            Err(e) => {
                eprintln!("[coverage] {package}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        eprintln!("[coverage] FAIL — see docs/COVERAGE.md for the policy");
        1
    } else {
        eprintln!("[coverage] PASS");
        0
    }
}

/// Run `cargo llvm-cov -p <package> --tests --summary-only --json` and parse the
/// JSON report.
fn measure(package: &str) -> Result<Value, String> {
    let output = Command::new(env!("CARGO"))
        .args([
            "llvm-cov",
            "-p",
            package,
            "--tests",
            "--summary-only",
            "--json",
        ])
        .output()
        .map_err(|e| format!("launch cargo llvm-cov: {e} (is cargo-llvm-cov installed?)"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo llvm-cov exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .last()
                .unwrap_or("")
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("parse llvm-cov json: {e}"))
}

/// Line-coverage percent: the crate total when `file_suffix` is `None`, else the
/// summary for the single file whose path ends with `file_suffix`.
fn line_percent(report: &Value, file_suffix: Option<&str>) -> Option<f64> {
    let data = report.get("data")?.as_array()?.first()?;
    match file_suffix {
        None => data.get("totals")?.get("lines")?.get("percent")?.as_f64(),
        Some(suffix) => {
            let want = suffix.replace('/', std::path::MAIN_SEPARATOR_STR);
            data.get("files")?.as_array()?.iter().find_map(|f| {
                let name = f.get("filename")?.as_str()?;
                (name.ends_with(&want) || name.ends_with(suffix))
                    .then(|| f.get("summary")?.get("lines")?.get("percent")?.as_f64())
                    .flatten()
            })
        }
    }
}
