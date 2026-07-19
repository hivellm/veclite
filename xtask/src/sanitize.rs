//! Sanitizer runs of the integration suite (SPEC-015 TST-052; CORE-054).
//!
//! `cargo xtask sanitize <asan|tsan> [--filter <substr>] [--native]`
//!
//! Builds and runs the veclite test suite under `-Zsanitizer=address` (ASan +
//! LeakSanitizer) or `-Zsanitizer=thread` (TSan). Sanitizers need a nightly
//! `-Zbuild-std` build on a Linux target — TSan does not exist on
//! windows-msvc at all — so on Windows the run happens inside the same
//! `rustlang/rust:nightly` container the fuzz orchestration uses (`--native`
//! forces a local attempt, for Linux hosts).
//!
//! Scope: the pure-Rust engine + `vecdb-interop` (`--features vecdb-interop`,
//! `--tests`). The `onnx` feature is excluded by design: it links the
//! prebuilt ONNX Runtime, which is neither rebuilt by `-Zbuild-std` nor
//! meaningfully instrumentable from here.

use std::process::Command;

/// `cargo xtask sanitize`.
pub fn run(args: &[String]) -> i32 {
    let sanitizer = match args.first().map(String::as_str) {
        Some("asan") => "address",
        Some("tsan") => "thread",
        _ => {
            eprintln!("usage: cargo xtask sanitize <asan|tsan> [--filter <substr>] [--native]");
            return 2;
        }
    };
    let filter: Option<&String> = args
        .iter()
        .position(|a| a == "--filter")
        .and_then(|i| args.get(i + 1));
    let native = args.iter().any(|a| a == "--native") || !cfg!(windows);

    let mut test_args = vec![
        "test".to_string(),
        "-Zbuild-std".to_string(),
        "--target".to_string(),
        "x86_64-unknown-linux-gnu".to_string(),
        "-p".to_string(),
        "hivellm-veclite".to_string(),
        "--features".to_string(),
        "vecdb-interop".to_string(),
        "--tests".to_string(),
    ];
    if let Some(filter) = filter {
        test_args.push("--".to_string());
        test_args.push(filter.clone());
    }
    let rustflags = format!("-Zsanitizer={sanitizer}");

    eprintln!(
        "[sanitize] {sanitizer} sanitizer, {} run…",
        if native { "native" } else { "docker" }
    );
    let status = if native {
        Command::new("cargo")
            .arg("+nightly")
            .args(&test_args)
            .env("RUSTFLAGS", &rustflags)
            .env("RUSTDOCFLAGS", &rustflags)
            .status()
    } else {
        let repo = std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default();
        // TSan needs ASLR off; `setarch -R` disables it per-process. The
        // default Docker seccomp profile blocks the personality() syscall it
        // uses, so the run also drops the profile (`--security-opt
        // seccomp=unconfined`) — harmless for ASan, required for TSan.
        let launcher = if sanitizer == "thread" {
            "setarch -R env "
        } else {
            ""
        };
        let script = format!(
            "rustup component add rust-src >/dev/null 2>&1; \
             {launcher}sh -c \"RUSTFLAGS='{rustflags}' RUSTDOCFLAGS='{rustflags}' cargo {}\"",
            test_args.join(" ")
        );
        Command::new("docker")
            .args([
                "run",
                "--rm",
                "--security-opt",
                "seccomp=unconfined",
                "-v",
                &format!("{repo}:/work"),
                "-v",
                "veclite-fuzz-cargo:/usr/local/cargo",
                "-v",
                "veclite-sanitize-rustup:/usr/local/rustup",
                "-v",
                "veclite-sanitize-target:/work/target",
                "-w",
                "/work",
                "rustlang/rust:nightly",
                "bash",
                "-ceu",
                &script,
            ])
            .status()
    };
    match status {
        Ok(s) if s.success() => {
            eprintln!("[sanitize] {sanitizer} clean");
            0
        }
        Ok(s) => {
            eprintln!("[sanitize] {sanitizer} FAILED ({s})");
            1
        }
        Err(e) => {
            eprintln!("[sanitize] could not launch: {e}");
            1
        }
    }
}
