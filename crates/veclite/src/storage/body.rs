//! MessagePack- and roaring-encoded segment bodies (SPEC-002 §3.1): CONFIG,
//! TOMBSTONE, PAYLOAD, PIDX, SPARSE, VOCAB, HNSW. Each is a self-contained
//! on-disk projection; the live engine maps its runtime types to and from
//! these. Slot bitmaps use a 64-bit roaring treemap (portable serialization) to
//! match the u64 slot space.

use std::io::Cursor;

use roaring::RoaringTreemap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Result, VecLiteError};
use crate::storage::le;

fn corrupt(what: &str, e: impl std::fmt::Display) -> VecLiteError {
    VecLiteError::Corrupt(format!("{what}: {e}"))
}

fn slice<'a>(b: &'a [u8], at: usize, len: usize, what: &str) -> Result<&'a [u8]> {
    at.checked_add(len)
        .filter(|&end| end <= b.len())
        .map(|end| &b[at..end])
        .ok_or_else(|| VecLiteError::Corrupt(format!("{what}: field past end of body")))
}

// ── CONFIG (seg_type 1) ──────────────────────────────────────────────────

/// On-disk collection config (SPEC-002 §3.1). A flat projection of the runtime
/// `CollectionOptions`, MessagePack-encoded (OQ-5); kept independent of the
/// options types so the format is decoupled from the public API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredConfig {
    pub(crate) dimension: u32,
    pub(crate) metric: u8,
    pub(crate) m: u32,
    pub(crate) ef_construction: u32,
    pub(crate) ef_search: u32,
    pub(crate) quantization: u8,
    pub(crate) quant_bits: u8,
    pub(crate) compression: u8,
    pub(crate) embedding_provider: Option<String>,
    pub(crate) created_epoch_s: u64,
}

impl StoredConfig {
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        rmp_serde::to_vec(self).map_err(|e| corrupt("config", e))
    }
    pub(crate) fn decode(b: &[u8]) -> Result<StoredConfig> {
        rmp_serde::from_slice(b).map_err(|e| corrupt("config", e))
    }
}

// ── TOMBSTONE (seg_type 3) ───────────────────────────────────────────────

/// Portable roaring serialization of the deleted slot set.
pub(crate) fn encode_tombstone(slots: &RoaringTreemap) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    slots
        .serialize_into(&mut out)
        .map_err(|e| corrupt("tombstone", e))?;
    Ok(out)
}

pub(crate) fn decode_tombstone(b: &[u8]) -> Result<RoaringTreemap> {
    RoaringTreemap::deserialize_from(Cursor::new(b)).map_err(|e| corrupt("tombstone", e))
}

// ── PAYLOAD (seg_type 4) ─────────────────────────────────────────────────

/// Payload block: `(slot u64, len u32, msgpack payload)` records (SPEC-002
/// §3.1).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PayloadBlock {
    pub(crate) entries: Vec<(u64, Value)>,
}

impl PayloadBlock {
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for (slot, value) in &self.entries {
            out.extend_from_slice(&slot.to_le_bytes());
            let mp = rmp_serde::to_vec(value).map_err(|e| corrupt("payload", e))?;
            let len = u32::try_from(mp.len())
                .map_err(|_| VecLiteError::Corrupt("payload: entry exceeds 4 GiB".to_owned()))?;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&mp);
        }
        Ok(out)
    }

    pub(crate) fn decode(b: &[u8]) -> Result<PayloadBlock> {
        let mut at = 0;
        let mut entries = Vec::new();
        while at < b.len() {
            let slot = le::u64(b, at, "payload")?;
            at += 8;
            let len = le::u32(b, at, "payload")? as usize;
            at += 4;
            let bytes = slice(b, at, len, "payload")?;
            let value: Value = rmp_serde::from_slice(bytes).map_err(|e| corrupt("payload", e))?;
            at += len;
            entries.push((slot, value));
        }
        Ok(PayloadBlock { entries })
    }
}

// ── PIDX (seg_type 5) ────────────────────────────────────────────────────

/// A payload index: a kind, its key, and `value → slot bitmap` postings sorted
/// by the caller. Values are opaque bytes (the query layer encodes keyword /
/// int / float order into them).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PayloadIndex {
    pub(crate) kind: u8,
    pub(crate) key: String,
    pub(crate) postings: Vec<(Vec<u8>, RoaringTreemap)>,
}

