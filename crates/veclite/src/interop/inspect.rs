//! Read-only file inspection (SPEC-014 `veclite inspect`): header, format
//! version, sizes, and the per-collection config + segment breakdown, as a
//! serializable report (`--json` emits it verbatim).

use std::io::Read;
use std::path::Path;

use crate::error::{Result, VecLiteError};
use crate::storage::body::StoredConfig;
use crate::storage::header::Header;
use crate::storage::segment::{Segment, SegmentType};
use crate::storage::toc::Toc;

/// One live segment in a collection's breakdown.
#[derive(Debug, serde::Serialize)]
pub struct SegmentInfo {
    /// Segment type name (`config`, `vectors`, `payload`, …).
    pub segment_type: String,
    /// Absolute file offset.
    pub offset: u64,
    /// On-disk length (32-byte frame + stored body).
    pub len: u64,
}

/// Per-collection inspection entry.
#[derive(Debug, serde::Serialize)]
pub struct CollectionInfo {
    /// Collection name.
    pub name: String,
    /// Aliases pointing at this collection.
    pub aliases: Vec<String>,
    /// Live vectors (per the committed TOC).
    pub vector_count: u64,
    /// Tombstoned slots awaiting vacuum.
    pub tombstone_count: u64,
    /// Vector dimension (from the CONFIG segment).
    pub dimension: u32,
    /// Distance metric (`cosine` / `euclidean` / `dot_product`).
    pub metric: String,
    /// Quantization (`none` / `sq-8` / … / `binary`).
    pub quantization: String,
    /// Auto-embed provider id; `None` for BYO-vector collections.
    pub embedding_provider: Option<String>,
    /// HNSW parameters `(m, ef_construction, ef_search)`.
    pub hnsw: (u32, u32, u32),
    /// Live segment breakdown, in TOC (replay) order.
    pub segments: Vec<SegmentInfo>,
}

/// The outcome of [`inspect_file`].
#[derive(Debug, serde::Serialize)]
pub struct InspectReport {
    /// `.veclite` format version from the header.
    pub format_version: u32,
    /// Minimum reader version the file demands.
    pub min_reader_version: u32,
    /// Whether the last close was clean (no WAL replay pending).
    pub clean_close: bool,
    /// File uuid, lowercase hex.
    pub file_uuid: String,
    /// Creation / last-modification epochs (seconds).
    pub created_epoch_s: u64,
    pub modified_epoch_s: u64,
    /// Checkpoint generation from the committed TOC.
    pub generation: u64,
    /// Main file size in bytes.
    pub file_size: u64,
    /// WAL sidecar size in bytes (0 when absent).
    pub wal_size: u64,
    /// Collections in the committed TOC.
    pub collections: Vec<CollectionInfo>,
}

fn metric_name(byte: u8) -> String {
    match byte {
        0 => "cosine".to_string(),
        1 => "euclidean".to_string(),
        2 => "dot_product".to_string(),
        other => format!("unknown({other})"),
    }
}

fn quantization_name(byte: u8, bits: u8) -> String {
    match byte {
        0 => "none".to_string(),
        1 => format!("sq-{bits}"),
        2 => "binary".to_string(),
        other => format!("unknown({other})"),
    }
}

fn type_name(seg_type: SegmentType) -> &'static str {
    match seg_type {
        SegmentType::Config => "config",
        SegmentType::Vectors => "vectors",
        SegmentType::Tombstone => "tombstone",
        SegmentType::Payload => "payload",
        SegmentType::Pidx => "pidx",
        SegmentType::Sparse => "sparse",
        SegmentType::Hnsw => "hnsw",
        SegmentType::Vocab => "vocab",
        SegmentType::Iddir => "iddir",
    }
}

