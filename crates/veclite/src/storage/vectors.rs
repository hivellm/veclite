//! VECTORS segment body (SPEC-002 §3.2, STG-030/031).
//!
//! Fixed-stride vector block, laid out for direct mmap access: the byte
//! offset of any vector is computed from its slot with no decoding required
//! (STG-004, STG-030). VECTORS segments are never compressed (STG-031); this
//! module only frames the body layout, not segment compression/CRC — that is
//! `segment.rs`'s job.

use crate::error::{Result, VecLiteError};
use crate::storage::le;

/// Vector encoding tag stored in the body header (SPEC-002 §3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoding {
    /// Raw `f32` components, `dimension * 4` bytes per record.
    F32,
    /// Scalar quantization, 8 bits per component.
    Sq8,
    /// Scalar quantization, 4 bits per component (packed 2/byte).
    Sq4,
    /// Scalar quantization, 2 bits per component (packed 4/byte).
    Sq2,
    /// Scalar quantization, 1 bit per component (packed 8/byte).
    Sq1,
    /// 1-bit binary codes, `dimension / 8` bytes per record.
    Binary,
    /// Product quantization (feature `pq`, not decodable by this build).
    Pq,
}

impl Encoding {
    pub(crate) fn to_byte(self) -> u8 {
        match self {
            Encoding::F32 => 0,
            Encoding::Sq8 => 1,
            Encoding::Sq4 => 2,
            Encoding::Sq2 => 3,
            Encoding::Sq1 => 4,
            Encoding::Binary => 5,
            Encoding::Pq => 6,
        }
    }

    /// Parse an `encoding` byte. Unknown values are `Corrupt` — there is no
    /// silent fallback (SPEC-001 CORE-012 spirit applied to storage).
    pub(crate) fn from_byte(b: u8) -> Result<Encoding> {
        Ok(match b {
            0 => Encoding::F32,
            1 => Encoding::Sq8,
            2 => Encoding::Sq4,
            3 => Encoding::Sq2,
            4 => Encoding::Sq1,
            5 => Encoding::Binary,
            6 => Encoding::Pq,
            other => {
                return Err(VecLiteError::Corrupt(format!(
                    "vectors: unknown encoding {other}"
                )));
            }
        })
    }

    /// Scalar-quantized encodings carry a `(scale, offset)` pair after the
    /// fixed body header (SPEC-002 §3.2).
    fn has_sq_params(self) -> bool {
        matches!(
            self,
            Encoding::Sq8 | Encoding::Sq4 | Encoding::Sq2 | Encoding::Sq1
        )
    }
}

/// Fixed body header: `encoding(1) + dimension(4) + count(8) + first_slot(8)`.
const HEADER_LEN: usize = 1 + 4 + 8 + 8;
/// `scale f32, offset f32` — present only for sq* encodings.
const SQ_PARAMS_LEN: usize = 4 + 4;

/// Parsed VECTORS segment body (SPEC-002 §3.2).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VectorsBody {
    pub(crate) encoding: Encoding,
    pub(crate) dimension: u32,
    pub(crate) first_slot: u64,
    pub(crate) count: u64,
    pub(crate) sq_params: Option<(f32, f32)>,
    pub(crate) records: Vec<u8>,
}

impl VectorsBody {
    /// Bytes per record for this body's encoding and dimension (SPEC-002
    /// §3.2). `Binary` requires `dimension` to be a multiple of 8.
    pub(crate) fn stride(&self) -> Result<usize> {
        stride_for(self.encoding, self.dimension)
    }

    /// The raw record bytes for `slot`, or `None` if it falls outside
    /// `first_slot .. first_slot + count` (STG-030 slot addressing — no
    /// decode required to locate a vector).
    pub(crate) fn record(&self, slot: u64) -> Option<&[u8]> {
        let index = slot.checked_sub(self.first_slot)?;
        if index >= self.count {
            return None;
        }
        let stride = self.stride().ok()?;
        let start = usize::try_from(index).ok()?.checked_mul(stride)?;
        let end = start.checked_add(stride)?;
        self.records.get(start..end)
    }

    /// Frame the body: header, optional sq params, then the raw records.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + SQ_PARAMS_LEN + self.records.len());
        out.push(self.encoding.to_byte());
        out.extend_from_slice(&self.dimension.to_le_bytes());
        out.extend_from_slice(&self.count.to_le_bytes());
        out.extend_from_slice(&self.first_slot.to_le_bytes());
        if let Some((scale, offset)) = self.sq_params {
            out.extend_from_slice(&scale.to_bits().to_le_bytes());
            out.extend_from_slice(&offset.to_bits().to_le_bytes());
        }
        out.extend_from_slice(&self.records);
        out
    }

    /// Parse a VECTORS body; any malformation is `Corrupt("vectors: ...")`,
    /// never a panic. `Pq` bodies are rejected as unsupported — product
    /// quantization decoding is gated behind the `pq` feature and out of
    /// scope for this codec (SPEC-002 §3.2).
    pub(crate) fn decode(bytes: &[u8]) -> Result<VectorsBody> {
        let encoding_byte = *bytes
            .first()
            .ok_or_else(|| VecLiteError::Corrupt("vectors: empty body".to_owned()))?;
        let encoding = Encoding::from_byte(encoding_byte)?;
        if encoding == Encoding::Pq {
            return Err(VecLiteError::UnsupportedProvider {
                requested: "pq".to_owned(),
                available: Vec::new(),
            });
        }
        let dimension = le::u32(bytes, 1, "vectors")?;
        let count = le::u64(bytes, 5, "vectors")?;
        let first_slot = le::u64(bytes, 13, "vectors")?;

        let mut at = HEADER_LEN;
        let sq_params = if encoding.has_sq_params() {
            let scale = f32::from_bits(le::u32(bytes, at, "vectors")?);
            let offset = f32::from_bits(le::u32(bytes, at + 4, "vectors")?);
            at += SQ_PARAMS_LEN;
            Some((scale, offset))
        } else {
            None
        };

        let stride = stride_for(encoding, dimension)?;
        let count_usize = usize::try_from(count)
            .map_err(|_| VecLiteError::Corrupt("vectors: count exceeds usize".to_owned()))?;
        let want = stride
            .checked_mul(count_usize)
            .ok_or_else(|| VecLiteError::Corrupt("vectors: record size overflow".to_owned()))?;
        let records = bytes
            .get(at..)
            .ok_or_else(|| VecLiteError::Corrupt("vectors: truncated header".to_owned()))?
            .to_vec();
        if records.len() != want {
            return Err(VecLiteError::Corrupt(format!(
                "vectors: expected {want} record bytes, got {}",
                records.len()
            )));
        }
        Ok(VectorsBody {
            encoding,
            dimension,
            first_slot,
            count,
            sq_params,
            records,
        })
    }
}

