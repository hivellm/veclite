//! Conversion between the runtime [`CollectionOptions`] and its on-disk
//! projection [`StoredConfig`] (SPEC-002 CONFIG segment / SPEC-003 CREATE_COLL
//! body). Lives in `persist` so it can see both the options types and the
//! storage codec without coupling those layers to each other.

use crate::error::{Result, VecLiteError};
use crate::options::CollectionOptions;
use crate::options::{Compression, HnswOptions, Metric, PayloadIndexKind, Quantization};
use crate::storage::body::StoredConfig;

fn metric_byte(m: Metric) -> u8 {
    match m {
        Metric::Cosine => 0,
        Metric::Euclidean => 1,
        Metric::DotProduct => 2,
    }
}

fn metric_from(b: u8) -> Result<Metric> {
    Ok(match b {
        0 => Metric::Cosine,
        1 => Metric::Euclidean,
        2 => Metric::DotProduct,
        other => return Err(corrupt(format!("unknown metric {other}"))),
    })
}

/// `(quantization byte, bits)`: None=0, Scalar{bits}=1, Binary=2.
fn quant_bytes(q: Quantization) -> (u8, u8) {
    match q {
        Quantization::None => (0, 0),
        Quantization::Scalar { bits } => (1, bits),
        Quantization::Binary => (2, 0),
    }
}

fn quant_from(byte: u8, bits: u8) -> Result<Quantization> {
    Ok(match byte {
        0 => Quantization::None,
        1 => Quantization::Scalar { bits },
        2 => Quantization::Binary,
        other => return Err(corrupt(format!("unknown quantization {other}"))),
    })
}

fn compression_byte(c: Compression) -> u8 {
    match c {
        Compression::None => 0,
        Compression::Lz4 { .. } => 1,
        Compression::Zstd { .. } => 2,
    }
}

/// The threshold is not persisted in the compact CONFIG byte; the default
/// (1024) is restored on load — it only affects future writes, not correctness.
fn compression_from(b: u8) -> Result<Compression> {
    Ok(match b {
        0 => Compression::None,
        1 => Compression::Lz4 { threshold: 1024 },
        2 => Compression::Zstd { threshold: 1024 },
        other => return Err(corrupt(format!("unknown compression {other}"))),
    })
}

fn corrupt(what: String) -> VecLiteError {
    VecLiteError::Corrupt(format!("config: {what}"))
}

/// Project a runtime config to its on-disk form.
pub(crate) fn to_stored(options: &CollectionOptions, created_epoch_s: u64) -> StoredConfig {
    let (quantization, quant_bits) = quant_bytes(options.quantization);
    #[allow(clippy::cast_possible_truncation)] // dimension <= 65_536, hnsw params bounded.
    StoredConfig {
        dimension: options.dimension as u32,
        metric: metric_byte(options.metric),
        m: options.hnsw.m as u32,
        ef_construction: options.hnsw.ef_construction as u32,
        ef_search: options.hnsw.ef_search as u32,
        quantization,
        quant_bits,
        compression: compression_byte(options.compression),
        embedding_provider: options.embedding_provider.clone(),
        created_epoch_s,
    }
}

/// Reconstruct a runtime config from its on-disk form. Declared payload indexes
/// are not part of the CONFIG projection in v1 (they are replayed from PIDX
/// segments / WAL); an empty list is restored.
pub(crate) fn from_stored(stored: &StoredConfig) -> Result<CollectionOptions> {
    Ok(CollectionOptions {
        dimension: stored.dimension as usize,
        metric: metric_from(stored.metric)?,
        hnsw: HnswOptions {
            m: stored.m as usize,
            ef_construction: stored.ef_construction as usize,
            ef_search: stored.ef_search as usize,
        },
        quantization: quant_from(stored.quantization, stored.quant_bits)?,
        compression: compression_from(stored.compression)?,
        embedding_provider: stored.embedding_provider.clone(),
        payload_indexes: Vec::<(String, PayloadIndexKind)>::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_round_trip_through_stored() {
        for (metric, quant) in [
            (Metric::Cosine, Quantization::Scalar { bits: 8 }),
            (Metric::Euclidean, Quantization::None),
            (Metric::DotProduct, Quantization::Binary),
        ] {
            let opts = CollectionOptions::new(384, metric)
                .hnsw(24, 300, 150)
                .quantization(quant);
            let stored = to_stored(&opts, 1000);
            let back = from_stored(&stored).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(back.dimension, 384);
            assert_eq!(back.metric, metric);
            assert_eq!(back.quantization, quant);
            assert_eq!(back.hnsw.m, 24);
            assert_eq!(back.hnsw.ef_construction, 300);
            assert_eq!(back.hnsw.ef_search, 150);
        }
    }
}
