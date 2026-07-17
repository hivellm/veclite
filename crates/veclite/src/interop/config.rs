//! Config projection between VecLite [`CollectionOptions`] and the server's
//! `CollectionConfig` mirror (SPEC-013 §1: the semantics map 1:1; the
//! serialized shapes differ).
//!
//! Import degradation policy (IOP-022/023): every server-only aspect this
//! projection cannot carry is surfaced as a warning — never silently, never
//! fatally — except payload encryption, which refuses the import (VecLite
//! cannot decrypt).

use serde_json::Value;

use super::model::{
    VecdbCompression, VecdbCompressionAlgorithm, VecdbConfig, VecdbHnsw, VecdbMetric,
    VecdbQuantization,
};
use crate::error::{Result, VecLiteError};
use crate::options::{CollectionOptions, Compression, HnswOptions, Metric, Quantization};

/// Project a VecLite collection config to the server shape (IOP-011). BYO
/// collections carry the server's default provider id (`bm25`) — the server
/// has no "no provider" concept; the `.vecidx` metadata records the truth so
/// a reverse import restores BYO exactly.
pub(crate) fn to_vecdb_config(
    name: &str,
    options: &CollectionOptions,
    warnings: &mut Vec<String>,
) -> VecdbConfig {
    let quantization = match options.quantization {
        Quantization::None => VecdbQuantization::None,
        Quantization::Scalar { bits } => VecdbQuantization::SQ {
            bits: usize::from(bits),
        },
        Quantization::Binary => VecdbQuantization::Binary,
    };
    let compression = match options.compression {
        Compression::None => VecdbCompression {
            enabled: false,
            threshold_bytes: 1024,
            algorithm: VecdbCompressionAlgorithm::None,
        },
        Compression::Lz4 { threshold } => VecdbCompression {
            enabled: true,
            threshold_bytes: threshold as usize,
            algorithm: VecdbCompressionAlgorithm::Lz4,
        },
        Compression::Zstd { threshold } => {
            warnings.push(format!(
                "collection {name:?}: zstd payload compression has no server equivalent; \
                 exported as lz4 (payload bytes are unaffected)"
            ));
            VecdbCompression {
                enabled: true,
                threshold_bytes: threshold as usize,
                algorithm: VecdbCompressionAlgorithm::Lz4,
            }
        }
    };
    VecdbConfig {
        dimension: options.dimension,
        metric: match options.metric {
            Metric::Cosine => VecdbMetric::Cosine,
            Metric::Euclidean => VecdbMetric::Euclidean,
            Metric::DotProduct => VecdbMetric::DotProduct,
        },
        hnsw_config: VecdbHnsw {
            m: options.hnsw.m,
            ef_construction: options.hnsw.ef_construction,
            ef_search: options.hnsw.ef_search,
            seed: None,
        },
        quantization,
        compression,
        embedding_provider: options
            .embedding_provider
            .clone()
            .unwrap_or_else(|| "bm25".to_string()),
        normalization: None,
        storage_type: Some(Value::String("Memory".to_string())),
        sharding: None,
        graph: None,
        encryption: None,
    }
}

/// The provider outcome an import resolved (IOP-022 last row).
pub(crate) enum ProviderPlan {
    /// BYO-vector collection: no auto-embed provider.
    Byo,
    /// Auto-embed with a provider this build can construct or defer.
    AutoEmbed(String),
}

