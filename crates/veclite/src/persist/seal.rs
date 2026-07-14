//! Sealing and loading a collection's live state to/from segments (SPEC-002
//! §5). A checkpoint seals the compacted live set (dead slots dropped —
//! space reclaimed as a side effect) into CONFIG + VECTORS(f32) + IDDIR +
//! PAYLOAD segments; load reverses it. The HNSW graph is not persisted in v1 —
//! it is rebuilt from the vectors on load (STG-063). Sparse/vocab/PIDX
//! persistence lands with their features (phase3).

use serde_json::Value;

use crate::error::{Result, VecLiteError};
use crate::options::CollectionOptions;
use crate::persist::config;
use crate::storage::body::{PayloadBlock, StoredConfig};
use crate::storage::iddir::IdDir;
use crate::storage::pager::CheckpointColl;
use crate::storage::segment::{Segment, SegmentType};
use crate::storage::vectors::{Encoding, VectorsBody};

/// One live point: id, dense vector, optional payload. Sparse is not yet
/// persisted (phase3c).
pub(crate) type LivePoint = (String, Vec<f32>, Option<Value>);

/// A collection reconstructed from its segments.
pub(crate) struct LoadedCollection {
    pub(crate) options: CollectionOptions,
    pub(crate) points: Vec<LivePoint>,
}

/// Bucket count heuristic: ~1 entry/bucket, clamped to a sane floor.
fn bucket_count(n: usize) -> usize {
    n.next_power_of_two().max(8)
}

/// Seal a collection's live set into a `CheckpointColl` ready for the pager
/// (SPEC-002 §5). Slots are compacted to `0..live.len()`.
pub(crate) fn seal(
    coll_id: u32,
    name: String,
    aliases: Vec<String>,
    options: &CollectionOptions,
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

    Ok(CheckpointColl {
        coll_id,
        name,
        aliases,
        vector_count: live.len() as u64,
        tombstone_count: 0,
        segments,
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
    for seg in segments {
        match seg.seg_type {
            SegmentType::Config => stored = Some(StoredConfig::decode(&seg.body)?),
            SegmentType::Vectors => vectors = Some(VectorsBody::decode(&seg.body)?),
            SegmentType::Iddir => iddir = Some(IdDir::decode(&seg.body)?),
            SegmentType::Payload => payload = PayloadBlock::decode(&seg.body)?,
            // HNSW is rebuilt from vectors (STG-063); other types are not yet
            // produced by seal.
            _ => {}
        }
    }
    let stored = stored.ok_or_else(|| VecLiteError::Corrupt("load: missing CONFIG".to_owned()))?;
    let vectors =
        vectors.ok_or_else(|| VecLiteError::Corrupt("load: missing VECTORS".to_owned()))?;
    let iddir = iddir.ok_or_else(|| VecLiteError::Corrupt("load: missing IDDIR".to_owned()))?;
    let options = config::from_stored(&stored)?;
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
    Ok(LoadedCollection { options, points })
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
        let sealed =
            seal(0, "docs".into(), vec![], &opts(), &live, 1000).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(sealed.vector_count, 3);
        let loaded = load(&sealed.segments).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(loaded.options.dimension, 3);
        assert_eq!(loaded.points, live);
    }

    #[test]
    fn empty_collection_round_trips() {
        let sealed =
            seal(1, "empty".into(), vec![], &opts(), &[], 1000).unwrap_or_else(|e| panic!("{e}"));
        let loaded = load(&sealed.segments).unwrap_or_else(|e| panic!("{e}"));
        assert!(loaded.points.is_empty());
    }

    #[test]
    fn missing_config_is_corrupt() {
        assert!(matches!(load(&[]), Err(VecLiteError::Corrupt(_))));
    }
}