impl PayloadIndex {
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        out.push(self.kind);
        let key_len = u16::try_from(self.key.len())
            .map_err(|_| VecLiteError::Corrupt("pidx: key exceeds 64 KiB".to_owned()))?;
        out.extend_from_slice(&key_len.to_le_bytes());
        out.extend_from_slice(self.key.as_bytes());
        let count = u32::try_from(self.postings.len())
            .map_err(|_| VecLiteError::Corrupt("pidx: too many postings".to_owned()))?;
        out.extend_from_slice(&count.to_le_bytes());
        for (value, bitmap) in &self.postings {
            let vl = u32::try_from(value.len())
                .map_err(|_| VecLiteError::Corrupt("pidx: value too long".to_owned()))?;
            out.extend_from_slice(&vl.to_le_bytes());
            out.extend_from_slice(value);
            let mut bm = Vec::new();
            bitmap
                .serialize_into(&mut bm)
                .map_err(|e| corrupt("pidx", e))?;
            let bl = u32::try_from(bm.len())
                .map_err(|_| VecLiteError::Corrupt("pidx: bitmap too large".to_owned()))?;
            out.extend_from_slice(&bl.to_le_bytes());
            out.extend_from_slice(&bm);
        }
        Ok(out)
    }

    pub(crate) fn decode(b: &[u8]) -> Result<PayloadIndex> {
        let kind = *b
            .first()
            .ok_or_else(|| VecLiteError::Corrupt("pidx: empty body".to_owned()))?;
        let key_len = le::u16(b, 1, "pidx")? as usize;
        let mut at = 3;
        let key = String::from_utf8(slice(b, at, key_len, "pidx")?.to_vec())
            .map_err(|_| VecLiteError::Corrupt("pidx: key not utf-8".to_owned()))?;
        at += key_len;
        let count = le::u32(b, at, "pidx")? as usize;
        at += 4;
        let mut postings = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let vl = le::u32(b, at, "pidx")? as usize;
            at += 4;
            let value = slice(b, at, vl, "pidx")?.to_vec();
            at += vl;
            let bl = le::u32(b, at, "pidx")? as usize;
            at += 4;
            let bitmap = RoaringTreemap::deserialize_from(Cursor::new(slice(b, at, bl, "pidx")?))
                .map_err(|e| corrupt("pidx", e))?;
            at += bl;
            postings.push((value, bitmap));
        }
        Ok(PayloadIndex {
            kind,
            key,
            postings,
        })
    }
}

// ── SPARSE (seg_type 6) ──────────────────────────────────────────────────

/// Sparse postings for the hybrid lane (SPEC-002 §3.1, SPEC-007): `term_id →
/// (slot, weight)` lists.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SparsePostings {
    pub(crate) terms: Vec<(u32, Vec<(u64, f32)>)>,
}

impl SparsePostings {
    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let tc = u32::try_from(self.terms.len())
            .map_err(|_| VecLiteError::Corrupt("sparse: too many terms".to_owned()))?;
        out.extend_from_slice(&tc.to_le_bytes());
        for (term_id, postings) in &self.terms {
            out.extend_from_slice(&term_id.to_le_bytes());
            let pc = u32::try_from(postings.len())
                .map_err(|_| VecLiteError::Corrupt("sparse: too many postings".to_owned()))?;
            out.extend_from_slice(&pc.to_le_bytes());
            for (slot, weight) in postings {
                out.extend_from_slice(&slot.to_le_bytes());
                out.extend_from_slice(&weight.to_bits().to_le_bytes());
            }
        }
        Ok(out)
    }

    pub(crate) fn decode(b: &[u8]) -> Result<SparsePostings> {
        let tc = le::u32(b, 0, "sparse")? as usize;
        let mut at = 4;
        let mut terms = Vec::with_capacity(tc.min(4096));
        for _ in 0..tc {
            let term_id = le::u32(b, at, "sparse")?;
            at += 4;
            let pc = le::u32(b, at, "sparse")? as usize;
            at += 4;
            let mut postings = Vec::with_capacity(pc.min(4096));
            for _ in 0..pc {
                let slot = le::u64(b, at, "sparse")?;
                at += 8;
                let weight = f32::from_bits(le::u32(b, at, "sparse")?);
                at += 4;
                postings.push((slot, weight));
            }
            terms.push((term_id, postings));
        }
        Ok(SparsePostings { terms })
    }
}