/// Rebuild VecLite [`CollectionOptions`] from a server config, degrading
/// server-only aspects per the IOP-022 matrix. `Err` only for payload
/// encryption (refused — cannot decrypt) so the caller aborts the import
/// with a clear cause.
pub(crate) fn from_vecdb_config(
    name: &str,
    config: &VecdbConfig,
    provider_plan: &ProviderPlan,
    warnings: &mut Vec<String>,
) -> Result<CollectionOptions> {
    if let Some(encryption) = &config.encryption {
        if encryption.required {
            return Err(VecLiteError::InvalidArgument(format!(
                "collection {name:?} enforces payload encryption; VecLite cannot decrypt \
                 server-encrypted payloads — export it unencrypted server-side first (IOP-022)"
            )));
        }
        warnings.push(format!(
            "collection {name:?}: server encryption policy (optional, mixed allowed) dropped; \
             any individually encrypted payloads remain undecryptable"
        ));
    }
    if let Some(sharding) = &config.sharding {
        let shards = sharding
            .get("shard_count")
            .and_then(Value::as_u64)
            .map_or(String::new(), |n| format!(" ({n} shards)"));
        warnings.push(format!(
            "collection {name:?}: sharded server collection{shards} merged into one (IOP-022)"
        ));
    }
    if let Some(graph) = &config.graph {
        let enabled = graph
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if enabled {
            warnings.push(format!(
                "collection {name:?}: server graph relationships dropped — VecLite has no graph \
                 support yet (IOP-022)"
            ));
        }
    }
    if config.normalization.is_some() {
        warnings.push(format!(
            "collection {name:?}: server text-normalization policy dropped (stored payloads are \
             kept verbatim)"
        ));
    }
    if config.hnsw_config.seed.is_some() {
        warnings.push(format!(
            "collection {name:?}: HNSW level seed dropped — VecLite does not pin graph seeds"
        ));
    }

    let quantization = match config.quantization {
        VecdbQuantization::None => Quantization::None,
        VecdbQuantization::SQ { bits } if matches!(bits, 1 | 2 | 4 | 8) => Quantization::Scalar {
            #[allow(clippy::cast_possible_truncation)] // matched 1|2|4|8 above
            bits: bits as u8,
        },
        VecdbQuantization::SQ { bits } => {
            warnings.push(format!(
                "collection {name:?}: SQ-{bits} quantization is not supported by VecLite \
                 (1/2/4/8-bit); imported without quantization — vectors are exact f32 either way"
            ));
            Quantization::None
        }
        VecdbQuantization::Binary => Quantization::Binary,
        VecdbQuantization::PQ { .. } => {
            warnings.push(format!(
                "collection {name:?}: product quantization is not supported by VecLite; imported \
                 without quantization — vectors are exact f32 either way"
            ));
            Quantization::None
        }
    };
    let compression = if config.compression.enabled {
        match config.compression.algorithm {
            #[allow(clippy::cast_possible_truncation)] // thresholds are small (bytes)
            VecdbCompressionAlgorithm::Lz4 => Compression::Lz4 {
                threshold: config.compression.threshold_bytes.min(u32::MAX as usize) as u32,
            },
            VecdbCompressionAlgorithm::None => Compression::None,
        }
    } else {
        Compression::None
    };

    Ok(CollectionOptions {
        dimension: config.dimension,
        metric: match config.metric {
            VecdbMetric::Cosine => Metric::Cosine,
            VecdbMetric::Euclidean => Metric::Euclidean,
            VecdbMetric::DotProduct => Metric::DotProduct,
        },
        hnsw: HnswOptions {
            m: config.hnsw_config.m,
            ef_construction: config.hnsw_config.ef_construction,
            ef_search: config.hnsw_config.ef_search,
        },
        quantization,
        compression,
        embedding_provider: match provider_plan {
            ProviderPlan::Byo => None,
            ProviderPlan::AutoEmbed(provider) => Some(provider.clone()),
        },
        payload_indexes: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interop::model::VecdbEncryption;

    fn server_config() -> VecdbConfig {
        VecdbConfig {
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
            storage_type: None,
            sharding: None,
            graph: None,
            encryption: None,
        }
    }

    #[test]
    fn clean_config_round_trips_without_warnings() {
        let mut warnings = Vec::new();
        let options = from_vecdb_config(
            "docs",
            &server_config(),
            &ProviderPlan::AutoEmbed("bm25".to_string()),
            &mut warnings,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(options.dimension, 384);
        assert_eq!(options.metric, Metric::Cosine);
        assert_eq!(options.quantization, Quantization::Scalar { bits: 8 });
        assert_eq!(options.embedding_provider.as_deref(), Some("bm25"));

        let back = to_vecdb_config("docs", &options, &mut warnings);
        assert!(warnings.is_empty());
        assert_eq!(back.dimension, 384);
        assert_eq!(back.metric, VecdbMetric::Cosine);
        assert_eq!(back.quantization, VecdbQuantization::SQ { bits: 8 });
    }

    #[test]
    fn required_encryption_refuses_with_clear_error() {
        let mut config = server_config();
        config.encryption = Some(VecdbEncryption {
            required: true,
            allow_mixed: false,
        });
        let mut warnings = Vec::new();
        let Err(err) = from_vecdb_config("secure", &config, &ProviderPlan::Byo, &mut warnings)
        else {
            panic!("required encryption must refuse the import");
        };
        let msg = err.to_string();
        assert!(
            msg.contains("encryption"),
            "error must name encryption: {msg}"
        );
        assert!(
            msg.contains("secure"),
            "error must name the collection: {msg}"
        );
    }

    #[test]
    fn server_only_aspects_warn_but_import() {
        let mut config = server_config();
        config.sharding = Some(serde_json::json!({"shard_count": 4}));
        config.graph = Some(serde_json::json!({"enabled": true}));
        config.normalization = Some(serde_json::json!({"enabled": true}));
        config.hnsw_config.seed = Some(7);
        config.quantization = VecdbQuantization::PQ {
            n_centroids: 256,
            n_subquantizers: 8,
        };
        let mut warnings = Vec::new();
        let options = from_vecdb_config("multi", &config, &ProviderPlan::Byo, &mut warnings)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(options.quantization, Quantization::None);
        assert!(options.embedding_provider.is_none());
        let joined = warnings.join("\n");
        for expectation in ["shard", "graph", "normalization", "seed", "quantization"] {
            assert!(
                joined.contains(expectation),
                "missing {expectation:?} warning in: {joined}"
            );
        }
    }

    #[test]
    fn disabled_graph_config_is_inert_no_warning() {
        let mut config = server_config();
        config.graph = Some(serde_json::json!({"enabled": false}));
        let mut warnings = Vec::new();
        from_vecdb_config("quiet", &config, &ProviderPlan::Byo, &mut warnings)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            warnings.is_empty(),
            "inert graph must not warn: {warnings:?}"
        );
    }
}
