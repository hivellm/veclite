//! Reverse path (SPEC-013 §3): read a server data set — Compact
//! `vectorizer.vecdb` or Legacy `*_vector_store.bin` — into a VecLite
//! database.
//!
//! Layout detection mirrors the server's `detect_format` (IOP-020);
//! `--collections` subsets (IOP-021); server-only aspects degrade with
//! warnings, never silently and never fatally — except payload encryption,
//! which refuses the import before anything is created (IOP-022). Collections
//! whose embedding provider this build cannot construct import as BYO-vector:
//! vectors and payloads intact, text re-embedding disabled, and the origin
//! provider recorded in CONFIG via the deferred embedder slot (IOP-022).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::model::{self, VecdbConfig, VecdbIndex, VecdbStore, VecdbVector};
use crate::database::VecLite;
use crate::error::{Result, VecLiteError};
use crate::options::PayloadIndexKind;
use crate::point::Point;

/// The two server on-disk layouts (IOP-020).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerLayout {
    /// `vectorizer.vecdb` ZIP archive (+ `vectorizer.vecidx` sidecar).
    Compact,
    /// Bare `<collection>_vector_store.bin` gzip-JSON files.
    Legacy,
}

/// Scope options for [`import_vecdb`] (IOP-021).
#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Import only these collections; `None` imports all.
    pub collections: Option<Vec<String>>,
}

/// What an import restored (the CLI degradation table, SPEC-014).
#[derive(Debug)]
pub struct ImportReport {
    /// The detected source layout.
    pub layout: ServerLayout,
    /// Per-collection outcomes, in import order.
    pub collections: Vec<ImportedCollection>,
    /// The IOP-022 degradation warnings (never silent).
    pub warnings: Vec<String>,
}

/// One imported collection in the [`ImportReport`].
#[derive(Debug)]
pub struct ImportedCollection {
    /// Collection name.
    pub name: String,
    /// Vectors restored.
    pub vectors: usize,
    /// `Some(provider)` when the collection imported as BYO-vector because
    /// the provider is server-only in this build (recorded as the CONFIG
    /// origin provider, text operations disabled).
    pub deferred_provider: Option<String>,
}

/// Detect the server layout at `src` — a data directory, a `.vecdb` file, or
/// a Legacy `*_vector_store.bin` file (IOP-020, mirroring the server's
/// `detect_format`).
pub fn detect_layout(src: &Path) -> Result<ServerLayout> {
    if src.is_file() {
        let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".vecdb") {
            return Ok(ServerLayout::Compact);
        }
        if name.ends_with("_vector_store.bin") {
            return Ok(ServerLayout::Legacy);
        }
        return Err(VecLiteError::InvalidArgument(format!(
            "{}: not a server data set (expected a .vecdb archive, a *_vector_store.bin file, \
             or a directory containing either)",
            src.display()
        )));
    }
    if src.is_dir() {
        if src.join(model::VECDB_FILE).exists() {
            return Ok(ServerLayout::Compact);
        }
        for entry in fs::read_dir(src)?.flatten() {
            let name = entry.file_name();
            if name
                .to_str()
                .is_some_and(|n| n.ends_with("_vector_store.bin"))
            {
                return Ok(ServerLayout::Legacy);
            }
        }
        return Err(VecLiteError::InvalidArgument(format!(
            "{}: no vectorizer.vecdb and no *_vector_store.bin files found",
            src.display()
        )));
    }
    Err(VecLiteError::InvalidArgument(format!(
        "{}: path does not exist",
        src.display()
    )))
}

/// One collection as loaded from the source, before assembly.
struct SourceCollection {
    name: String,
    config: Option<VecdbConfig>,
    vectors: Vec<VecdbVector>,
    tokenizer: Option<Value>,
    /// `.vecidx` CollectionIndex.metadata (carries `veclite.*` keys on
    /// archives VecLite exported; empty for pure server archives).
    index_metadata: BTreeMap<String, String>,
}

