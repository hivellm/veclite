//! Docs gate (SPEC-016 REL-040/041; DAG T6.3). `cargo xtask docs [--quickstarts]
//! [--links] [--build]` (no flag = all three):
//!
//! - **quickstarts**: runs each of the six language quickstarts — the executed
//!   docs samples (REL-041: a sample that drifts from the API fails here). Each
//!   language is *probed* first; when its toolchain or built artifact is absent
//!   the quickstart is **skipped** (the full platform matrix runs the rest),
//!   never silently passed. Rust is always runnable and must pass.
//! - **links**: a relative-link checker over every `docs/**/*.md` + `README.md`
//!   — a link to a missing file fails the build (stale cross-references).
//! - **build**: `mdbook build` of the docs site when `mdbook` is installed.
//!
//! No network access; nothing is published — this only proves the docs are
//! runnable and internally consistent.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One language quickstart: how to detect its toolchain and how to run it.
struct Quickstart {
    lang: &'static str,
    /// The executed sample file (shown in output; also link-checked).
    sample: &'static str,
    /// Probe command — success means the toolchain/artifact is present.
    probe: fn() -> bool,
    /// The run command as `(program, args, cwd)`.
    run: fn() -> (String, Vec<String>, Option<PathBuf>),
    /// Extra environment for the run command (e.g. cgo flags for Go).
    env: fn() -> Vec<(String, String)>,
}

/// The C compiler cgo should use, if a bare `cc`/`gcc` is not on PATH but `zig`
/// is (`zig cc` is a drop-in). Empty means "let cgo use its default".
fn cgo_env() -> Vec<(String, String)> {
    let mut env = vec![("CGO_ENABLED".to_string(), "1".to_string())];
    if !have("cc", &["--version"]) && !have("gcc", &["--version"]) && have("zig", &["version"]) {
        env.push(("CC".to_string(), "zig cc".to_string()));
    }
    env
}

fn have(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn quickstarts() -> Vec<Quickstart> {
    vec![
        Quickstart {
            lang: "rust",
            sample: "crates/veclite/examples/quickstart.rs",
            probe: || true, // cargo is always present when running xtask
            run: || {
                (
                    "cargo".into(),
                    vec![
                        "run".into(),
                        "-p".into(),
                        "hivellm-veclite".into(),
                        "--example".into(),
                        "quickstart".into(),
                    ],
                    None,
                )
            },
            env: Vec::new,
        },
        Quickstart {
            lang: "python",
            sample: "examples/quickstart.py",
            // Runnable only if the veclite wheel is importable.
            probe: || have("python", &["-c", "import veclite"]),
            run: || ("python".into(), vec!["examples/quickstart.py".into()], None),
            env: Vec::new,
        },
        Quickstart {
            lang: "node",
            sample: "examples/quickstart.mjs",
            // The addon must resolve as the `veclite` package (npm install /
            // link); skip when it does not.
            probe: || have("node", &["-e", "require.resolve('veclite')"]),
            run: || ("node".into(), vec!["examples/quickstart.mjs".into()], None),
            env: Vec::new,
        },
        Quickstart {
            lang: "go",
            sample: "bindings/go/examples/quickstart/main.go",
            // cgo needs a C compiler; skip when none is on PATH.
            probe: || {
                have("go", &["version"])
                    && (have("cc", &["--version"])
                        || have("gcc", &["--version"])
                        || have("zig", &["version"]))
            },
            run: || {
                (
                    "go".into(),
                    vec!["run".into(), ".".into()],
                    Some(PathBuf::from("bindings/go/examples/quickstart")),
                )
            },
            env: cgo_env,
        },
        Quickstart {
            lang: "csharp",
            sample: "bindings/csharp/Quickstart/Program.cs",
            probe: || {
                have("dotnet", &["--version"])
                    && Path::new("bindings/csharp/VecLite/runtimes").exists()
            },
            run: || {
                (
                    "dotnet".into(),
                    vec![
                        "run".into(),
                        "--project".into(),
                        "bindings/csharp/Quickstart".into(),
                        "-c".into(),
                        "Release".into(),
                    ],
                    None,
                )
            },
            env: Vec::new,
        },
        Quickstart {
            lang: "wasm",
            sample: "crates/veclite-wasm/examples/quickstart.mjs",
            // Needs the built wasm artifact next to the JS facade.
            probe: || wasm_built(),
            run: || {
                (
                    "node".into(),
                    vec!["examples/quickstart.mjs".into()],
                    Some(PathBuf::from("crates/veclite-wasm")),
                )
            },
            env: Vec::new,
        },
    ]
}

/// The wasm package is built when a `*_bg.wasm` (or equivalent) sits next to the
/// JS loader.
fn wasm_built() -> bool {
    let js = Path::new("crates/veclite-wasm/js");
    std::fs::read_dir(js)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".wasm")))
        })
        .unwrap_or(false)
}

