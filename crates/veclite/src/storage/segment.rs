//! Segment framing (SPEC-002 §3): the 32-byte segment header, compression, and
//! the per-segment CRC. This layer is body-agnostic — it frames an opaque body
//! blob; the per-type body codecs (CONFIG, VECTORS, …) live in their own
//! modules and hand their encoded bytes here.

use crate::error::{Result, VecLiteError};
use crate::storage::compression::{self, Codec};
use crate::storage::le;

/// Every segment starts with this fixed 32-byte header (SPEC-002 §3).
pub(crate) const SEGMENT_HEADER_SIZE: usize = 32;
/// `coll_id` sentinel for database-scope segments (SPEC-002 §3).
pub(crate) const DATABASE_SCOPE: u32 = 0xFFFF_FFFF;
/// Bodies below this SHOULD NOT be compressed (STG-020, server parity).
pub(crate) const COMPRESS_THRESHOLD: usize = 1024;
/// `seg_flags` bit 15 — an advisory segment a v1 reader may skip (STG-022).
pub(crate) const FLAG_IGNORABLE: u16 = 1 << 15;

/// Segment type tags (SPEC-002 §3.1). The order of variants is not the replay
/// order — that is STG-041, applied by the TOC loader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SegmentType {
    Config,
    Vectors,
    Tombstone,
    Payload,
    Pidx,
    Sparse,
    Hnsw,
    Vocab,
    Iddir,
}

impl SegmentType {
    pub(crate) fn to_byte(self) -> u8 {
        match self {
            SegmentType::Config => 1,
            SegmentType::Vectors => 2,
            SegmentType::Tombstone => 3,
            SegmentType::Payload => 4,
            SegmentType::Pidx => 5,
            SegmentType::Sparse => 6,
            SegmentType::Hnsw => 7,
            SegmentType::Vocab => 8,
            SegmentType::Iddir => 9,
        }
    }

    /// Deterministic replay rank (STG-041): CONFIG, IDDIR, VECTORS, TOMBSTONE,
    /// PAYLOAD, PIDX, SPARSE, VOCAB, HNSW. Segments of the same rank keep append
    /// order (by offset). The TOC loader sorts a collection's segments by
    /// `(replay_rank, offset)` before reconstructing its state.
    pub(crate) fn replay_rank(self) -> u8 {
        match self {
            SegmentType::Config => 0,
            SegmentType::Iddir => 1,
            SegmentType::Vectors => 2,
            SegmentType::Tombstone => 3,
            SegmentType::Payload => 4,
            SegmentType::Pidx => 5,
            SegmentType::Sparse => 6,
            SegmentType::Vocab => 7,
            SegmentType::Hnsw => 8,
        }
    }

    /// Parse a `seg_type`. Unknown non-ignorable types are `Corrupt` (STG-022);
    /// the caller checks the ignorable flag before treating an unknown type as
    /// fatal.
    pub(crate) fn from_byte(b: u8) -> Result<SegmentType> {
        Ok(match b {
            1 => SegmentType::Config,
            2 => SegmentType::Vectors,
            3 => SegmentType::Tombstone,
            4 => SegmentType::Payload,
            5 => SegmentType::Pidx,
            6 => SegmentType::Sparse,
            7 => SegmentType::Hnsw,
            8 => SegmentType::Vocab,
            9 => SegmentType::Iddir,
            other => {
                return Err(VecLiteError::Corrupt(format!(
                    "segment: unknown seg_type {other}"
                )));
            }
        })
    }
}

/// A framed segment ready to append, or one just read from the file. `body` is
/// always the **decompressed** body; framing/CRC/compression are handled by
/// [`encode`](Segment::encode) / [`read`](Segment::read).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Segment {
    pub(crate) seg_type: SegmentType,
    pub(crate) seg_flags: u16,
    pub(crate) coll_id: u32,
    pub(crate) body: Vec<u8>,
}

/// Choose the codec for a body: never for VECTORS (STG-031) or tiny bodies
/// (STG-020), otherwise the requested codec.
pub(crate) fn codec_for(seg_type: SegmentType, requested: Codec, body_len: usize) -> Codec {
    if seg_type == SegmentType::Vectors || body_len < COMPRESS_THRESHOLD {
        Codec::None
    } else {
        requested
    }
}

