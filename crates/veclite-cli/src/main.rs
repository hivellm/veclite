//! `veclite` — the CLI over the VecLite library + SPEC-013 interop
//! (SPEC-014). A thin veneer: it adds no engine behavior of its own.
//!
//! Exit codes (CLI-001, stable — scripts depend on them):
//! 0 success · 1 data/integrity error · 2 usage error · 3 environment error
//! (`Locked`, permissions, disk full). Warnings go to stderr; data to stdout
//! (CLI-003). No network access, ever (CLI-004).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use veclite::interop::{
    ExportOptions, ImportOptions, ImportReport, InspectReport, VerifyReport, WalStatus,
    export_vecdb, import_vecdb, inspect_file, verify_file,
};
use veclite::{VecLite, VecLiteError};

#[derive(Parser)]
#[command(
    name = "veclite",
    // Pinned so --help output (and its committed snapshots) is identical on
    // every platform — otherwise clap renders argv[0] (`veclite.exe`).
    bin_name = "veclite",
    version,
    about = "Inspect, verify, and maintain .veclite databases; exchange data with a Vectorizer server (.vecdb).",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show header, format version, sizes, and per-collection configuration
    /// and segment breakdown. Opens read-only (shared lock).
    Inspect {
        /// Path to the .veclite database file.
        db: PathBuf,
        /// Emit the report as JSON (stable schema) instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Export collections to a Vectorizer server data directory
    /// (vectorizer.vecdb + vectorizer.vecidx, Compact layout).
    Export {
        /// Path to the .veclite database file.
        db: PathBuf,
        /// Target format; only "vecdb" is defined.
        #[arg(long)]
        format: String,
        /// Output directory for the server data set.
        #[arg(long)]
        out: PathBuf,
        /// Comma-separated collection names; default all.
        #[arg(long, value_delimiter = ',')]
        collections: Option<Vec<String>>,
    },
    /// Import a Vectorizer server data set (Compact .vecdb or Legacy
    /// *_vector_store.bin) into a new .veclite database.
    Import {
        /// Source: a .vecdb file, a *_vector_store.bin file, or a directory
        /// containing either layout.
        src: PathBuf,
        /// The .veclite file to create.
        #[arg(long)]
        out: PathBuf,
        /// Comma-separated collection names; default all.
        #[arg(long, value_delimiter = ',')]
        collections: Option<Vec<String>>,
        /// Overwrite an existing output file.
        #[arg(long)]
        force: bool,
    },
    /// Reclaim dead space in place (library vacuum).
    Vacuum {
        /// Path to the .veclite database file.
        db: PathBuf,
    },
    /// Write a compacted, standalone point-in-time copy.
    Snapshot {
        /// Path to the .veclite database file.
        db: PathBuf,
        /// Path for the copy (must not exist).
        #[arg(long)]
        out: PathBuf,
    },
    /// Read-only integrity pass: header, TOC, every segment CRC and body,
    /// collection reconstruction, WAL scan. Exit 0 = clean; 1 = corruption
    /// found (each finding printed with segment offset and type).
    Verify {
        /// Path to the .veclite database file.
        db: PathBuf,
    },
}

/// CLI-001 exit-code contract.
fn exit_for(error: &VecLiteError) -> ExitCode {
    match error {
        VecLiteError::Corrupt(_) | VecLiteError::UnsupportedFormatVersion { .. } => {
            ExitCode::from(1)
        }
        VecLiteError::InvalidArgument(_)
        | VecLiteError::CollectionNotFound(_)
        | VecLiteError::VectorNotFound(_)
        | VecLiteError::AlreadyExists(_)
        | VecLiteError::DimensionMismatch { .. }
        | VecLiteError::UnsupportedProvider { .. } => ExitCode::from(2),
        // Locked, ReadOnly, Io, WalPending, Closed, and any future variant:
        // the environment, not the data or the invocation.
        _ => ExitCode::from(3),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Inspect { db, json } => inspect(&db, json),
        Command::Export {
            db,
            format,
            out,
            collections,
        } => export(&db, &format, &out, collections),
        Command::Import {
            src,
            out,
            collections,
            force,
        } => import(&src, &out, collections, force),
        Command::Vacuum { db } => vacuum(&db),
        Command::Snapshot { db, out } => snapshot(&db, &out),
        Command::Verify { db } => verify(&db),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("veclite: {error}");
            exit_for(&error)
        }
    }
}

fn inspect(db: &Path, json: bool) -> veclite::Result<ExitCode> {
    let report = inspect_file(db)?;
    if json {
        let rendered = serde_json::to_string_pretty(&report)
            .map_err(|e| VecLiteError::InvalidArgument(format!("serialize report: {e}")))?;
        println!("{rendered}");
    } else {
        print_inspect(db, &report);
    }
    Ok(ExitCode::SUCCESS)
}