// ── VOCAB (seg_type 8) ───────────────────────────────────────────────────

/// The VOCAB body is the opaque `Embedder::export_state` blob (SPEC-005) — no
/// framing. These identity helpers exist for symmetry with the other bodies.
pub(crate) fn encode_vocab(state: &[u8]) -> Vec<u8> {
    state.to_vec()
}
pub(crate) fn decode_vocab(b: &[u8]) -> Vec<u8> {
    b.to_vec()
}

// ── HNSW (seg_type 7) ────────────────────────────────────────────────────

/// Serialized HNSW graph: a 1-byte graph-format version then the opaque
/// `hnsw_rs` dump (SPEC-002 §3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HnswBody {
    pub(crate) graph_format: u8,
    pub(crate) dump: Vec<u8>,
}

impl HnswBody {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.dump.len());
        out.push(self.graph_format);
        out.extend_from_slice(&self.dump);
        out
    }
    pub(crate) fn decode(b: &[u8]) -> Result<HnswBody> {
        let graph_format = *b
            .first()
            .ok_or_else(|| VecLiteError::Corrupt("hnsw: empty body".to_owned()))?;
        Ok(HnswBody {
            graph_format,
            dump: b[1..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trip() {
        let c = StoredConfig {
            dimension: 384,
            metric: 0,
            m: 16,
            ef_construction: 200,
            ef_search: 100,
            quantization: 1,
            quant_bits: 8,
            compression: 1,
            embedding_provider: Some("bm25".into()),
            created_epoch_s: 1_700_000_000,
        };
        assert_eq!(
            StoredConfig::decode(&c.encode().unwrap_or_else(|e| panic!("{e}")))
                .unwrap_or_else(|e| panic!("{e}")),
            c
        );
        assert!(matches!(
            StoredConfig::decode(&[0xC1]),
            Err(VecLiteError::Corrupt(_))
        ));
    }

    #[test]
    fn tombstone_round_trip() {
        let mut t = RoaringTreemap::new();
        for s in [1u64, 5, 9, 1_000_000, 5_000_000_000] {
            t.insert(s);
        }
        let back = decode_tombstone(&encode_tombstone(&t).unwrap_or_else(|e| panic!("{e}")))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, t);
    }

    #[test]
    fn payload_round_trip_and_truncation() {
        let p = PayloadBlock {
            entries: vec![
                (0, serde_json::json!({"lang": "en"})),
                (7, serde_json::json!({"n": 42, "tags": ["a", "b"]})),
            ],
        };
        let bytes = p.encode().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            PayloadBlock::decode(&bytes).unwrap_or_else(|e| panic!("{e}")),
            p
        );
        assert!(matches!(
            PayloadBlock::decode(&bytes[..bytes.len() - 3]),
            Err(VecLiteError::Corrupt(_))
        ));
    }

    #[test]
    fn pidx_round_trip() {
        let mut a = RoaringTreemap::new();
        a.insert(1);
        a.insert(9);
        let mut b = RoaringTreemap::new();
        b.insert(2);
        let idx = PayloadIndex {
            kind: 1,
            key: "lang".into(),
            postings: vec![(b"en".to_vec(), a), (b"pt".to_vec(), b)],
        };
        assert_eq!(
            PayloadIndex::decode(&idx.encode().unwrap_or_else(|e| panic!("{e}")))
                .unwrap_or_else(|e| panic!("{e}")),
            idx
        );
    }

    #[test]
    fn sparse_round_trip() {
        let s = SparsePostings {
            terms: vec![(3, vec![(0, 0.5), (10, 1.25)]), (99, vec![(7, -0.1)])],
        };
        assert_eq!(
            SparsePostings::decode(&s.encode().unwrap_or_else(|e| panic!("{e}")))
                .unwrap_or_else(|e| panic!("{e}")),
            s
        );
    }

    #[test]
    fn hnsw_and_vocab_round_trip() {
        let h = HnswBody {
            graph_format: 1,
            dump: vec![9, 8, 7, 6, 5],
        };
        assert_eq!(
            HnswBody::decode(&h.encode()).unwrap_or_else(|e| panic!("{e}")),
            h
        );
        assert!(matches!(
            HnswBody::decode(&[]),
            Err(VecLiteError::Corrupt(_))
        ));

        let state = b"opaque-embedder-state".to_vec();
        assert_eq!(decode_vocab(&encode_vocab(&state)), state);
    }
}
