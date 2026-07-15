//! Sealing and loading a collection's live state to/from segments (SPEC-002
//! §5). A checkpoint seals the compacted live set (dead slots dropped —
//! space reclaimed as a side effect) into CONFIG + VECTORS(f32) + IDDIR +
//! PAYLOAD segments; load reverses it. The HNSW graph is not persisted in v1 —
//! it is rebuilt from the vectors on load (STG-063). Sparse/vocab/PIDX
//! persistence lands with their features (phase3).

use serde_json::Value;

use crate::error::{Result, VecLiteError};
use crate::filter::index::{PayloadIndexes, PostingValue};
use crate::options::{CollectionOptions, PayloadIndexKind};
use crate::persist::config;
use crate::storage::body::{PayloadBlock, PayloadIndex, StoredConfig};
use crate::storage::iddir::IdDir;
use crate::storage::pager::CheckpointColl;
use crate::storage::segment::{Segment, SegmentType};
use crate::storage::vectors::{Encoding, VectorsBody};

/// One live point: id, dense vector, optional payload. Sparse is not yet
/// persisted (phase3c).
pub(crate) type LivePoint = (String, Vec<f32>, Option<Value>);

/// A collection reconstructed from its segments. Exactly one of `points` /
/// `base` carries the vectors: the materialized tier fills `points`; the mmap
/// tier (STG-004, ADR-0004) leaves `points` empty and hands the vector
/// regions plus slot metadata in `base`.
pub(crate) struct LoadedCollection {
    pub(crate) options: CollectionOptions,
    pub(crate) points: Vec<LivePoint>,
    pub(crate) base: Option<LoadedBase>,
    /// Checkpointed embedder state (VOCAB segment, SPEC-005 EMB-030): imported
    /// on open so text search needs no rebuild. `None` for BYO collections and
    /// legacy files (whose first search refits from `_text`).
    pub(crate) vocab: Option<Vec<u8>>,
}

/// The mmap tier's load product (ADR-0004): slot metadata in RAM, vector bytes
/// left in the file map. `seg_refs` keeps the collection's committed segment
/// references so an unmutated collection can be carried forward by reference at
/// the next checkpoint instead of resealed.
pub(crate) struct LoadedBase {
    /// Mapped VECTORS windows, covering slots `0..slot_count` contiguously.
    pub(crate) regions: Vec<crate::storage::mmap::VectorsRegion>,
    /// Total slots (live + dead) addressed by `regions`.
    pub(crate) slot_count: usize,
    /// Slot → id; `None` marks a dead slot (absent from the IDDIR).
    pub(crate) ids: Vec<Option<String>>,
    /// Slot → payload.
    pub(crate) payloads: Vec<Option<Value>>,
    /// The committed live segments (replay order), for clean carry-forward.
    pub(crate) seg_refs: Vec<crate::storage::toc::SegRef>,
    pub(crate) vector_count: u64,
    pub(crate) tombstone_count: u64,
    /// Whether the mapped vectors fit `OpenOptions::memory_budget` — build the
    /// in-RAM HNSW when true, serve exact scans from the map when false
    /// (STG-064).
    pub(crate) indexed: bool,
}

/// Bucket count heuristic: ~1 entry/bucket, clamped to a sane floor.
fn bucket_count(n: usize) -> usize {
    n.next_power_of_two().max(8)
}

/// Encode one sealed posting value into the PIDX opaque-value bytes
/// (SPEC-002 §3.1): keyword = utf8, integer = i64 LE, float = f64 bits LE.
fn posting_value_bytes(v: &PostingValue) -> Vec<u8> {
    match v {
        PostingValue::Keyword(s) => s.as_bytes().to_vec(),
        PostingValue::Integer(i) => i.to_le_bytes().to_vec(),
        PostingValue::Float(f) => f.to_bits().to_le_bytes().to_vec(),
    }
}

