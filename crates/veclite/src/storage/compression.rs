//! Block compression for segment bodies (SPEC-002 §3, STG-020/031). Adapted
//! from hivellm/vectorizer vectorizer-core@3.5.0 (crates/vectorizer-core/src/
//! compression/{lz4,zstd}.rs), Apache-2.0 — the same `lz4_flex`
//! `compress_prepend_size` framing and `zstd` level-3 stream encoding, so the
//! bytes stay compatible with the server's `.vecdb` (SPEC-013 interop).
//!
//! VECTORS segments are never compressed (STG-031); everything else may be.

use crate::error::{Result, VecLiteError};

/// The `compression` byte stored in a segment header (SPEC-002 §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Codec {
    /// Stored uncompressed (0).
    None,
    /// LZ4 via `lz4_flex`, size-prepended (1).
    Lz4,
    /// Zstandard, level 3 (2).
    Zstd,
}

impl Codec {
    pub(crate) fn from_byte(b: u8) -> Result<Codec> {
        match b {
            0 => Ok(Codec::None),
            1 => Ok(Codec::Lz4),
            2 => Ok(Codec::Zstd),
            other => Err(VecLiteError::Corrupt(format!(
                "segment: unknown compression codec {other}"
            ))),
        }
    }

    pub(crate) fn to_byte(self) -> u8 {
        match self {
            Codec::None => 0,
            Codec::Lz4 => 1,
            Codec::Zstd => 2,
        }
    }
}

/// Default zstd level, matching the server (SPEC-002 §3 parity).
const ZSTD_LEVEL: i32 = 3;

/// Compress `data` with `codec`. `None` is the identity.
pub(crate) fn compress(codec: Codec, data: &[u8]) -> Result<Vec<u8>> {
    match codec {
        Codec::None => Ok(data.to_vec()),
        Codec::Lz4 => Ok(lz4_flex::compress_prepend_size(data)),
        Codec::Zstd => zstd::stream::encode_all(data, ZSTD_LEVEL)
            .map_err(|e| VecLiteError::Corrupt(format!("zstd compress: {e}"))),
    }
}

/// Decompress `data` with `codec`, validating that the result is exactly
/// `uncompressed_len` bytes (the length declared in the segment header). A
/// mismatch or a decoder error is `Corrupt` — never a panic (STG-021).
pub(crate) fn decompress(codec: Codec, data: &[u8], uncompressed_len: usize) -> Result<Vec<u8>> {
    let out = match codec {
        Codec::None => data.to_vec(),
        Codec::Lz4 => lz4_flex::decompress_size_prepended(data)
            .map_err(|e| VecLiteError::Corrupt(format!("lz4 decompress: {e}")))?,
        Codec::Zstd => zstd::stream::decode_all(data)
            .map_err(|e| VecLiteError::Corrupt(format!("zstd decompress: {e}")))?,
    };
    if out.len() != uncompressed_len {
        return Err(VecLiteError::Corrupt(format!(
            "segment: decompressed length {} != declared {uncompressed_len}",
            out.len()
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Vec<u8> {
        // Compressible: repeated structure so lz4/zstd actually shrink it.
        (0..4096u32).flat_map(|i| (i % 17).to_le_bytes()).collect()
    }

    #[test]
    fn round_trip_every_codec() {
        let data = payload();
        for codec in [Codec::None, Codec::Lz4, Codec::Zstd] {
            let packed = compress(codec, &data).unwrap_or_else(|e| panic!("{e}"));
            let back = decompress(codec, &packed, data.len()).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back, data, "codec {codec:?}");
        }
    }

    #[test]
    fn compression_actually_shrinks() {
        let data = payload();
        assert!(
            compress(Codec::Lz4, &data)
                .unwrap_or_else(|e| panic!("{e}"))
                .len()
                < data.len()
        );
        assert!(
            compress(Codec::Zstd, &data)
                .unwrap_or_else(|e| panic!("{e}"))
                .len()
                < data.len()
        );
    }

    #[test]
    fn wrong_declared_length_is_corrupt() {
        let data = payload();
        let packed = compress(Codec::Lz4, &data).unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            decompress(Codec::Lz4, &packed, data.len() + 1),
            Err(VecLiteError::Corrupt(_))
        ));
    }

    #[test]
    fn codec_byte_round_trip() {
        for codec in [Codec::None, Codec::Lz4, Codec::Zstd] {
            assert_eq!(
                Codec::from_byte(codec.to_byte()).unwrap_or_else(|e| panic!("{e}")),
                codec
            );
        }
        assert!(matches!(Codec::from_byte(9), Err(VecLiteError::Corrupt(_))));
    }
}