/// Stride in bytes for one record of `encoding` at `dimension` (SPEC-002
/// §3.2). `Pq` is rejected by the caller before this is reached.
fn stride_for(encoding: Encoding, dimension: u32) -> Result<usize> {
    let dim = usize::try_from(dimension)
        .map_err(|_| VecLiteError::Corrupt("vectors: dimension exceeds usize".to_owned()))?;
    Ok(match encoding {
        Encoding::F32 => dim
            .checked_mul(4)
            .ok_or_else(|| VecLiteError::Corrupt("vectors: stride overflow".to_owned()))?,
        Encoding::Sq8 => dim,
        Encoding::Sq4 => dim.div_ceil(2),
        Encoding::Sq2 => dim.div_ceil(4),
        Encoding::Sq1 => dim.div_ceil(8),
        Encoding::Binary => {
            if dim % 8 != 0 {
                return Err(VecLiteError::Corrupt(format!(
                    "vectors: binary dimension {dim} not a multiple of 8"
                )));
            }
            dim / 8
        }
        Encoding::Pq => {
            return Err(VecLiteError::Corrupt(
                "vectors: pq stride requires the pq feature".to_owned(),
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_f32_and_slot_addressing() {
        let dimension = 4u32;
        let first_slot = 10u64;
        let floats: Vec<f32> = (0..8u32).map(|i| i as f32).collect();
        let records: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect(); // 2 records
        let body = VectorsBody {
            encoding: Encoding::F32,
            dimension,
            first_slot,
            count: 2,
            sq_params: None,
            records,
        };
        let bytes = body.encode();
        let back = VectorsBody::decode(&bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, body);
        assert_eq!(back.stride().unwrap_or_else(|e| panic!("{e}")), 16);
        // Slot addressing: first_slot maps to the first record, no decode.
        let first_record: Vec<u8> = floats[0..4].iter().flat_map(|f| f.to_le_bytes()).collect();
        let second_record: Vec<u8> = floats[4..8].iter().flat_map(|f| f.to_le_bytes()).collect();
        assert_eq!(back.record(10), Some(&first_record[..]));
        assert_eq!(back.record(11), Some(&second_record[..]));
        assert!(back.record(12).is_none()); // past first_slot + count
        assert!(back.record(9).is_none()); // before first_slot
    }

    #[test]
    fn round_trip_sq8_with_scale_offset_params() {
        let body = VectorsBody {
            encoding: Encoding::Sq8,
            dimension: 3,
            first_slot: 0,
            count: 2,
            sq_params: Some((0.5, -1.0)),
            records: vec![1, 2, 3, 4, 5, 6],
        };
        let bytes = body.encode();
        let back = VectorsBody::decode(&bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, body);
        assert_eq!(back.record(1), Some(&[4u8, 5, 6][..]));
    }

    #[test]
    fn wrong_record_length_is_corrupt() {
        let body = VectorsBody {
            encoding: Encoding::Sq8,
            dimension: 3,
            first_slot: 0,
            count: 2,
            sq_params: Some((1.0, 0.0)),
            records: vec![1, 2, 3, 4, 5], // one byte short of 6
        };
        let bytes = body.encode();
        assert!(matches!(
            VectorsBody::decode(&bytes),
            Err(VecLiteError::Corrupt(_))
        ));
    }

    #[test]
    fn binary_dimension_must_be_multiple_of_8() {
        let body = VectorsBody {
            encoding: Encoding::Binary,
            dimension: 12, // not a multiple of 8
            first_slot: 0,
            count: 1,
            sq_params: None,
            records: vec![0xFF, 0x0F],
        };
        let bytes = body.encode();
        assert!(matches!(
            VectorsBody::decode(&bytes),
            Err(VecLiteError::Corrupt(_))
        ));
    }

    #[test]
    fn pq_encoding_is_unsupported_provider() {
        let mut bytes = vec![Encoding::Pq.to_byte()];
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        assert!(matches!(
            VectorsBody::decode(&bytes),
            Err(VecLiteError::UnsupportedProvider { .. })
        ));
    }

    #[test]
    fn truncated_body_is_corrupt_not_panic() {
        assert!(matches!(
            VectorsBody::decode(&[Encoding::F32.to_byte(), 1, 2]),
            Err(VecLiteError::Corrupt(_))
        ));
        assert!(matches!(
            VectorsBody::decode(&[]),
            Err(VecLiteError::Corrupt(_))
        ));
    }
}
