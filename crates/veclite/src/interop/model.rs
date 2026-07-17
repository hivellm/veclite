//! Serde mirrors of the Vectorizer server's `.vecdb` wire shapes (SPEC-013).
//!
//! Field names, enum tags, and casing replicate the server structs exactly
//! (`crates/vectorizer/src/persistence/mod.rs`, `storage/index.rs`,
//! `models/mod.rs` in the pinned server tree) — the export writer emits JSON
//! the server's `StorageReader` deserializes strictly, and the import reader
//! parses server output leniently (`#[serde(default)]` on every field the
//! server marks optional, plus `serde_json::Value` for server-only aspects we
//! only need to detect, never interpret).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Server `PersistedVectorStore`: the JSON document inside each
/// `<collection>_vector_store.bin` archive entry (and the whole Legacy file).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbStore {
    /// Always `1` — the only value the server accepts.
    pub version: u32,
    #[serde(default)]
    pub collections: Vec<VecdbCollection>,
}

/// Server `PersistedCollection`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbCollection {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub config: Option<VecdbConfig>,
    #[serde(default)]
    pub vectors: Vec<VecdbVector>,
    /// Always `None` in current server builds (HNSW graphs are rebuilt on
    /// load, IOP-012). The server deserializes this field strictly, so the
    /// export writer must emit it.
    #[serde(default)]
    pub hnsw_dump_basename: Option<String>,
}

/// Server `PersistedVector`. Vector data is raw f32 — the server's persisted
/// form carries no quantized codes, so f32-exact translation IS the lossless
/// contract of IOP-001 (the SQ/binary encodings are config, vendored
/// byte-identical, and recomputed deterministically on either side).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbVector {
    pub id: String,
    pub data: Vec<f32>,
    /// Payload re-serialized as a JSON string (server convention).
    #[serde(default)]
    pub payload_json: Option<String>,
    /// True when `data` is already L2-normalized (cosine collections).
    #[serde(default)]
    pub normalized: bool,
}

/// Server `CollectionConfig`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbConfig {
    pub dimension: usize,
    pub metric: VecdbMetric,
    pub hnsw_config: VecdbHnsw,
    pub quantization: VecdbQuantization,
    pub compression: VecdbCompression,
    #[serde(default = "default_embedding_provider")]
    pub embedding_provider: String,
    /// Server-only text-normalization policy; carried opaquely.
    #[serde(default)]
    pub normalization: Option<serde_json::Value>,
    /// `"Memory"` / `"Mmap"`; carried opaquely (VecLite decides its own tier).
    #[serde(default)]
    pub storage_type: Option<serde_json::Value>,
    /// Server-only: presence triggers the merged-shards warning (IOP-022).
    #[serde(default)]
    pub sharding: Option<serde_json::Value>,
    /// Server-only: presence triggers the dropped-graph warning (IOP-022).
    #[serde(default)]
    pub graph: Option<serde_json::Value>,
    /// Server-only: presence refuses the import (IOP-022 — cannot decrypt).
    #[serde(default)]
    pub encryption: Option<VecdbEncryption>,
}

fn default_embedding_provider() -> String {
    "bm25".to_string()
}

/// Server `DistanceMetric` (`rename_all = "lowercase"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VecdbMetric {
    Cosine,
    Euclidean,
    DotProduct,
}

/// Server `HnswConfig`. `seed` has no server-side default, so the writer must
/// emit it (as `null`); the reader tolerates its absence.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbHnsw {
    pub m: usize,
    pub ef_construction: usize,
    pub ef_search: usize,
    #[serde(default)]
    pub seed: Option<u64>,
}

/// Server `QuantizationConfig` (`tag = "type"`, `rename_all = "lowercase"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum VecdbQuantization {
    None,
    PQ {
        n_centroids: usize,
        n_subquantizers: usize,
    },
    SQ {
        bits: usize,
    },
    Binary,
}