/// Inspect the committed state of the `.veclite` file at `path`, read-only
/// under a shared advisory lock (CLI-002). Corruption fails with `Corrupt`
/// naming the damaged element — `verify` is the diagnosis tool.
pub fn inspect_file(path: &Path) -> Result<InspectReport> {
    let mut file = std::fs::File::open(path)?;
    {
        use fs4::fs_std::FileExt;
        if !FileExt::try_lock_shared(&file)? {
            return Err(VecLiteError::Locked);
        }
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    let header = Header::decode(&bytes)?;
    let toc_start = usize::try_from(header.toc_offset)
        .map_err(|_| VecLiteError::Corrupt("toc: offset exceeds usize".into()))?;
    let toc_end = toc_start
        .checked_add(usize::try_from(header.toc_len).unwrap_or(usize::MAX))
        .filter(|&end| end <= bytes.len())
        .ok_or_else(|| VecLiteError::Corrupt("toc: truncated".into()))?;
    let toc_body = &bytes[toc_start..toc_end];
    if crc32fast::hash(toc_body) != header.toc_crc32 {
        return Err(VecLiteError::Corrupt("toc: crc mismatch".into()));
    }
    let toc = Toc::decode(toc_body)?;

    let mut collections = Vec::with_capacity(toc.collections.len());
    for entry in &toc.collections {
        let mut segments = Vec::with_capacity(entry.live_segments.len());
        let mut config: Option<StoredConfig> = None;
        for seg_ref in &entry.live_segments {
            let seg_type = SegmentType::from_byte(seg_ref.seg_type)?;
            segments.push(SegmentInfo {
                segment_type: type_name(seg_type).to_string(),
                offset: seg_ref.offset,
                len: seg_ref.len,
            });
            if seg_type == SegmentType::Config {
                let start = usize::try_from(seg_ref.offset)
                    .map_err(|_| VecLiteError::Corrupt("segment: offset exceeds usize".into()))?;
                let end = start
                    .checked_add(usize::try_from(seg_ref.len).unwrap_or(usize::MAX))
                    .filter(|&end| end <= bytes.len())
                    .ok_or_else(|| {
                        VecLiteError::Corrupt(format!(
                            "segment@{}: past end of file",
                            seg_ref.offset
                        ))
                    })?;
                let (segment, _) = Segment::read(&bytes[..end], start, seg_ref.offset)?;
                config = Some(StoredConfig::decode(&segment.body)?);
            }
        }
        let config = config.ok_or_else(|| {
            VecLiteError::Corrupt(format!("collection {:?}: no CONFIG segment", entry.name))
        })?;
        collections.push(CollectionInfo {
            name: entry.name.clone(),
            aliases: entry.aliases.clone(),
            vector_count: entry.vector_count,
            tombstone_count: entry.tombstone_count,
            dimension: config.dimension,
            metric: metric_name(config.metric),
            quantization: quantization_name(config.quantization, config.quant_bits),
            embedding_provider: config.embedding_provider,
            hnsw: (config.m, config.ef_construction, config.ef_search),
            segments,
        });
    }

    let wal_size = std::fs::metadata(crate::persist::wal_path(path))
        .map(|m| m.len())
        .unwrap_or(0);
    let mut uuid_hex = String::with_capacity(32);
    for byte in header.file_uuid {
        use std::fmt::Write;
        let _ = write!(uuid_hex, "{byte:02x}");
    }

    Ok(InspectReport {
        format_version: header.format_version,
        min_reader_version: header.min_reader_version,
        clean_close: header.clean_close(),
        file_uuid: uuid_hex,
        created_epoch_s: header.created_epoch_s,
        modified_epoch_s: header.modified_epoch_s,
        generation: toc.generation,
        file_size: bytes.len() as u64,
        wal_size,
        collections,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::database::VecLite;
    use crate::options::CollectionOptions;

    fn temp_db(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "veclite-inspect-{}-{name}.veclite",
            std::process::id()
        ))
    }

    #[test]
    fn inspect_reports_header_config_and_segments() {
        let path = temp_db("basic");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::persist::wal_path(&path));
        {
            let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
            let docs = db
                .create_collection("docs", CollectionOptions::auto_embed("bm25", 32))
                .unwrap_or_else(|e| panic!("{e}"));
            docs.upsert_text("a", "hello world of embedded vector databases")
                .unwrap_or_else(|e| panic!("{e}"));
            db.create_alias("current", "docs")
                .unwrap_or_else(|e| panic!("{e}"));
            db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        }

        let report = inspect_file(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.format_version, 1);
        assert!(report.file_size > 4096);
        assert_eq!(report.collections.len(), 1);
        let docs = &report.collections[0];
        assert_eq!(docs.name, "docs");
        assert_eq!(docs.aliases, vec!["current".to_string()]);
        assert_eq!(docs.vector_count, 1);
        assert_eq!(docs.dimension, 32);
        assert_eq!(docs.metric, "cosine");
        assert_eq!(docs.embedding_provider.as_deref(), Some("bm25"));
        assert!(
            docs.segments
                .iter()
                .any(|segment| segment.segment_type == "config")
        );
        assert!(
            docs.segments
                .iter()
                .any(|segment| segment.segment_type == "vectors")
        );
        // The report serializes for --json (CLI-003).
        let json = serde_json::to_string(&report).unwrap_or_else(|e| panic!("{e}"));
        assert!(json.contains("\"collections\""));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::persist::wal_path(&path));
    }
}