/// Seal a collection's live set into a `CheckpointColl` ready for the pager
/// (SPEC-002 §5). Slots are compacted to `0..live.len()`; `declared` payload
/// indexes are rebuilt over the compacted numbering and sealed as one PIDX
/// segment per key (SPEC-006 FLT-020/021).
#[allow(clippy::too_many_arguments)] // seal mirrors the full segment set
pub(crate) fn seal(
    coll_id: u32,
    name: String,
    aliases: Vec<String>,
    options: &CollectionOptions,
    declared: &[(String, PayloadIndexKind)],
    vocab: Option<&[u8]>,
    live: &[LivePoint],
    created_epoch_s: u64,
) -> Result<CheckpointColl> {
    let dim = options.dimension;
    let mut segments = Vec::with_capacity(4);

    segments.push(Segment {
        seg_type: SegmentType::Config,
        seg_flags: 0,
        coll_id,
        body: config::to_stored(options, created_epoch_s).encode()?,
    });

    let mut records = Vec::with_capacity(live.len() * dim * 4);
    for (_, vector, _) in live {
        for f in vector {
            records.extend_from_slice(&f.to_le_bytes());
        }
    }
    let vectors = VectorsBody {
        encoding: Encoding::F32,
        dimension: u32::try_from(dim)
            .map_err(|_| VecLiteError::Corrupt("seal: dimension exceeds u32".to_owned()))?,
        first_slot: 0,
        count: live.len() as u64,
        sq_params: None,
        records,
    };
    segments.push(Segment {
        seg_type: SegmentType::Vectors,
        seg_flags: 0,
        coll_id,
        body: vectors.encode(),
    });

    let mut dir = IdDir::new(bucket_count(live.len()));
    for (slot, (id, _, _)) in live.iter().enumerate() {
        dir.insert(id.clone(), slot as u64);
    }
    segments.push(Segment {
        seg_type: SegmentType::Iddir,
        seg_flags: 0,
        coll_id,
        body: dir.encode(),
    });

    let payload_entries: Vec<(u64, Value)> = live
        .iter()
        .enumerate()
        .filter_map(|(slot, (_, _, payload))| payload.clone().map(|v| (slot as u64, v)))
        .collect();
    if !payload_entries.is_empty() {
        segments.push(Segment {
            seg_type: SegmentType::Payload,
            seg_flags: 0,
            coll_id,
            body: PayloadBlock {
                entries: payload_entries,
            }
            .encode()?,
        });
    }

    // One PIDX segment per declared index (SPEC-002 §3.1): rebuild the
    // postings over the compacted slot numbering so the sealed bitmaps match
    // the sealed VECTORS/IDDIR. Declarations survive reopen through these
    // segments; readers rebuild the in-memory bitmaps from payloads (FLT-021).
    if !declared.is_empty() {
        let mut indexes = PayloadIndexes::new(declared);
        for (slot, (_, _, payload)) in live.iter().enumerate() {
            indexes.insert(slot as u64, payload.as_ref());
        }
        for (key, kind) in declared {
            let postings = indexes.postings(key).unwrap_or_default();
            let mut encoded = Vec::with_capacity(postings.len());
            for (value, slots) in &postings {
                let bitmap: roaring::RoaringTreemap = slots.iter().copied().collect();
                encoded.push((posting_value_bytes(value), bitmap));
            }
            segments.push(Segment {
                seg_type: SegmentType::Pidx,
                seg_flags: 0,
                coll_id,
                body: PayloadIndex {
                    kind: config::pidx_kind_byte(*kind),
                    key: key.clone(),
                    postings: encoded,
                }
                .encode()?,
            });
        }
    }

    // VOCAB segment (SPEC-005 EMB-030): the embedder's exported state, so a
    // reopened auto-embed collection searches identically with no rebuild.
    if let Some(state) = vocab {
        if !state.is_empty() {
            segments.push(Segment {
                seg_type: SegmentType::Vocab,
                seg_flags: 0,
                coll_id,
                body: crate::storage::body::encode_vocab(state),
            });
        }
    }

    Ok(CheckpointColl {
        coll_id,
        name,
        aliases,
        vector_count: live.len() as u64,
        tombstone_count: 0,
        segments,
        reused: None,
    })
}