/// Server `CompressionConfig` + `CompressionAlgorithm`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbCompression {
    pub enabled: bool,
    pub threshold_bytes: usize,
    pub algorithm: VecdbCompressionAlgorithm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VecdbCompressionAlgorithm {
    None,
    Lz4,
}

/// Server `EncryptionConfig`. Only presence matters to import (refusal);
/// typed so the refusal can name the policy in the error.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbEncryption {
    #[serde(default)]
    pub required: bool,
    #[serde(default = "default_allow_mixed")]
    pub allow_mixed: bool,
}

fn default_allow_mixed() -> bool {
    true
}

/// Server `StorageIndex`: the pretty-printed `.vecidx` sidecar. Timestamps are
/// RFC 3339 strings (the server parses them as `chrono::DateTime<Utc>`).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbIndex {
    /// `"1.0"` — the server `STORAGE_VERSION`.
    pub version: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub collections: Vec<VecdbCollectionIndex>,
    pub total_size: u64,
    pub compressed_size: u64,
    pub compression_ratio: f64,
}

/// Server `CollectionIndex`. `metadata` is a free string map; the export
/// writer parks VecLite-only aspects there (`veclite.aliases`,
/// `veclite.payload_indexes`) so a reverse import can restore them — the
/// server ignores unknown keys.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbCollectionIndex {
    pub name: String,
    #[serde(default)]
    pub files: Vec<VecdbFileEntry>,
    pub vector_count: usize,
    pub dimension: usize,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// Server `FileEntry`. `type` values are the lowercase server `FileType`
/// variants (`vectors`, `metadata`, `config`, `index`, `tokenizer`, `other`);
/// kept as a plain string so unknown future kinds don't fail an import.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbFileEntry {
    pub path: String,
    pub size: u64,
    pub compressed_size: u64,
    /// SHA-256 of the uncompressed content, 64-char lowercase hex.
    pub checksum: String,
    #[serde(rename = "type")]
    pub file_type: String,
}

/// Server `CollectionMetadataForStorage`: the `<collection>_metadata.json`
/// archive entry.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct VecdbMetadata {
    #[serde(default)]
    pub name: String,
    pub config: VecdbConfig,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub modified_at: Option<String>,
    #[serde(default)]
    pub vector_count: usize,
}

/// Archive entry names inside `vectorizer.vecdb` (flat layout, one set per
/// collection).
pub(crate) fn vector_store_entry(collection: &str) -> String {
    format!("{collection}_vector_store.bin")
}

pub(crate) fn metadata_entry(collection: &str) -> String {
    format!("{collection}_metadata.json")
}

pub(crate) fn tokenizer_entry(collection: &str) -> String {
    format!("{collection}_tokenizer.json")
}

/// The server data-directory file names.
pub(crate) const VECDB_FILE: &str = "vectorizer.vecdb";
pub(crate) const VECIDX_FILE: &str = "vectorizer.vecidx";
/// The server `STORAGE_VERSION` the `.vecidx` carries.
pub(crate) const VECIDX_VERSION: &str = "1.0";
/// The only `PersistedVectorStore.version` the server accepts.
pub(crate) const STORE_VERSION: u32 = 1;

/// Format `epoch_s` as an RFC 3339 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) —
/// what the server's `chrono::DateTime<Utc>` fields parse. Civil-from-days
/// per Howard Hinnant's algorithm; no calendar dependency.
pub(crate) fn rfc3339_utc(epoch_s: u64) -> String {
    let days = (epoch_s / 86_400) as i64;
    let secs_of_day = epoch_s % 86_400;
    // days_from_civil inverse, shifted era.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day / 60) % 60,
        secs_of_day % 60
    )
}