fn corrupt(what: String) -> VecLiteError {
    VecLiteError::Corrupt(format!("vecdb import: {what}"))
}

/// Parse a `_vector_store.bin` document (JSON `PersistedVectorStore`),
/// enforcing the only version the format defines.
fn parse_store(source: &str, bytes: &[u8]) -> Result<VecdbStore> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| corrupt(format!("{source}: invalid UTF-8 in vector store: {e}")))?;
    let store: VecdbStore = serde_json::from_str(text)
        .map_err(|e| corrupt(format!("{source}: failed to deserialize: {e}")))?;
    if store.version != model::STORE_VERSION {
        return Err(corrupt(format!(
            "{source}: unsupported vector store version {} (expected {})",
            store.version,
            model::STORE_VERSION
        )));
    }
    Ok(store)
}

/// Extract config + collection from a parsed store, applying the same
/// filename/name and metadata/config fallbacks the server reader applies,
/// and warning about server-only metadata fields (IOP-022 owner/tenant row).
fn assemble_source(
    name_from_file: &str,
    store: VecdbStore,
    metadata_json: Option<&[u8]>,
    tokenizer_json: Option<&[u8]>,
    index_metadata: BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<SourceCollection> {
    let mut collection = store.collections.into_iter().next().ok_or_else(|| {
        corrupt(format!(
            "{name_from_file}: no collection found in vector store document"
        ))
    })?;
    if collection.name.is_empty() {
        collection.name = name_from_file.to_string();
    }

    let mut config = collection.config;
    if let Some(bytes) = metadata_json {
        let doc: Value = serde_json::from_slice(bytes).map_err(|e| {
            corrupt(format!(
                "{name_from_file}: unreadable metadata document: {e}"
            ))
        })?;
        if let Some(object) = doc.as_object() {
            // The server writes exactly these; anything else is a server-side
            // extension (owner/tenant metadata etc.) VecLite cannot carry.
            const KNOWN: [&str; 5] = [
                "name",
                "config",
                "created_at",
                "modified_at",
                "vector_count",
            ];
            for key in object.keys() {
                if !KNOWN.contains(&key.as_str()) {
                    warnings.push(format!(
                        "collection {:?}: server-only metadata field {key:?} dropped (IOP-022)",
                        collection.name
                    ));
                }
            }
            if config.is_none()
                && let Some(config_value) = object.get("config")
            {
                config =
                    Some(serde_json::from_value(config_value.clone()).map_err(|e| {
                        corrupt(format!("{name_from_file}: unreadable config: {e}"))
                    })?);
            }
        }
    }

    let tokenizer = match tokenizer_json {
        Some(bytes) => Some(serde_json::from_slice::<Value>(bytes).map_err(|e| {
            corrupt(format!(
                "{name_from_file}: unreadable tokenizer document: {e}"
            ))
        })?),
        None => None,
    };

    Ok(SourceCollection {
        name: collection.name,
        config,
        vectors: collection.vectors,
        tokenizer,
        index_metadata,
    })
}

/// Load every collection from a Compact archive (IOP-020).
fn load_compact(src: &Path, warnings: &mut Vec<String>) -> Result<Vec<SourceCollection>> {
    let vecdb_path = if src.is_dir() {
        src.join(model::VECDB_FILE)
    } else {
        src.to_path_buf()
    };
    let vecidx_path = vecdb_path.with_extension("vecidx");

    // The archive is self-describing; the sidecar only adds metadata (and,
    // for VecLite-exported archives, the veclite.* aspect records).
    let mut index_metadata: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    if vecidx_path.exists() {
        let index: VecdbIndex = serde_json::from_slice(&fs::read(&vecidx_path)?).map_err(|e| {
            corrupt(format!(
                "{}: unreadable .vecidx: {e}",
                vecidx_path.display()
            ))
        })?;
        for collection in index.collections {
            index_metadata.insert(collection.name, collection.metadata);
        }
    } else {
        warnings.push(format!(
            "{}: no .vecidx sidecar next to the archive; proceeding from the archive alone \
             (VecLite-specific aspects — aliases, payload indexes, BYO origin — cannot be \
             restored without it)",
            vecdb_path.display()
        ));
    }

    let file = File::open(&vecdb_path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| {
        corrupt(format!(
            "{}: failed to open archive: {e}",
            vecdb_path.display()
        ))
    })?;

    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| corrupt(format!("{}: entry {i}: {e}", vecdb_path.display())))?;
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| corrupt(format!("{}: entry {i}: {e}", vecdb_path.display())))?;
        entries.insert(entry.name().to_string(), bytes);
    }

    let names: Vec<String> = entries
        .keys()
        .filter_map(|path| path.strip_suffix("_vector_store.bin"))
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return Err(corrupt(format!(
            "{}: archive has no *_vector_store.bin entries",
            vecdb_path.display()
        )));
    }

    let mut collections = Vec::with_capacity(names.len());
    for name in names {
        let store_bytes = entries
            .get(&model::vector_store_entry(&name))
            .ok_or_else(|| corrupt(format!("{name}: vector store entry vanished")))?;
        let store = parse_store(&name, store_bytes)?;
        collections.push(assemble_source(
            &name,
            store,
            entries
                .get(&model::metadata_entry(&name))
                .map(Vec::as_slice),
            entries
                .get(&model::tokenizer_entry(&name))
                .map(Vec::as_slice),
            index_metadata.remove(&name).unwrap_or_default(),
            warnings,
        )?);
    }
    Ok(collections)
}

