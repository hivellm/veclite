//! Table of contents (SPEC-002 §4): the MessagePack document written as the
//! last data structure of every checkpoint, before the header swap. It lists
//! the live segments of every collection in replay order (STG-040/041) and a
//! monotonically increasing generation counter.

use serde::{Deserialize, Serialize};

use crate::error::{Result, VecLiteError};
use crate::storage::segment::SegmentType;

/// A reference to one live segment: its type tag and on-disk location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SegRef {
    pub(crate) seg_type: u8,
    pub(crate) offset: u64,
    pub(crate) len: u64,
}

impl SegRef {
    /// Sort key for deterministic replay (STG-041): `(replay_rank, offset)`.
    /// An unrecognized type sorts last so a corrupt TOC still yields a total
    /// order (the load path rejects the unknown type separately, STG-022).
    fn replay_key(&self) -> (u8, u64) {
        let rank = SegmentType::from_byte(self.seg_type)
            .map(SegmentType::replay_rank)
            .unwrap_or(u8::MAX);
        (rank, self.offset)
    }
}

/// Per-collection entry in the TOC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CollEntry {
    pub(crate) coll_id: u32,
    pub(crate) name: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) vector_count: u64,
    pub(crate) tombstone_count: u64,
    pub(crate) live_segments: Vec<SegRef>,
}

impl CollEntry {
    /// Sort `live_segments` into the deterministic replay order (STG-041). The
    /// writer calls this before committing so the stored order is canonical.
    pub(crate) fn sort_replay_order(&mut self) {
        self.live_segments.sort_by_key(SegRef::replay_key);
    }
}

/// The root TOC document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Toc {
    pub(crate) generation: u64,
    pub(crate) collections: Vec<CollEntry>,
    pub(crate) free_tail_offset: u64,
}

impl Toc {
    /// MessagePack-encode (compact, positional — deterministic for golden
    /// files). OQ-5: MessagePack everywhere.
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        rmp_serde::to_vec(self)
            .map_err(|e| VecLiteError::Corrupt(format!("toc: encode failed: {e}")))
    }

    /// Decode a TOC body; any malformation is `Corrupt("toc")` (STG-051).
    pub(crate) fn decode(bytes: &[u8]) -> Result<Toc> {
        // Bound container nesting before rmp-serde recurses through it (a
        // fuzzed TOC nests arrays where fixed fields are expected — SPEC-015).
        crate::storage::body::guard_msgpack_depth(bytes)?;
        rmp_serde::from_slice(bytes).map_err(|e| VecLiteError::Corrupt(format!("toc: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Toc {
        Toc {
            generation: 7,
            free_tail_offset: 12_345,
            collections: vec![CollEntry {
                coll_id: 0,
                name: "docs".into(),
                aliases: vec!["d".into()],
                vector_count: 100,
                tombstone_count: 3,
                live_segments: vec![
                    SegRef {
                        seg_type: SegmentType::Hnsw.to_byte(),
                        offset: 900,
                        len: 50,
                    },
                    SegRef {
                        seg_type: SegmentType::Config.to_byte(),
                        offset: 4096,
                        len: 40,
                    },
                    SegRef {
                        seg_type: SegmentType::Vectors.to_byte(),
                        offset: 500,
                        len: 200,
                    },
                    SegRef {
                        seg_type: SegmentType::Iddir.to_byte(),
                        offset: 800,
                        len: 30,
                    },
                ],
            }],
        }
    }

    #[test]
    fn round_trip() {
        let toc = sample();
        let bytes = toc.encode().unwrap_or_else(|e| panic!("{e}"));
        let back = Toc::decode(&bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, toc);
    }

    #[test]
    fn encoding_is_deterministic() {
        let toc = sample();
        assert_eq!(
            toc.encode().unwrap_or_else(|e| panic!("{e}")),
            toc.encode().unwrap_or_else(|e| panic!("{e}"))
        );
    }

    #[test]
    fn replay_order_is_config_iddir_vectors_hnsw() {
        let mut e = sample().collections.remove(0);
        e.sort_replay_order();
        let ranks: Vec<u8> = e.live_segments.iter().map(|s| s.seg_type).collect();
        assert_eq!(
            ranks,
            [
                SegmentType::Config.to_byte(),
                SegmentType::Iddir.to_byte(),
                SegmentType::Vectors.to_byte(),
                SegmentType::Hnsw.to_byte(),
            ]
        );
    }

    #[test]
    fn garbage_is_corrupt_not_panic() {
        assert!(matches!(
            Toc::decode(&[0xC1, 0xFF, 0x00, 0x99]),
            Err(VecLiteError::Corrupt(_))
        ));
    }
}