/// Reconstruct a collection's config and live points from its segments. The
/// caller supplies the segments in any order (they are dispatched by type).
pub(crate) fn load(segments: &[Segment]) -> Result<LoadedCollection> {
    let mut stored: Option<StoredConfig> = None;
    let mut vectors: Option<VectorsBody> = None;
    let mut iddir: Option<IdDir> = None;
    let mut payload = PayloadBlock {
        entries: Vec::new(),
    };
    let mut declared: Vec<(String, PayloadIndexKind)> = Vec::new();
    let mut vocab: Option<Vec<u8>> = None;
    for seg in segments {
        match seg.seg_type {
            SegmentType::Config => stored = Some(StoredConfig::decode(&seg.body)?),
            SegmentType::Vocab => vocab = Some(crate::storage::body::decode_vocab(&seg.body)),
            SegmentType::Vectors => vectors = Some(VectorsBody::decode(&seg.body)?),
            SegmentType::Iddir => iddir = Some(IdDir::decode(&seg.body)?),
            SegmentType::Payload => payload = PayloadBlock::decode(&seg.body)?,
            // Declarations are harvested from PIDX; the in-memory bitmaps are
            // rebuilt from the loaded payloads (FLT-021 rebuild model).
            SegmentType::Pidx => {
                let pidx = PayloadIndex::decode(&seg.body)?;
                declared.push((pidx.key, config::pidx_kind_from(pidx.kind)?));
            }
            // HNSW is rebuilt from vectors (STG-063); other types are not yet
            // produced by seal.
            _ => {}
        }
    }
    let stored = stored.ok_or_else(|| VecLiteError::Corrupt("load: missing CONFIG".to_owned()))?;
    let vectors =
        vectors.ok_or_else(|| VecLiteError::Corrupt("load: missing VECTORS".to_owned()))?;
    let iddir = iddir.ok_or_else(|| VecLiteError::Corrupt("load: missing IDDIR".to_owned()))?;
    let mut options = config::from_stored(&stored)?;
    options.payload_indexes = declared;
    let dim = options.dimension;

    // slot → id and slot → payload lookups.
    let count = usize::try_from(vectors.count)
        .map_err(|_| VecLiteError::Corrupt("load: count exceeds usize".to_owned()))?;
    let mut ids: Vec<Option<String>> = vec![None; count];
    for (id, slot) in iddir.entries() {
        let s = usize::try_from(slot)
            .ok()
            .filter(|&s| s < count)
            .ok_or_else(|| VecLiteError::Corrupt("load: IDDIR slot out of range".to_owned()))?;
        ids[s] = Some(id.to_owned());
    }
    let mut payloads: Vec<Option<Value>> = vec![None; count];
    for (slot, value) in payload.entries {
        if let Ok(s) = usize::try_from(slot) {
            if s < count {
                payloads[s] = Some(value);
            }
        }
    }

    let mut points = Vec::with_capacity(count);
    for slot in 0..count {
        let id = ids[slot]
            .take()
            .ok_or_else(|| VecLiteError::Corrupt(format!("load: slot {slot} has no id")))?;
        let bytes = vectors
            .record(slot as u64)
            .ok_or_else(|| VecLiteError::Corrupt(format!("load: slot {slot} has no vector")))?;
        let vector: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        debug_assert_eq!(vector.len(), dim);
        points.push((id, vector, payloads[slot].take()));
    }
    Ok(LoadedCollection {
        options,
        points,
        base: None,
        vocab,
    })
}

