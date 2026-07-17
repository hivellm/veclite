//! Read-only full-file integrity pass (SPEC-014 `veclite verify`): header →
//! TOC CRC → every live segment's frame CRC and body decode → IDDIR
//! consistency → WAL sidecar scan. Every damaged element is reported with its
//! absolute offset and segment type (STG-021 error convention); nothing is
//! repaired or written.

use std::io::Read;
use std::path::Path;

use crate::error::{Result, VecLiteError};
use crate::storage::header::{HEADER_SIZE, Header};
use crate::storage::segment::{Segment, SegmentType};
use crate::storage::toc::Toc;
use crate::storage::wal::Wal;

/// One integrity finding: what is damaged and where.
#[derive(Debug, serde::Serialize)]
pub struct Finding {
    /// Absolute file offset of the damaged element (0 for the header).
    pub offset: u64,
    /// Segment type name when the finding is segment-scoped (`config`,
    /// `vectors`, …); `None` for header/TOC/WAL findings.
    pub segment_type: Option<String>,
    /// Collection the segment belongs to, when known.
    pub collection: Option<String>,
    /// What failed, in the storage layer's own words.
    pub detail: String,
}

/// State of the WAL sidecar next to the file.
#[derive(Debug, serde::Serialize)]
pub enum WalStatus {
    /// No sidecar on disk (clean close or never written).
    Absent,
    /// Sidecar scanned: recovered entry count, and whether a torn/stale tail
    /// was discarded (normal after a crash — recovery handles it; not
    /// corruption).
    Scanned {
        /// Entries that replay cleanly.
        entries: usize,
        /// A torn or stale tail was present (discarded on replay).
        discarded_tail: bool,
    },
}

/// The outcome of [`verify_file`]. `findings.is_empty()` means the file is
/// clean (CLI exit 0); any finding is corruption (CLI exit 1).
#[derive(Debug, serde::Serialize)]
pub struct VerifyReport {
    /// Damaged elements, in file order. Empty = clean.
    pub findings: Vec<Finding>,
    /// Collections listed by the TOC (0 when the TOC itself is unreadable).
    pub collections: usize,
    /// Live segments whose frame and body were checked.
    pub segments_checked: usize,
    /// WAL sidecar state.
    pub wal: WalStatus,
}

fn seg_type_name(byte: u8) -> String {
    match SegmentType::from_byte(byte) {
        Ok(SegmentType::Config) => "config".to_string(),
        Ok(SegmentType::Vectors) => "vectors".to_string(),
        Ok(SegmentType::Tombstone) => "tombstone".to_string(),
        Ok(SegmentType::Payload) => "payload".to_string(),
        Ok(SegmentType::Pidx) => "pidx".to_string(),
        Ok(SegmentType::Sparse) => "sparse".to_string(),
        Ok(SegmentType::Hnsw) => "hnsw".to_string(),
        Ok(SegmentType::Vocab) => "vocab".to_string(),
        Ok(SegmentType::Iddir) => "iddir".to_string(),
        Err(_) => format!("unknown({byte})"),
    }
}