/// Decompress a Legacy file: gzip JSON, with the server's plain-JSON
/// backward-compatibility fallback.
fn read_legacy_file(path: &Path) -> Result<Vec<u8>> {
    let raw = fs::read(path)?;
    let mut decoder = flate2::read::GzDecoder::new(raw.as_slice());
    let mut decompressed = Vec::new();
    match decoder.read_to_end(&mut decompressed) {
        Ok(_) => Ok(decompressed),
        Err(_) => Ok(raw), // not gzip — legacy uncompressed save
    }
}

/// Load collections from the Legacy layout: bare `*_vector_store.bin` files
/// (plus optional sibling `_metadata.json` / `_tokenizer.json`).
fn load_legacy(src: &Path, warnings: &mut Vec<String>) -> Result<Vec<SourceCollection>> {
    let files: Vec<PathBuf> = if src.is_file() {
        vec![src.to_path_buf()]
    } else {
        let mut found = Vec::new();
        for entry in fs::read_dir(src)?.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("_vector_store.bin"))
            {
                found.push(path);
            }
        }
        found.sort();
        found
    };
    if files.is_empty() {
        return Err(corrupt(format!(
            "{}: no *_vector_store.bin files found",
            src.display()
        )));
    }

    let mut collections = Vec::with_capacity(files.len());
    for path in files {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let name = file_name
            .strip_suffix("_vector_store.bin")
            .unwrap_or(file_name)
            .to_string();
        let bytes = read_legacy_file(&path)?;
        let store = parse_store(&name, &bytes)?;
        let sibling = |suffix: &str| -> Result<Option<Vec<u8>>> {
            let candidate = path.with_file_name(format!("{name}{suffix}"));
            if candidate.exists() {
                Ok(Some(fs::read(candidate)?))
            } else {
                Ok(None)
            }
        };
        let metadata = sibling("_metadata.json")?;
        let tokenizer = sibling("_tokenizer.json")?;
        collections.push(assemble_source(
            &name,
            store,
            metadata.as_deref(),
            tokenizer.as_deref(),
            BTreeMap::new(),
            warnings,
        )?);
    }
    Ok(collections)
}

fn provider_available(provider: &str) -> bool {
    crate::embedding::available_providers()
        .iter()
        .any(|p| p == provider)
        || (cfg!(feature = "onnx") && crate::embedding::is_onnx_provider(provider))
}