impl Segment {
    /// Frame the segment: compress the body with `codec`, CRC the stored bytes,
    /// and prepend the 32-byte header. Returns the full on-disk segment.
    pub(crate) fn encode(&self, codec: Codec) -> Result<Vec<u8>> {
        let stored = compression::compress(codec, &self.body)?;
        let body_crc32 = crc32fast::hash(&stored);
        let mut out = Vec::with_capacity(SEGMENT_HEADER_SIZE + stored.len());
        out.push(self.seg_type.to_byte());
        out.push(codec.to_byte());
        out.extend_from_slice(&self.seg_flags.to_le_bytes());
        out.extend_from_slice(&self.coll_id.to_le_bytes());
        out.extend_from_slice(&(stored.len() as u64).to_le_bytes());
        out.extend_from_slice(&(self.body.len() as u64).to_le_bytes());
        out.extend_from_slice(&body_crc32.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // reserved
        out.extend_from_slice(&stored);
        Ok(out)
    }

    /// Parse the segment whose header starts at `buf[at]`. `file_offset` is the
    /// absolute offset used in error messages (STG-021). Verifies the body CRC
    /// before decompressing. Returns the segment and its total on-disk length
    /// (header + stored body) so callers can walk to the next segment.
    pub(crate) fn read(buf: &[u8], at: usize, file_offset: u64) -> Result<(Segment, usize)> {
        let loc = || format!("segment@{file_offset}");
        if at + SEGMENT_HEADER_SIZE > buf.len() {
            return Err(VecLiteError::Corrupt(format!(
                "{}: truncated header",
                loc()
            )));
        }
        let seg_type = SegmentType::from_byte(buf[at])?;
        let codec = Codec::from_byte(buf[at + 1])?;
        let seg_flags = le::u16(buf, at + 2, "segment")?;
        let coll_id = le::u32(buf, at + 4, "segment")?;
        let body_len = le::u64(buf, at + 8, "segment")? as usize;
        let uncompressed_len = le::u64(buf, at + 16, "segment")? as usize;
        let body_crc32 = le::u32(buf, at + 24, "segment")?;

        let body_start = at + SEGMENT_HEADER_SIZE;
        let body_end = body_start
            .checked_add(body_len)
            .filter(|&e| e <= buf.len())
            .ok_or_else(|| VecLiteError::Corrupt(format!("{}: body past end of file", loc())))?;
        let stored = &buf[body_start..body_end];
        if crc32fast::hash(stored) != body_crc32 {
            return Err(VecLiteError::Corrupt(format!(
                "{}: body crc mismatch",
                loc()
            )));
        }
        let body = compression::decompress(codec, stored, uncompressed_len)
            .map_err(|_| VecLiteError::Corrupt(format!("{}: body decode failed", loc())))?;
        Ok((
            Segment {
                seg_type,
                seg_flags,
                coll_id,
                body,
            },
            SEGMENT_HEADER_SIZE + body_len,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(body: Vec<u8>) -> Segment {
        Segment {
            seg_type: SegmentType::Payload,
            seg_flags: 0,
            coll_id: 42,
            body,
        }
    }

    #[test]
    fn round_trip_uncompressed_and_compressed() {
        let big: Vec<u8> = (0..8192u32).flat_map(|i| (i % 13).to_le_bytes()).collect();
        for codec in [Codec::None, Codec::Lz4, Codec::Zstd] {
            let s = seg(big.clone());
            let bytes = s.encode(codec).unwrap_or_else(|e| panic!("{e}"));
            let (back, total) = Segment::read(&bytes, 0, 0).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, s, "codec {codec:?}");
            assert_eq!(total, bytes.len());
        }
    }

    #[test]
    fn body_crc_detects_corruption() {
        let s = seg((0..2000u32).flat_map(|i| (i % 7).to_le_bytes()).collect());
        let mut bytes = s.encode(Codec::Lz4).unwrap_or_else(|e| panic!("{e}"));
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        let Err(VecLiteError::Corrupt(m)) = Segment::read(&bytes, 42, 4096) else {
            panic!("expected Corrupt")
        };
        assert!(m.contains("segment@4096"), "message was {m}");
    }

    #[test]
    fn codec_policy_forces_vectors_uncompressed_and_skips_tiny() {
        assert_eq!(
            codec_for(SegmentType::Vectors, Codec::Zstd, 1 << 20),
            Codec::None
        );
        assert_eq!(codec_for(SegmentType::Payload, Codec::Lz4, 10), Codec::None);
        assert_eq!(
            codec_for(SegmentType::Payload, Codec::Lz4, 4096),
            Codec::Lz4
        );
    }

    #[test]
    fn unknown_seg_type_is_corrupt() {
        assert!(matches!(
            SegmentType::from_byte(200),
            Err(VecLiteError::Corrupt(_))
        ));
    }

    #[test]
    fn truncated_body_is_corrupt_not_panic() {
        let s = seg((0..2000u32).flat_map(|i| (i % 7).to_le_bytes()).collect());
        let bytes = s.encode(Codec::Lz4).unwrap_or_else(|e| panic!("{e}"));
        // Cut the body short.
        assert!(matches!(
            Segment::read(&bytes[..bytes.len() - 5], 0, 0),
            Err(VecLiteError::Corrupt(_))
        ));
    }
}