fn print_inspect(db: &Path, report: &InspectReport) {
    println!("{}", db.display());
    println!(
        "  format v{} (min reader v{}), generation {}, clean close: {}",
        report.format_version, report.min_reader_version, report.generation, report.clean_close
    );
    println!(
        "  file {} bytes, WAL {} bytes, uuid {}",
        report.file_size, report.wal_size, report.file_uuid
    );
    println!("  collections: {}", report.collections.len());
    for collection in &report.collections {
        let aliases = if collection.aliases.is_empty() {
            String::new()
        } else {
            format!(" (aliases: {})", collection.aliases.join(", "))
        };
        println!("\n  {}{aliases}", collection.name);
        println!(
            "    {} vectors, {} tombstones, dim {}, metric {}, quantization {}",
            collection.vector_count,
            collection.tombstone_count,
            collection.dimension,
            collection.metric,
            collection.quantization
        );
        println!(
            "    hnsw m={} ef_construction={} ef_search={}, provider: {}",
            collection.hnsw.0,
            collection.hnsw.1,
            collection.hnsw.2,
            collection
                .embedding_provider
                .as_deref()
                .unwrap_or("none (BYO vectors)")
        );
        for segment in &collection.segments {
            println!(
                "    segment {:<9} @{:<10} {} bytes",
                segment.segment_type, segment.offset, segment.len
            );
        }
    }
}

fn export(
    db_path: &Path,
    format: &str,
    out: &Path,
    collections: Option<Vec<String>>,
) -> veclite::Result<ExitCode> {
    if format != "vecdb" {
        return Err(VecLiteError::InvalidArgument(format!(
            "unknown export format {format:?}; the only defined format is \"vecdb\""
        )));
    }
    // Exclusive open: an export must see WAL-replayed state, and settling
    // text refits may write (CLI-002 mutating-command locking).
    let db = VecLite::open(db_path)?;
    let report = export_vecdb(&db, out, &ExportOptions { collections })?;
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    let total_vectors: usize = report.collections.iter().map(|c| c.vectors).sum();
    println!(
        "exported {} collections, {} vectors, {} bytes -> {}",
        report.collections.len(),
        total_vectors,
        report.total_bytes,
        report.vecdb_path.display()
    );
    for collection in &report.collections {
        println!(
            "  {}: {} vectors, {} bytes",
            collection.name, collection.vectors, collection.bytes
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn import(
    src: &Path,
    out: &Path,
    collections: Option<Vec<String>>,
    force: bool,
) -> veclite::Result<ExitCode> {
    if out.exists() {
        if !force {
            return Err(VecLiteError::InvalidArgument(format!(
                "{}: output already exists; pass --force to overwrite",
                out.display()
            )));
        }
        std::fs::remove_file(out)?;
        let _ = std::fs::remove_file(wal_sidecar(out));
    }
    let db = VecLite::open(out)?;
    let imported = import_vecdb(src, &db, &ImportOptions { collections })
        .and_then(|report| db.checkpoint().map(|()| report));
    drop(db);
    match imported {
        Ok(report) => {
            print_import(out, &report);
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            // Never leave a half-imported database behind.
            let _ = std::fs::remove_file(out);
            let _ = std::fs::remove_file(wal_sidecar(out));
            Err(error)
        }
    }
}

/// The WAL sidecar path (`<db>.veclite-wal`, WAL-001) — cleanup targets.
fn wal_sidecar(db: &Path) -> PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push("-wal");
    db.with_file_name(name)
}

fn print_import(out: &Path, report: &ImportReport) {
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    let total_vectors: usize = report.collections.iter().map(|c| c.vectors).sum();
    println!(
        "imported {} collections, {} vectors ({:?} layout) -> {}",
        report.collections.len(),
        total_vectors,
        report.layout,
        out.display()
    );
    for collection in &report.collections {
        let deferred = collection
            .deferred_provider
            .as_deref()
            .map(|provider| format!(" [BYO fallback, origin provider {provider:?}]"))
            .unwrap_or_default();
        println!(
            "  {}: {} vectors{deferred}",
            collection.name, collection.vectors
        );
    }
}

fn vacuum(db_path: &Path) -> veclite::Result<ExitCode> {
    let before = std::fs::metadata(db_path)?.len();
    let db = VecLite::open(db_path)?;
    db.vacuum()?;
    drop(db);
    let after = std::fs::metadata(db_path)?.len();
    println!(
        "vacuumed {}: {} -> {} bytes",
        db_path.display(),
        before,
        after
    );
    Ok(ExitCode::SUCCESS)
}

fn snapshot(db_path: &Path, out: &Path) -> veclite::Result<ExitCode> {
    let db = VecLite::open(db_path)?;
    db.snapshot(out)?;
    let size = std::fs::metadata(out)?.len();
    println!(
        "snapshot {} -> {} ({} bytes)",
        db_path.display(),
        out.display(),
        size
    );
    Ok(ExitCode::SUCCESS)
}

fn verify(db_path: &Path) -> veclite::Result<ExitCode> {
    let report = verify_file(db_path)?;
    print_verify(db_path, &report);
    if report.findings.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn print_verify(db: &Path, report: &VerifyReport) {
    println!(
        "{}: {} collections, {} segments checked",
        db.display(),
        report.collections,
        report.segments_checked
    );
    match &report.wal {
        WalStatus::Absent => {}
        WalStatus::Scanned {
            entries,
            discarded_tail,
        } => {
            let tail = if *discarded_tail {
                " (torn/stale tail present — recovery discards it)"
            } else {
                ""
            };
            println!("WAL: {entries} entries pending replay{tail}");
        }
    }
    if report.findings.is_empty() {
        println!("clean");
        return;
    }
    for finding in &report.findings {
        let segment = finding
            .segment_type
            .as_deref()
            .map(|t| format!(" type={t}"))
            .unwrap_or_default();
        let collection = finding
            .collection
            .as_deref()
            .map(|c| format!(" collection={c}"))
            .unwrap_or_default();
        println!(
            "finding: offset={}{segment}{collection} — {}",
            finding.offset, finding.detail
        );
    }
    println!("{} finding(s)", report.findings.len());
}