/// SHA-256 of `data` as 64-char lowercase hex (the `.vecidx` checksum form).
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(data);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_serializes_with_server_field_names_and_tags() {
        let config = VecdbConfig {
            dimension: 384,
            metric: VecdbMetric::Cosine,
            hnsw_config: VecdbHnsw {
                m: 16,
                ef_construction: 200,
                ef_search: 100,
                seed: None,
            },
            quantization: VecdbQuantization::SQ { bits: 8 },
            compression: VecdbCompression {
                enabled: true,
                threshold_bytes: 1024,
                algorithm: VecdbCompressionAlgorithm::Lz4,
            },
            embedding_provider: "bm25".to_string(),
            normalization: None,
            storage_type: Some(serde_json::json!("Memory")),
            sharding: None,
            graph: None,
            encryption: None,
        };
        let v = serde_json::to_value(&config).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v["metric"], "cosine");
        assert_eq!(v["quantization"]["type"], "sq");
        assert_eq!(v["quantization"]["bits"], 8);
        assert_eq!(v["compression"]["algorithm"], "lz4");
        assert_eq!(v["hnsw_config"]["seed"], serde_json::Value::Null);
        assert_eq!(v["storage_type"], "Memory");
        assert_eq!(v["embedding_provider"], "bm25");
    }

    #[test]
    fn parses_server_config_json_strictly_shaped() {
        // Shape as the server's serde emits it, including server-only aspects.
        let json = r#"{
            "dimension": 768,
            "metric": "dotproduct",
            "hnsw_config": {"m": 32, "ef_construction": 100, "ef_search": 50, "seed": 42},
            "quantization": {"type": "none"},
            "compression": {"enabled": false, "threshold_bytes": 1024, "algorithm": "none"},
            "embedding_provider": "candle/all-minilm",
            "normalization": {"enabled": true},
            "storage_type": "Mmap",
            "sharding": {"shard_count": 4, "virtual_nodes_per_shard": 100, "rebalance_threshold": 0.2},
            "graph": {"enabled": true, "auto_relationship": {}},
            "encryption": {"required": true, "allow_mixed": false}
        }"#;
        let config: VecdbConfig = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(config.metric, VecdbMetric::DotProduct);
        assert_eq!(config.hnsw_config.seed, Some(42));
        assert_eq!(config.quantization, VecdbQuantization::None);
        assert!(config.sharding.is_some());
        assert!(config.graph.is_some());
        assert!(config.encryption.as_ref().is_some_and(|e| e.required));
    }

    #[test]
    fn parses_minimal_legacy_collection_leniently() {
        // Old files may miss name/config/vectors (server marks them default).
        let json = r#"{"version": 1, "collections": [{"hnsw_dump_basename": null}]}"#;
        let store: VecdbStore = serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.version, 1);
        assert_eq!(store.collections.len(), 1);
        assert!(store.collections[0].name.is_empty());
        assert!(store.collections[0].config.is_none());
    }

    #[test]
    fn store_round_trips_all_required_server_fields() {
        let store = VecdbStore {
            version: STORE_VERSION,
            collections: vec![VecdbCollection {
                name: "docs".to_string(),
                config: None,
                vectors: vec![VecdbVector {
                    id: "a".to_string(),
                    data: vec![0.6, 0.8],
                    payload_json: Some(r#"{"lang":"en"}"#.to_string()),
                    normalized: true,
                }],
                hnsw_dump_basename: None,
            }],
        };
        let v = serde_json::to_value(&store).unwrap_or_else(|e| panic!("{e}"));
        // The server deserializes these strictly — they must be present.
        assert!(v["collections"][0].get("hnsw_dump_basename").is_some());
        let vec0 = &v["collections"][0]["vectors"][0];
        assert!(vec0.get("payload_json").is_some());
        assert!(vec0.get("normalized").is_some());
        assert!(vec0.get("id").is_some());
        assert!(vec0.get("data").is_some());
    }

    #[test]
    fn rfc3339_matches_known_instants() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(951_782_400), "2000-02-29T00:00:00Z");
        // 2026-07-17T12:34:56Z
        assert_eq!(rfc3339_utc(1_784_291_696), "2026-07-17T12:34:56Z");
        // Leap-year boundary: 2024-12-31T23:59:59Z.
        assert_eq!(rfc3339_utc(1_735_689_599), "2024-12-31T23:59:59Z");
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        // SHA-256("") — the canonical empty-input digest.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(sha256_hex(b"abc").len(), 64);
    }
}
