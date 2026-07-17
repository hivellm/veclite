//! CLI contract tests (SPEC-014): the exit-code table (CLI-001), lock
//! behavior (CLI-002), stdout/stderr separation (CLI-003), and committed
//! `--help` snapshots so the docs cannot drift from the binary (§2.4).

use std::path::PathBuf;
use std::process::{Command, Output};

use veclite::{CollectionOptions, Metric, Point, VecLite};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_veclite"))
}

fn run(args: &[&str]) -> Output {
    bin()
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run veclite: {e}"))
}

fn exit_code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("veclite-cli-{}-{name}", std::process::id()))
}

fn wal_sidecar(db: &PathBuf) -> PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push("-wal");
    db.with_file_name(name)
}

fn remove_db(path: &PathBuf) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(wal_sidecar(path));
}

/// A small durable database: one auto-embed collection, one BYO collection.
fn build_db(path: &PathBuf) {
    remove_db(path);
    let db = VecLite::open(path).unwrap_or_else(|e| panic!("{e}"));
    let docs = db
        .create_collection("docs", CollectionOptions::auto_embed("bm25", 32))
        .unwrap_or_else(|e| panic!("{e}"));
    docs.upsert_text("a", "the quick brown fox jumps over the lazy dog")
        .unwrap_or_else(|e| panic!("{e}"));
    docs.upsert_text("b", "veclite is an embedded vector database")
        .unwrap_or_else(|e| panic!("{e}"));
    let vecs = db
        .create_collection("vecs", CollectionOptions::new(3, Metric::Euclidean))
        .unwrap_or_else(|e| panic!("{e}"));
    vecs.upsert(Point::new("v1", vec![1.0, 2.0, 3.0]).payload(serde_json::json!({"lang": "en"})))
        .unwrap_or_else(|e| panic!("{e}"));
    db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
}

// ── exit-code contract (CLI-001) ─────────────────────────────────────────

#[test]
fn clean_verify_exits_0_and_prints_clean() {
    let db = temp_path("verify-clean.veclite");
    build_db(&db);
    let output = run(&["verify", db.to_str().unwrap_or_default()]);
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("clean"));
    remove_db(&db);
}

#[test]
fn corrupted_file_verify_exits_1_naming_offset_and_type() {
    let db = temp_path("verify-corrupt.veclite");
    build_db(&db);
    // Flip the last byte of the file: the committed TOC is always the final
    // element of the header→TOC chain, so this deterministically damages a
    // live element (the per-segment-type sweep lives in the library tests).
    let mut bytes = std::fs::read(&db).unwrap_or_else(|e| panic!("{e}"));
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    std::fs::write(&db, &bytes).unwrap_or_else(|e| panic!("{e}"));

    let output = run(&["verify", db.to_str().unwrap_or_default()]);
    assert_eq!(exit_code(&output), 1, "stdout: {}", stdout(&output));
    let text = stdout(&output);
    assert!(text.contains("finding:"), "no finding printed: {text}");
    assert!(text.contains("offset="), "finding lacks offset: {text}");
    remove_db(&db);
}

#[test]
fn usage_errors_exit_2() {
    // Unknown subcommand (clap).
    let output = run(&["frobnicate"]);
    assert_eq!(exit_code(&output), 2);

    // Unknown export format (CLI validation).
    let db = temp_path("usage.veclite");
    build_db(&db);
    let out_dir = temp_path("usage-out");
    let output = run(&[
        "export",
        db.to_str().unwrap_or_default(),
        "--format",
        "parquet",
        "--out",
        out_dir.to_str().unwrap_or_default(),
    ]);
    assert_eq!(exit_code(&output), 2, "stderr: {}", stderr(&output));
    assert!(stderr(&output).contains("vecdb"));
    remove_db(&db);
}

#[test]
fn import_refuses_existing_output_without_force_exit_2() {
    let db = temp_path("import-src.veclite");
    build_db(&db);
    let export_dir = temp_path("import-refuse-dir");
    let _ = std::fs::remove_dir_all(&export_dir);
    let output = run(&[
        "export",
        db.to_str().unwrap_or_default(),
        "--format",
        "vecdb",
        "--out",
        export_dir.to_str().unwrap_or_default(),
    ]);
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));

    // The source db itself exists → import to it must refuse without --force.
    let output = run(&[
        "import",
        export_dir.to_str().unwrap_or_default(),
        "--out",
        db.to_str().unwrap_or_default(),
    ]);
    assert_eq!(exit_code(&output), 2, "stderr: {}", stderr(&output));
    assert!(stderr(&output).contains("--force"));

    remove_db(&db);
    let _ = std::fs::remove_dir_all(&export_dir);
}