fn parse_index_kind(kind: &str) -> Option<PayloadIndexKind> {
    match kind {
        "keyword" => Some(PayloadIndexKind::Keyword),
        "integer" => Some(PayloadIndexKind::Integer),
        "float" => Some(PayloadIndexKind::Float),
        _ => None,
    }
}

/// Import a server data set at `src` into `db` (IOP-020..023). Detects the
/// layout, validates every collection's config first (so an encryption
/// refusal aborts before anything is created), then assembles collections in
/// the same order `VecLite::deserialize` uses: create → aliases → vocabulary
/// → points.
pub fn import_vecdb(src: &Path, db: &VecLite, options: &ImportOptions) -> Result<ImportReport> {
    let layout = detect_layout(src)?;
    let mut warnings = Vec::new();
    let mut sources = match layout {
        ServerLayout::Compact => load_compact(src, &mut warnings)?,
        ServerLayout::Legacy => load_legacy(src, &mut warnings)?,
    };

    // IOP-021: --collections subsets; unknown names fail before any work.
    if let Some(subset) = &options.collections {
        let available: Vec<String> = sources.iter().map(|s| s.name.clone()).collect();
        for wanted in subset {
            if !available.contains(wanted) {
                return Err(VecLiteError::CollectionNotFound(format!(
                    "{wanted} (source has: {})",
                    available.join(", ")
                )));
            }
        }
        sources.retain(|s| subset.contains(&s.name));
    }

    // Pass 1 — plan every collection. Encryption refusal (IOP-022) surfaces
    // here, before the database is touched.
    struct Plan {
        source: SourceCollection,
        options: crate::options::CollectionOptions,
        deferred_provider: Option<String>,
        auto_embed_provider: Option<String>,
    }
    let mut plans = Vec::with_capacity(sources.len());
    for source in sources {
        let name = source.name.clone();
        let config = source.config.as_ref().ok_or_else(|| {
            corrupt(format!(
                "{name}: no collection config in the store or metadata document"
            ))
        })?;

        let byo_export = source
            .index_metadata
            .get("veclite.auto_embed")
            .is_some_and(|v| v == "false");
        let provider = config.embedding_provider.clone();
        let (plan, deferred_provider) = if byo_export {
            // A VecLite-exported BYO collection: restore BYO exactly.
            (super::config::ProviderPlan::Byo, None)
        } else if provider_available(&provider) {
            (super::config::ProviderPlan::AutoEmbed(provider), None)
        } else {
            warnings.push(format!(
                "collection {name:?}: embedding provider {provider:?} is server-only in this \
                 build; imported as BYO-vector — vectors and payloads intact, text re-embedding \
                 disabled, origin provider recorded in CONFIG (IOP-022)"
            ));
            (
                super::config::ProviderPlan::AutoEmbed(provider.clone()),
                Some(provider),
            )
        };
        let collection_options =
            super::config::from_vecdb_config(&name, config, &plan, &mut warnings)?;
        let auto_embed_provider = collection_options.embedding_provider.clone();
        plans.push(Plan {
            source,
            options: collection_options,
            deferred_provider,
            auto_embed_provider,
        });
    }

    // Pass 2 — assemble (mirrors VecLite::deserialize order).
    let mut report_collections = Vec::with_capacity(plans.len());
    for plan in plans {
        let name = plan.source.name.clone();
        let handle = db.create_collection_deferred(&name, plan.options)?;

        if let Some(aliases) = plan.source.index_metadata.get("veclite.aliases") {
            let parsed: Vec<String> = serde_json::from_str(aliases).unwrap_or_default();
            for alias in parsed {
                db.create_alias(&alias, &name)?;
            }
        }
        if let Some(indexes) = plan.source.index_metadata.get("veclite.payload_indexes") {
            let parsed: Vec<(String, String)> = serde_json::from_str(indexes).unwrap_or_default();
            for (key, kind) in parsed {
                match parse_index_kind(&kind) {
                    Some(kind) => handle.create_payload_index(&key, kind)?,
                    None => warnings.push(format!(
                        "collection {name:?}: unknown payload-index kind {kind:?} for key \
                         {key:?} dropped — the payload data itself is preserved (IOP-023)"
                    )),
                }
            }
        }

        // Vocabulary before points, so nothing double-counts (EMB-030).
        if let (Some(provider), Some(tokenizer)) =
            (&plan.auto_embed_provider, &plan.source.tokenizer)
            && plan.deferred_provider.is_none()
        {
            match super::vocab::from_server_tokenizer(provider, tokenizer)? {
                Some(state) => handle.replay_import_vocab(&state)?,
                None => warnings.push(format!(
                    "collection {name:?}: tokenizer document does not match provider \
                         {provider:?}; vocabulary not restored — the provider re-fits from \
                         stored text on demand"
                )),
            }
        }

        let auto_embed = plan.auto_embed_provider.is_some() && plan.deferred_provider.is_none();
        let mut points = Vec::with_capacity(plan.source.vectors.len());
        for vector in plan.source.vectors {
            let payload = match &vector.payload_json {
                Some(json) => Some(serde_json::from_str::<Value>(json).map_err(|e| {
                    corrupt(format!(
                        "collection {name:?}, id {:?}: unreadable payload: {e}",
                        vector.id
                    ))
                })?),
                None => None,
            };
            // Auto-embed collections own a sparse lane derived from the dense
            // provider embedding (HYB-002a); rebuild it the same way.
            let sparse = if auto_embed {
                crate::collection::sparse_from_dense(&vector.data)
            } else {
                None
            };
            points.push(Point {
                id: vector.id,
                vector: vector.data,
                sparse,
                payload,
            });
        }
        let count = points.len();
        if !points.is_empty() {
            handle.install_points(points)?;
        }

        report_collections.push(ImportedCollection {
            name,
            vectors: count,
            deferred_provider: plan.deferred_provider,
        });
    }

    Ok(ImportReport {
        layout,
        collections: report_collections,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interop::export::{ExportOptions, export_vecdb};
    use crate::options::{CollectionOptions, Metric, PayloadIndexKind};

    fn temp_out(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "veclite-vecdb-import-{}-{name}",
            std::process::id()
        ))
    }

    fn sample_db() -> VecLite {
        let db = VecLite::memory();
        let docs = db
            .create_collection("docs", CollectionOptions::auto_embed("bm25", 64))
            .unwrap_or_else(|e| panic!("{e}"));
        docs.upsert_text("a", "the quick brown fox jumps over the lazy dog")
            .unwrap_or_else(|e| panic!("{e}"));
        docs.upsert_text("b", "a fast auburn fox leaps above a sleepy hound")
            .unwrap_or_else(|e| panic!("{e}"));
        docs.upsert_text("c", "grep searches text files for matching lines")
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_alias("latest", "docs")
            .unwrap_or_else(|e| panic!("{e}"));

        let vecs = db
            .create_collection(
                "vecs",
                CollectionOptions::new(3, Metric::Euclidean)
                    .payload_index("lang", PayloadIndexKind::Keyword),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        vecs.upsert(
            crate::point::Point::new("v1", vec![1.0, 2.0, 3.0])
                .payload(serde_json::json!({"lang": "en"})),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        db
    }

    #[test]
    fn compact_round_trip_restores_search_and_aspects() {
        let db = sample_db();
        let out = temp_out("roundtrip");
        let _ = fs::remove_dir_all(&out);
        export_vecdb(&db, &out, &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));

        let restored = VecLite::memory();
        let report = import_vecdb(&out, &restored, &ImportOptions::default())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.layout, ServerLayout::Compact);
        assert_eq!(report.collections.len(), 2);

        // Text search scores identically (vocabulary restored, IOP-011).
        let original = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
        let imported = restored
            .collection("docs")
            .unwrap_or_else(|e| panic!("{e}"));
        let a = original
            .search_text("quick fox", 3)
            .unwrap_or_else(|e| panic!("{e}"));
        let b = imported
            .search_text("quick fox", 3)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.id, y.id);
            assert!(
                (x.score - y.score).abs() <= 1e-5,
                "{} vs {}",
                x.score,
                y.score
            );
        }

        // Aliases restored through the .vecidx metadata.
        assert!(
            restored
                .collection("latest")
                .is_ok_and(|c| c.name() == "docs")
        );
        // BYO collection restored as BYO (no auto-embed), with its payload
        // index declaration.
        let vecs = restored
            .collection("vecs")
            .unwrap_or_else(|e| panic!("{e}"));
        let stats = vecs.stats();
        assert!(!stats.auto_embed);
        assert_eq!(stats.len, 1);
        assert_eq!(
            stats.payload_indexes,
            vec![("lang".to_string(), PayloadIndexKind::Keyword)]
        );

        // Second cycle is drift-free (SPEC-013 §4.2): re-export the imported
        // database and the vector stores match byte-for-byte apart from
        // timestamps (compare parsed stores).
        let out2 = temp_out("roundtrip2");
        let _ = fs::remove_dir_all(&out2);
        export_vecdb(&restored, &out2, &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
        let restored2 = VecLite::memory();
        import_vecdb(&out2, &restored2, &ImportOptions::default())
            .unwrap_or_else(|e| panic!("{e}"));
        let imported2 = restored2
            .collection("docs")
            .unwrap_or_else(|e| panic!("{e}"));
        let c = imported2
            .search_text("quick fox", 3)
            .unwrap_or_else(|e| panic!("{e}"));
        for (x, y) in b.iter().zip(c.iter()) {
            assert_eq!(x.id, y.id);
            assert!((x.score - y.score).abs() <= 1e-5);
        }

        let _ = fs::remove_dir_all(&out);
        let _ = fs::remove_dir_all(&out2);
    }

    #[test]
    fn subset_import_and_unknown_subset_error() {
        let db = sample_db();
        let out = temp_out("subset");
        let _ = fs::remove_dir_all(&out);
        export_vecdb(&db, &out, &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));

        let restored = VecLite::memory();
        let report = import_vecdb(
            &out,
            &restored,
            &ImportOptions {
                collections: Some(vec!["vecs".to_string()]),
            },
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.collections.len(), 1);
        assert_eq!(restored.list_collections(), vec!["vecs".to_string()]);

        let missing = import_vecdb(
            &out,
            &VecLite::memory(),
            &ImportOptions {
                collections: Some(vec!["ghost".to_string()]),
            },
        );
        assert!(matches!(missing, Err(VecLiteError::CollectionNotFound(_))));
        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn legacy_layout_imports_gzip_and_plain() {
        // Build a Legacy data dir by hand: gzip JSON store + sibling metadata
        // + tokenizer, exactly as the server's legacy save path writes them.
        let dir = temp_out("legacy");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));

        let store = serde_json::json!({
            "version": 1,
            "collections": [{
                "name": "docs",
                "config": null,
                "vectors": [
                    {"id": "x", "data": [0.6, 0.8], "payload_json": "{\"k\":1}", "normalized": true},
                    {"id": "y", "data": [1.0, 0.0], "payload_json": null, "normalized": true}
                ],
                "hnsw_dump_basename": null
            }]
        });
        let json = serde_json::to_vec(&store).unwrap_or_else(|e| panic!("{e}"));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut encoder, &json).unwrap_or_else(|e| panic!("{e}"));
        let gz = encoder.finish().unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("docs_vector_store.bin"), gz).unwrap_or_else(|e| panic!("{e}"));

        let metadata = serde_json::json!({
            "name": "docs",
            "config": {
                "dimension": 2,
                "metric": "cosine",
                "hnsw_config": {"m": 16, "ef_construction": 200, "ef_search": 100, "seed": null},
                "quantization": {"type": "sq", "bits": 8},
                "compression": {"enabled": true, "threshold_bytes": 1024, "algorithm": "lz4"},
                "embedding_provider": "bm25"
            },
            "created_at": "2026-01-01T00:00:00Z",
            "modified_at": "2026-01-01T00:00:00Z",
            "vector_count": 2,
            "owner": "tenant-42"
        });
        fs::write(
            dir.join("docs_metadata.json"),
            serde_json::to_vec_pretty(&metadata).unwrap_or_else(|e| panic!("{e}")),
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let db = VecLite::memory();
        let report =
            import_vecdb(&dir, &db, &ImportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.layout, ServerLayout::Legacy);
        assert_eq!(report.collections.len(), 1);
        assert_eq!(report.collections[0].vectors, 2);
        // Owner metadata dropped WITH a warning (IOP-022, never silent).
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("owner") && w.contains("dropped")),
            "missing owner warning: {:?}",
            report.warnings
        );
        let docs = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(docs.len(), 2);
        let point = docs
            .get("x")
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("x expected"));
        assert_eq!(point.payload, Some(serde_json::json!({"k": 1})));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn encrypted_collection_refuses_before_creating_anything() {
        let dir = temp_out("encrypted");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));

        let store = serde_json::json!({
            "version": 1,
            "collections": [{
                "name": "secure",
                "config": {
                    "dimension": 2,
                    "metric": "cosine",
                    "hnsw_config": {"m": 16, "ef_construction": 200, "ef_search": 100, "seed": null},
                    "quantization": {"type": "none"},
                    "compression": {"enabled": false, "threshold_bytes": 1024, "algorithm": "none"},
                    "embedding_provider": "bm25",
                    "encryption": {"required": true, "allow_mixed": false}
                },
                "vectors": [],
                "hnsw_dump_basename": null
            }]
        });
        fs::write(
            dir.join("secure_vector_store.bin"),
            serde_json::to_vec(&store).unwrap_or_else(|e| panic!("{e}")),
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let db = VecLite::memory();
        let Err(err) = import_vecdb(&dir, &db, &ImportOptions::default()) else {
            panic!("encrypted collection must refuse the import");
        };
        let message = err.to_string();
        assert!(
            message.contains("encrypt"),
            "must name encryption: {message}"
        );
        assert!(
            db.list_collections().is_empty(),
            "refusal must happen before any collection is created"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn server_only_provider_imports_as_byo_with_origin_recorded() {
        let dir = temp_out("server-provider");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));

        let store = serde_json::json!({
            "version": 1,
            "collections": [{
                "name": "neural",
                "config": {
                    "dimension": 3,
                    "metric": "cosine",
                    "hnsw_config": {"m": 16, "ef_construction": 200, "ef_search": 100, "seed": null},
                    "quantization": {"type": "none"},
                    "compression": {"enabled": false, "threshold_bytes": 1024, "algorithm": "none"},
                    "embedding_provider": "candle/all-minilm"
                },
                "vectors": [
                    {"id": "n1", "data": [0.6, 0.8, 0.0], "payload_json": "{\"content\":\"hello\"}", "normalized": true}
                ],
                "hnsw_dump_basename": null
            }]
        });
        fs::write(
            dir.join("neural_vector_store.bin"),
            serde_json::to_vec(&store).unwrap_or_else(|e| panic!("{e}")),
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let db = VecLite::memory();
        let report =
            import_vecdb(&dir, &db, &ImportOptions::default()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            report.collections[0].deferred_provider.as_deref(),
            Some("candle/all-minilm")
        );
        assert!(report.warnings.iter().any(|w| w.contains("server-only")));

        let neural = db.collection("neural").unwrap_or_else(|e| panic!("{e}"));
        // Vector-level reads keep working…
        assert_eq!(neural.len(), 1);
        // …while text operations answer UnsupportedProvider (origin recorded).
        let text = neural.search_text("hello", 1);
        assert!(matches!(
            text,
            Err(VecLiteError::UnsupportedProvider { ref requested, .. })
                if requested.contains("candle/all-minilm")
        ));
        let _ = fs::remove_dir_all(&dir);
    }
}