/// `load_based`'s product: collection options plus slot → id / payload tables.
pub(crate) type BasedMeta = (CollectionOptions, Vec<Option<String>>, Vec<Option<Value>>);

/// Reconstruct a collection's config and slot metadata **without** its vectors
/// — the mmap-tier load (ADR-0004). `segments` carries every live segment
/// except VECTORS (those stay in the file map); `slot_count` is the total slot
/// range covered by the mapped regions. Slots absent from the IDDIR are dead
/// (`ids[slot] = None`) — with carry-forward a checkpoint may commit an
/// uncompacted base, so a sparse IDDIR is data, not corruption.
pub(crate) fn load_based(segments: &[Segment], slot_count: usize) -> Result<BasedMeta> {
    let mut stored: Option<StoredConfig> = None;
    let mut iddir: Option<IdDir> = None;
    let mut payload = PayloadBlock {
        entries: Vec::new(),
    };
    let mut declared: Vec<(String, PayloadIndexKind)> = Vec::new();
    for seg in segments {
        match seg.seg_type {
            SegmentType::Config => stored = Some(StoredConfig::decode(&seg.body)?),
            SegmentType::Iddir => iddir = Some(IdDir::decode(&seg.body)?),
            SegmentType::Payload => payload = PayloadBlock::decode(&seg.body)?,
            SegmentType::Pidx => {
                let pidx = PayloadIndex::decode(&seg.body)?;
                declared.push((pidx.key, config::pidx_kind_from(pidx.kind)?));
            }
            _ => {}
        }
    }
    let stored = stored.ok_or_else(|| VecLiteError::Corrupt("load: missing CONFIG".to_owned()))?;
    let iddir = iddir.ok_or_else(|| VecLiteError::Corrupt("load: missing IDDIR".to_owned()))?;
    let mut options = config::from_stored(&stored)?;
    options.payload_indexes = declared;

    let mut ids: Vec<Option<String>> = vec![None; slot_count];
    for (id, slot) in iddir.entries() {
        let s = usize::try_from(slot)
            .ok()
            .filter(|&s| s < slot_count)
            .ok_or_else(|| VecLiteError::Corrupt("load: IDDIR slot out of range".to_owned()))?;
        ids[s] = Some(id.to_owned());
    }
    let mut payloads: Vec<Option<Value>> = vec![None; slot_count];
    for (slot, value) in payload.entries {
        if let Ok(s) = usize::try_from(slot) {
            if s < slot_count {
                payloads[s] = Some(value);
            }
        }
    }
    Ok((options, ids, payloads))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::{Metric, Quantization};

    fn opts() -> CollectionOptions {
        CollectionOptions::new(3, Metric::Euclidean).quantization(Quantization::None)
    }

    #[test]
    fn seal_load_round_trip() {
        let live: Vec<LivePoint> = vec![
            (
                "a".into(),
                vec![1.0, 2.0, 3.0],
                Some(serde_json::json!({"k": 1})),
            ),
            ("b".into(), vec![4.0, 5.0, 6.0], None),
            (
                "c".into(),
                vec![7.0, 8.0, 9.0],
                Some(serde_json::json!("x")),
            ),
        ];
        let sealed = seal(0, "docs".into(), vec![], &opts(), &[], None, &live, 1000)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(sealed.vector_count, 3);
        let loaded = load(&sealed.segments).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loaded.options.dimension, 3);
        assert_eq!(loaded.points, live);
    }

    #[test]
    fn empty_collection_round_trips() {
        let sealed = seal(1, "empty".into(), vec![], &opts(), &[], None, &[], 1000)
            .unwrap_or_else(|e| panic!("{e}"));
        let loaded = load(&sealed.segments).unwrap_or_else(|e| panic!("{e}"));
        assert!(loaded.points.is_empty());
    }

    #[test]
    fn missing_config_is_corrupt() {
        assert!(matches!(load(&[]), Err(VecLiteError::Corrupt(_))));
    }
}
