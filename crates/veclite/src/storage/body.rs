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

/// Maximum MessagePack container nesting accepted from untrusted bytes.
/// rmp-serde recurses one stack frame per nesting level with no built-in
/// limit, so a deeply nested array/map (a fuzzed or corrupt file) overflows
/// the stack before it can be rejected — found by the SPEC-015 `image` and
/// `wal` fuzz targets, which decode attacker-controlled payloads into
/// `serde_json::Value`. The bound must be safe on the smallest reasonable
/// stack (Windows' 1 MiB main thread, debug builds with large frames), not
/// just an 8 MiB Linux stack. rmp-serde's recursive `Value` decode carries a
/// heavy per-level frame in debug (~tens of KiB), so the deepest accepted
/// decode must stay comfortably under 1 MiB: 16 levels clears every real
/// payload (documents nest a handful deep) with wide margin, and anything
/// deeper is rejected here before rmp-serde recurses into it.
pub(crate) const MAX_MSGPACK_DEPTH: usize = 16;

/// Reject MessagePack whose container nesting exceeds [`MAX_MSGPACK_DEPTH`]
/// before it reaches rmp-serde's unbounded recursive decoder (STG-021). A
/// single non-recursive pass over the first value: each open container pushes
/// its child count onto an explicit stack; the live stack height is the depth.
/// Structurally short/truncated input is left for rmp-serde to report — this
/// guard only bounds depth, so a well-formed shallow value always passes.
pub(crate) fn guard_msgpack_depth(bytes: &[u8]) -> Result<()> {
    fn be_len(b: &[u8], at: usize, n: usize) -> Result<usize> {
        let raw = slice(b, at, n, "msgpack length")?;
        Ok(raw
            .iter()
            .fold(0usize, |acc, &byte| (acc << 8) | byte as usize))
    }
    let too_deep =
        || VecLiteError::Corrupt(format!("payload: nesting exceeds {MAX_MSGPACK_DEPTH}"));

    // Each stack entry is the number of child elements still expected in an
    // open container; the top-level value is modeled as one expected child.
    let mut expected: Vec<u64> = vec![1];
    let mut at = 0usize;
    while let Some(&remaining) = expected.last() {
        if remaining == 0 {
            expected.pop();
            continue;
        }
        if let Some(top) = expected.last_mut() {
            *top -= 1;
        }
        if expected.len() > MAX_MSGPACK_DEPTH {
            return Err(too_deep());
        }
        let Some(&marker) = bytes.get(at) else {
            // Truncated: not this guard's job — hand it to rmp-serde, which
            // reports the precise structural error.
            return Ok(());
        };
        at += 1;
        let mut skip = 0usize;
        let mut children = 0u64;
        match marker {
            // fixint (pos/neg), nil, false, true — atoms, no body.
            0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => {}
            0xc1 => {
                return Err(VecLiteError::Corrupt(
                    "payload: reserved msgpack marker".into(),
                ));
            }
            0x80..=0x8f => children = 2 * u64::from(marker & 0x0f), // fixmap
            0x90..=0x9f => children = u64::from(marker & 0x0f),     // fixarray
            0xa0..=0xbf => skip = usize::from(marker & 0x1f),       // fixstr
            0xcc | 0xd0 => skip = 1,                                // u8 / i8
            0xcd | 0xd1 => skip = 2,                                // u16 / i16
            0xca | 0xce | 0xd2 => skip = 4,                         // f32 / u32 / i32
            0xcb | 0xcf | 0xd3 => skip = 8,                         // f64 / u64 / i64
            0xd4 => skip = 2,                                       // fixext1 (type+1)
            0xd5 => skip = 3,                                       // fixext2
            0xd6 => skip = 5,                                       // fixext4
            0xd7 => skip = 9,                                       // fixext8
            0xd8 => skip = 17,                                      // fixext16
            0xc4 | 0xd9 => {
                skip = be_len(bytes, at, 1)?; // bin8 / str8
                at += 1;
            }
            0xc5 | 0xda => {
                skip = be_len(bytes, at, 2)?; // bin16 / str16
                at += 2;
            }
            0xc6 | 0xdb => {
                skip = be_len(bytes, at, 4)?; // bin32 / str32
                at += 4;
            }
            0xc7 => {
                skip = be_len(bytes, at, 1)? + 1; // ext8 (len + type)
                at += 1;
            }
            0xc8 => {
                skip = be_len(bytes, at, 2)? + 1; // ext16
                at += 2;
            }
            0xc9 => {
                skip = be_len(bytes, at, 4)? + 1; // ext32
                at += 4;
            }
            0xdc => {
                children = be_len(bytes, at, 2)? as u64; // array16
                at += 2;
            }
            0xdd => {
                children = be_len(bytes, at, 4)? as u64; // array32
                at += 4;
            }
            0xde => {
                children = 2 * be_len(bytes, at, 2)? as u64; // map16
                at += 2;
            }
            0xdf => {
                children = 2 * be_len(bytes, at, 4)? as u64; // map32
                at += 4;
            }
        }
        match at.checked_add(skip) {
            Some(next) if next <= bytes.len() => at = next,
            // Body runs past the slice — truncated; leave it for rmp-serde.
            _ => return Ok(()),
        }
        if children > 0 {
            expected.push(children);
        }
    }
    Ok(())
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
        // Even a fixed-shape struct recurses in rmp-serde when the input nests
        // containers where scalar fields are expected (serde walks and skips
        // them). Bound the nesting before that recursion (SPEC-015 `config`
        // fuzz target).
        guard_msgpack_depth(b)?;
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
            guard_msgpack_depth(bytes)?;
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
    fn deep_msgpack_payload_is_rejected_not_overflowed() {
        // Nest fixarrays (0x91 = 1-element array) far past the limit, then a
        // nil leaf. rmp-serde would recurse per level and overflow the stack;
        // the guard rejects it as Corrupt in a single non-recursive pass.
        let depth = MAX_MSGPACK_DEPTH + 5000;
        let mut deep = vec![0x91u8; depth];
        deep.push(0xc0); // nil leaf
        assert!(matches!(
            guard_msgpack_depth(&deep),
            Err(VecLiteError::Corrupt(ref m)) if m.contains("nesting")
        ));

        // A real payload nested well within the limit passes the guard and
        // decodes identically to a direct rmp round trip.
        let value = serde_json::json!({"a": {"b": {"c": [1, 2, {"d": true}]}}});
        let mp = rmp_serde::to_vec(&value).unwrap_or_else(|e| panic!("{e}"));
        guard_msgpack_depth(&mp).unwrap_or_else(|e| panic!("legit payload rejected: {e}"));
        let back: Value = rmp_serde::from_slice(&mp).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, value);

        // The exact fuzz reproducer that first overflowed the stack: whatever
        // it is structurally, decode must return a typed error, never panic.
        let deep_block = {
            let mut body = Vec::new();
            body.extend_from_slice(&0u64.to_le_bytes()); // slot
            body.extend_from_slice(&(deep.len() as u32).to_le_bytes());
            body.extend_from_slice(&deep);
            body
        };
        assert!(matches!(
            PayloadBlock::decode(&deep_block),
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