#[test]
fn locked_database_exits_3() {
    let db = temp_path("locked.veclite");
    build_db(&db);
    // Hold the exclusive lock in-process (a live writer), then vacuum — a
    // mutating command that needs the exclusive lock (CLI-002).
    let held = VecLite::open(&db).unwrap_or_else(|e| panic!("{e}"));
    let output = run(&["vacuum", db.to_str().unwrap_or_default()]);
    assert_eq!(exit_code(&output), 3, "stderr: {}", stderr(&output));
    drop(held);
    remove_db(&db);
}

#[test]
fn missing_file_exits_3() {
    let output = run(&["inspect", "definitely-not-a-real-file.veclite"]);
    assert_eq!(exit_code(&output), 3, "stderr: {}", stderr(&output));
}

// ── round-trip smoke (SPEC-014 §2.1) ─────────────────────────────────────

#[test]
fn export_import_inspect_round_trip() {
    let db = temp_path("roundtrip.veclite");
    build_db(&db);
    let export_dir = temp_path("roundtrip-dir");
    let _ = std::fs::remove_dir_all(&export_dir);
    let imported = temp_path("roundtrip-imported.veclite");
    remove_db(&imported);

    let output = run(&[
        "export",
        db.to_str().unwrap_or_default(),
        "--format",
        "vecdb",
        "--out",
        export_dir.to_str().unwrap_or_default(),
    ]);
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("exported 2 collections"));
    // Warnings (the BYO provider-default note) go to stderr, not stdout.
    assert!(stderr(&output).contains("warning:"));
    assert!(!stdout(&output).contains("warning:"));

    let output = run(&[
        "import",
        export_dir.to_str().unwrap_or_default(),
        "--out",
        imported.to_str().unwrap_or_default(),
    ]);
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));
    assert!(stdout(&output).contains("imported 2 collections"));

    let output = run(&["inspect", imported.to_str().unwrap_or_default(), "--json"]);
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&output)).unwrap_or_else(|e| panic!("{e}"));
    let collections = report["collections"]
        .as_array()
        .unwrap_or_else(|| panic!("collections array expected"));
    assert_eq!(collections.len(), 2);

    let output = run(&["verify", imported.to_str().unwrap_or_default()]);
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));

    remove_db(&db);
    remove_db(&imported);
    let _ = std::fs::remove_dir_all(&export_dir);
}

#[test]
fn vacuum_and_snapshot_smoke() {
    let db = temp_path("maintenance.veclite");
    build_db(&db);
    let copy = temp_path("maintenance-copy.veclite");
    remove_db(&copy);

    let output = run(&["vacuum", db.to_str().unwrap_or_default()]);
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));

    let output = run(&[
        "snapshot",
        db.to_str().unwrap_or_default(),
        "--out",
        copy.to_str().unwrap_or_default(),
    ]);
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));
    let output = run(&["verify", copy.to_str().unwrap_or_default()]);
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));

    remove_db(&db);
    remove_db(&copy);
}

// ── --help snapshots (§2.4: docs stay in sync) ───────────────────────────

fn assert_help_snapshot(args: &[&str], snapshot: &str) {
    let output = run(args);
    assert_eq!(exit_code(&output), 0);
    let actual = stdout(&output);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(snapshot);
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing snapshot {}: {e}", path.display()))
        .replace("\r\n", "\n");
    assert_eq!(
        actual.trim_end(),
        expected.trim_end(),
        "--help drifted from the committed snapshot {}; update the snapshot \
         deliberately if the change is intended",
        path.display()
    );
}

#[test]
fn help_snapshots_are_current() {
    assert_help_snapshot(&["--help"], "help.txt");
    assert_help_snapshot(&["inspect", "--help"], "help_inspect.txt");
    assert_help_snapshot(&["export", "--help"], "help_export.txt");
    assert_help_snapshot(&["import", "--help"], "help_import.txt");
    assert_help_snapshot(&["vacuum", "--help"], "help_vacuum.txt");
    assert_help_snapshot(&["snapshot", "--help"], "help_snapshot.txt");
    assert_help_snapshot(&["verify", "--help"], "help_verify.txt");
}