fn run_quickstarts() -> i32 {
    let mut failed = Vec::new();
    let mut ran = 0usize;
    let mut skipped = Vec::new();
    for qs in quickstarts() {
        if !(qs.probe)() {
            eprintln!(
                "[docs] {:>7} SKIP (toolchain/artifact absent) — {}",
                qs.lang, qs.sample
            );
            skipped.push(qs.lang);
            continue;
        }
        eprintln!("[docs] {:>7} run  {}", qs.lang, qs.sample);
        let (program, args, cwd) = (qs.run)();
        let mut cmd = Command::new(&program);
        cmd.args(&args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        for (key, value) in (qs.env)() {
            cmd.env(key, value);
        }
        let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
        if ok {
            ran += 1;
        } else {
            eprintln!("[docs] {:>7} FAIL", qs.lang);
            failed.push(qs.lang);
        }
    }
    if failed.is_empty() {
        eprintln!(
            "[docs] quickstarts: {ran} ran clean, {} skipped ({:?})",
            skipped.len(),
            skipped
        );
        0
    } else {
        eprintln!("[docs] quickstarts FAILED: {failed:?}");
        1
    }
}

/// Collect every markdown file under `docs/` plus the top-level `README.md`.
fn markdown_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if Path::new("README.md").exists() {
        out.push(PathBuf::from("README.md"));
    }
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip the built book output.
                if path.file_name().and_then(|n| n.to_str()) == Some("site") {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }
    walk(Path::new("docs"), &mut out);
    out
}

/// Extract `[text](target)` link targets from markdown, ignoring code spans is
/// not attempted (a false positive inside a code fence is rare and harmless —
/// the checker only fails on a *relative path that does not resolve*).
fn link_targets(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b']' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            let start = i + 2;
            if let Some(rel) = bytes[start..].iter().position(|&b| b == b')') {
                let target = &text[start..start + rel];
                out.push(target.to_string());
                i = start + rel + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn check_links() -> i32 {
    let mut broken = Vec::new();
    let mut checked = 0usize;
    for file in markdown_files() {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let base = file.parent().unwrap_or_else(|| Path::new("."));
        for target in link_targets(&text) {
            // Skip external links, anchors, and mailto.
            if target.starts_with("http://")
                || target.starts_with("https://")
                || target.starts_with('#')
                || target.starts_with("mailto:")
            {
                continue;
            }
            // Drop any URL fragment / query before resolving the path.
            let path_part = target
                .split('#')
                .next()
                .unwrap_or(&target)
                .split('?')
                .next()
                .unwrap_or(&target);
            if path_part.is_empty() {
                continue; // pure in-page anchor
            }
            checked += 1;
            let resolved = base.join(path_part);
            if !resolved.exists() {
                broken.push(format!("{} -> {}", file.display(), target));
            }
        }
    }
    if broken.is_empty() {
        eprintln!("[docs] links: {checked} relative links resolve");
        0
    } else {
        eprintln!("[docs] {} broken link(s):", broken.len());
        for b in &broken {
            eprintln!("  {b}");
        }
        1
    }
}

fn build_book() -> i32 {
    if !have("mdbook", &["--version"]) {
        eprintln!(
            "[docs] mdbook not installed — skipping site build (install: `cargo install mdbook`)"
        );
        return 0;
    }
    if !Path::new("book.toml").exists() {
        eprintln!("[docs] no book.toml — skipping site build");
        return 0;
    }
    let ok = Command::new("mdbook")
        .arg("build")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        eprintln!("[docs] mdbook build OK");
        0
    } else {
        eprintln!("[docs] mdbook build FAILED");
        1
    }
}

/// `cargo xtask docs [--quickstarts] [--links] [--build]`.
pub fn run(args: &[String]) -> i32 {
    let want = |flag: &str| args.iter().any(|a| a == flag);
    let all = !want("--quickstarts") && !want("--links") && !want("--build");

    let mut code = 0;
    if all || want("--quickstarts") {
        code |= run_quickstarts();
    }
    if all || want("--links") {
        code |= check_links();
    }
    if all || want("--build") {
        code |= build_book();
    }
    if code == 0 {
        eprintln!("[docs] PASS");
    }
    code
}
