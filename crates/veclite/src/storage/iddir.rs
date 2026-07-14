//! IDDIR segment body (SPEC-002 §3.3, STG-032): a hash-bucketed `id → slot`
//! directory. `xxhash64(id)` picks a bucket; collisions are resolved within the
//! bucket by full id comparison. Tombstoned slots stay until vacuum rewrites
//! the directory.

use crate::error::{Result, VecLiteError};
use crate::storage::le;

/// `id → slot` directory. Buckets hold `(id, slot)` pairs; lookup hashes the id
/// to a bucket then scans it (STG-032).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IdDir {
    buckets: Vec<Vec<(String, u64)>>,
}

impl IdDir {
    /// Empty directory with `bucket_count` buckets (at least 1).
    pub(crate) fn new(bucket_count: usize) -> Self {
        IdDir {
            buckets: vec![Vec::new(); bucket_count.max(1)],
        }
    }

    fn bucket_of(&self, id: &str) -> usize {
        // buckets is never empty (new clamps to >=1, decode rejects 0).
        let h = xxhash_rust::xxh64::xxh64(id.as_bytes(), 0);
        usize::try_from(h % self.buckets.len() as u64).unwrap_or(0)
    }

    /// Add a mapping. The caller ensures ids are unique per live directory;
    /// duplicates simply coexist in the bucket (last-inserted found first is
    /// not guaranteed — the live layer never inserts a live duplicate).
    pub(crate) fn insert(&mut self, id: String, slot: u64) {
        let b = self.bucket_of(&id);
        self.buckets[b].push((id, slot));
    }

    /// Resolve an id to its slot, or `None` if absent.
    pub(crate) fn get(&self, id: &str) -> Option<u64> {
        let b = self.bucket_of(id);
        self.buckets[b]
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, slot)| *slot)
    }

    /// Total live entries.
    pub(crate) fn len(&self) -> usize {
        self.buckets.iter().map(Vec::len).sum()
    }

    /// Iterate all `(id, slot)` mappings (unordered) — used to rebuild the
    /// slot→id direction when loading a collection.
    pub(crate) fn entries(&self) -> impl Iterator<Item = (&str, u64)> {
        self.buckets
            .iter()
            .flatten()
            .map(|(id, slot)| (id.as_str(), *slot))
    }

    /// Layout: `bucket_count u32`, then per bucket `entry_count u32` and each
    /// entry `id_len u16 · id bytes · slot u64`.
    #[allow(clippy::cast_possible_truncation)] // ids <=512 bytes (CORE-010); counts fit by construction.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.buckets.len() as u32).to_le_bytes());
        for bucket in &self.buckets {
            out.extend_from_slice(&(bucket.len() as u32).to_le_bytes());
            for (id, slot) in bucket {
                out.extend_from_slice(&(id.len() as u16).to_le_bytes());
                out.extend_from_slice(id.as_bytes());
                out.extend_from_slice(&slot.to_le_bytes());
            }
        }
        out
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<IdDir> {
        let bucket_count = le::u32(bytes, 0, "iddir")? as usize;
        if bucket_count == 0 {
            return Err(VecLiteError::Corrupt("iddir: zero buckets".to_owned()));
        }
        let mut at = 4;
        let mut buckets = Vec::with_capacity(bucket_count);
        for _ in 0..bucket_count {
            let entry_count = le::u32(bytes, at, "iddir")? as usize;
            at += 4;
            let mut bucket = Vec::with_capacity(entry_count.min(1024));
            for _ in 0..entry_count {
                let id_len = le::u16(bytes, at, "iddir")? as usize;
                at += 2;
                let end = at
                    .checked_add(id_len)
                    .filter(|&e| e <= bytes.len())
                    .ok_or_else(|| VecLiteError::Corrupt("iddir: id past end".to_owned()))?;
                let id = String::from_utf8(bytes[at..end].to_vec())
                    .map_err(|_| VecLiteError::Corrupt("iddir: id not utf-8".to_owned()))?;
                at = end;
                let slot = le::u64(bytes, at, "iddir")?;
                at += 8;
                bucket.push((id, slot));
            }
            buckets.push(bucket);
        }
        Ok(IdDir { buckets })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_lookup() {
        let mut d = IdDir::new(8);
        for i in 0..50u64 {
            d.insert(format!("id-{i}"), i * 10);
        }
        let bytes = d.encode();
        let back = IdDir::decode(&bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, d);
        assert_eq!(back.len(), 50);
        assert_eq!(back.get("id-7"), Some(70));
        assert_eq!(back.get("id-49"), Some(490));
        assert_eq!(back.get("missing"), None);
    }

    #[test]
    fn collisions_resolved_by_full_id() {
        // One bucket forces every id into the same bucket; get must still
        // distinguish them by full comparison (STG-032).
        let mut d = IdDir::new(1);
        d.insert("alpha".into(), 1);
        d.insert("beta".into(), 2);
        d.insert("gamma".into(), 3);
        assert_eq!(d.get("beta"), Some(2));
        assert_eq!(d.get("gamma"), Some(3));
        assert_eq!(d.get("delta"), None);
        let back = IdDir::decode(&d.encode()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.get("alpha"), Some(1));
    }

    #[test]
    fn truncated_and_zero_buckets_are_corrupt() {
        assert!(matches!(
            IdDir::decode(&[1, 0, 0]),
            Err(VecLiteError::Corrupt(_))
        ));
        assert!(matches!(
            IdDir::decode(&0u32.to_le_bytes()),
            Err(VecLiteError::Corrupt(_))
        ));
    }
}