/// Open `path` read-only under a shared advisory lock (CLI-002) and run the
/// full integrity pass. `Locked` when a writer holds the exclusive lock; I/O
/// errors propagate; corruption never errors — it lands in `findings`.
pub fn verify_file(path: &Path) -> Result<VerifyReport> {
    let mut file = std::fs::File::open(path)?;
    {
        use fs4::fs_std::FileExt;
        if !FileExt::try_lock_shared(&file)? {
            return Err(VecLiteError::Locked);
        }
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    let mut findings = Vec::new();
    let mut segments_checked = 0usize;
    let mut collections = 0usize;

    // WAL scan is independent of the main chain; report it even when the
    // header is damaged (recovery reads it the same way).
    let wal = scan_wal(path, &bytes);

    // 1. Header (STG-010).
    let header = match Header::decode(&bytes) {
        Ok(header) => Some(header),
        Err(e) => {
            findings.push(Finding {
                offset: 0,
                segment_type: None,
                collection: None,
                detail: e.to_string(),
            });
            None
        }
    };

    // 2. The committed TOC the header points at (STG-051).
    let toc = header.as_ref().and_then(|header| {
        let start = usize::try_from(header.toc_offset).unwrap_or(usize::MAX);
        let len = usize::try_from(header.toc_len).unwrap_or(usize::MAX);
        let end = start.checked_add(len).filter(|&e| e <= bytes.len());
        let Some(end) = end else {
            findings.push(Finding {
                offset: header.toc_offset,
                segment_type: None,
                collection: None,
                detail: "toc: truncated (past end of file)".to_string(),
            });
            return None;
        };
        let body = &bytes[start..end];
        if crc32fast::hash(body) != header.toc_crc32 {
            findings.push(Finding {
                offset: header.toc_offset,
                segment_type: None,
                collection: None,
                detail: "toc: crc mismatch".to_string(),
            });
            return None;
        }
        match Toc::decode(body) {
            Ok(toc) => Some(toc),
            Err(e) => {
                findings.push(Finding {
                    offset: header.toc_offset,
                    segment_type: None,
                    collection: None,
                    detail: e.to_string(),
                });
                None
            }
        }
    });

    // 3. Every live segment: frame CRC + body decode, then a per-collection
    // deep pass (config/vectors/iddir consistency) via the load codec.
    if let Some(toc) = &toc {
        collections = toc.collections.len();
        for entry in &toc.collections {
            let mut segments = Vec::with_capacity(entry.live_segments.len());
            let mut collection_damaged = false;
            for seg_ref in &entry.live_segments {
                segments_checked += 1;
                let type_name = seg_type_name(seg_ref.seg_type);
                let start = usize::try_from(seg_ref.offset).unwrap_or(usize::MAX);
                let len = usize::try_from(seg_ref.len).unwrap_or(usize::MAX);
                if start.checked_add(len).is_none_or(|end| end > bytes.len()) {
                    findings.push(Finding {
                        offset: seg_ref.offset,
                        segment_type: Some(type_name),
                        collection: Some(entry.name.clone()),
                        detail: format!("segment@{}: past end of file", seg_ref.offset),
                    });
                    collection_damaged = true;
                    continue;
                }
                match Segment::read(&bytes[..start + len], start, seg_ref.offset) {
                    Ok((segment, total)) => {
                        if total as u64 != seg_ref.len {
                            findings.push(Finding {
                                offset: seg_ref.offset,
                                segment_type: Some(type_name.clone()),
                                collection: Some(entry.name.clone()),
                                detail: format!(
                                    "segment@{}: on-disk length {} does not match TOC length {}",
                                    seg_ref.offset, total, seg_ref.len
                                ),
                            });
                            collection_damaged = true;
                        }
                        if segment.seg_type.to_byte() != seg_ref.seg_type {
                            findings.push(Finding {
                                offset: seg_ref.offset,
                                segment_type: Some(type_name.clone()),
                                collection: Some(entry.name.clone()),
                                detail: format!(
                                    "segment@{}: type {} does not match TOC type {}",
                                    seg_ref.offset,
                                    seg_type_name(segment.seg_type.to_byte()),
                                    type_name
                                ),
                            });
                            collection_damaged = true;
                        }
                        segments.push(segment);
                    }
                    Err(e) => {
                        findings.push(Finding {
                            offset: seg_ref.offset,
                            segment_type: Some(type_name),
                            collection: Some(entry.name.clone()),
                            detail: e.to_string(),
                        });
                        collection_damaged = true;
                    }
                }
            }
            // Deep pass only over intact frames — a frame finding already
            // condemns the collection, and decoding a partial set would
            // produce misleading follow-on findings.
            if !collection_damaged {
                if let Err(e) = crate::persist::seal::load(&segments) {
                    findings.push(Finding {
                        offset: entry
                            .live_segments
                            .first()
                            .map_or(HEADER_SIZE as u64, |s| s.offset),
                        segment_type: None,
                        collection: Some(entry.name.clone()),
                        detail: format!("collection state does not reconstruct: {e}"),
                    });
                }
            }
        }
    }

    Ok(VerifyReport {
        findings,
        collections,
        segments_checked,
        wal,
    })
}

/// Scan the WAL sidecar read-only (never creates or writes it).
fn scan_wal(db_path: &Path, main_bytes: &[u8]) -> WalStatus {
    let wal_path = crate::persist::wal_path(db_path);
    let Ok(bytes) = std::fs::read(&wal_path) else {
        return WalStatus::Absent;
    };
    // The uuid prefix guards against a stale sidecar (WAL-002); with an
    // unreadable header the scan still runs (prefix mismatch → whole tail
    // discarded, which the report surfaces honestly).
    let mut uuid_prefix = [0u8; 8];
    if main_bytes.len() >= 64 {
        uuid_prefix.copy_from_slice(&main_bytes[48..56]);
    }
    let replay = Wal::scan(&bytes, uuid_prefix);
    WalStatus::Scanned {
        entries: replay.entries.len(),
        discarded_tail: replay.discarded_tail,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom, Write};
    use std::path::PathBuf;

    use super::*;
    use crate::database::VecLite;
    use crate::options::{CollectionOptions, Metric};
    use crate::point::Point;

    fn temp_db(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "veclite-verify-{}-{name}.veclite",
            std::process::id()
        ))
    }

    fn build_db(path: &PathBuf) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(crate::persist::wal_path(path));
        let db = VecLite::open(path).unwrap_or_else(|e| panic!("{e}"));
        let docs = db
            .create_collection("docs", CollectionOptions::new(4, Metric::Cosine))
            .unwrap_or_else(|e| panic!("{e}"));
        for i in 0..50 {
            #[allow(clippy::cast_precision_loss)]
            docs.upsert(
                Point::new(format!("id-{i}"), vec![1.0, i as f32, 2.0, 3.0])
                    .payload(serde_json::json!({"i": i, "blob": "x".repeat(64)})),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
        drop(db);
    }

    #[test]
    fn clean_file_verifies_clean() {
        let path = temp_db("clean");
        build_db(&path);
        let report = verify_file(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(report.findings.is_empty(), "{:?}", report.findings);
        assert_eq!(report.collections, 1);
        assert!(report.segments_checked > 0);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::persist::wal_path(&path));
    }

    #[test]
    fn bit_flip_in_every_segment_is_found_with_offset_and_type() {
        let path = temp_db("bitflip");
        build_db(&path);

        // Read the TOC to enumerate live segments, then flip one body byte in
        // each and expect a finding naming that segment's offset (SPEC-002
        // §9.3 drill, surfaced through verify).
        let clean = std::fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
        let header = Header::decode(&clean).unwrap_or_else(|e| panic!("{e}"));
        #[allow(clippy::cast_possible_truncation)]
        let toc_start = header.toc_offset as usize;
        #[allow(clippy::cast_possible_truncation)]
        let toc = Toc::decode(&clean[toc_start..toc_start + header.toc_len as usize])
            .unwrap_or_else(|e| panic!("{e}"));

        let segments: Vec<_> = toc
            .collections
            .iter()
            .flat_map(|c| c.live_segments.iter().copied())
            .collect();
        assert!(!segments.is_empty());

        for seg_ref in segments {
            let mut damaged = clean.clone();
            // Flip a byte inside the stored body (past the 32-byte header).
            #[allow(clippy::cast_possible_truncation)]
            let target = seg_ref.offset as usize + 32;
            damaged[target] ^= 0x01;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap_or_else(|e| panic!("{e}"));
            file.seek(SeekFrom::Start(0))
                .unwrap_or_else(|e| panic!("{e}"));
            file.write_all(&damaged).unwrap_or_else(|e| panic!("{e}"));
            drop(file);

            let report = verify_file(&path).unwrap_or_else(|e| panic!("{e}"));
            let expected_type = seg_type_name(seg_ref.seg_type);
            assert!(
                report.findings.iter().any(|f| f.offset == seg_ref.offset
                    && f.segment_type.as_deref() == Some(expected_type.as_str())),
                "flip at segment@{} ({expected_type}) not reported: {:?}",
                seg_ref.offset,
                report.findings
            );

            // Restore the clean image for the next segment.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap_or_else(|e| panic!("{e}"));
            file.seek(SeekFrom::Start(0))
                .unwrap_or_else(|e| panic!("{e}"));
            file.write_all(&clean).unwrap_or_else(|e| panic!("{e}"));
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::persist::wal_path(&path));
    }

    #[test]
    fn header_corruption_is_a_finding_not_an_error() {
        let path = temp_db("header");
        build_db(&path);
        let mut bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
        bytes[0] = b'X'; // break the magic
        std::fs::write(&path, &bytes).unwrap_or_else(|e| panic!("{e}"));
        let report = verify_file(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.offset == 0 && f.detail.contains("header")),
            "{:?}",
            report.findings
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(crate::persist::wal_path(&path));
    }
}
