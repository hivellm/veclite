//! Graduation export (SPEC-013 §2): write a server data directory —
//! `vectorizer.vecdb` (ZIP/DEFLATE, Compact layout) plus the
//! `vectorizer.vecidx` SHA-256 sidecar — that the server's `StorageReader`
//! accepts (IOP-010).
//!
//! Per collection the archive carries the config, every live vector
//! (f32-exact, IOP-001), payloads (auto-embed `_text` renamed to the server's
//! stored-text key `content`, IOP-013), declared payload-index kinds, aliases,
//! and the embedding vocabulary as a server tokenizer entry (IOP-011).
//! Tombstoned data is never exported. The HNSW graph is omitted — the server
//! rebuilds it, which IOP-012 sanctions (current server builds never persist
//! graphs either).

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use super::model::{
    self, VecdbCollection, VecdbCollectionIndex, VecdbFileEntry, VecdbIndex, VecdbMetadata,
    VecdbStore, VecdbVector,
};
use crate::database::VecLite;
use crate::error::{Result, VecLiteError};

/// Scope options for [`export_vecdb`] (IOP-013).
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Export only these collections; `None` exports the whole database.
    pub collections: Option<Vec<String>>,
}

/// What an export wrote (the CLI summary, SPEC-014).
#[derive(Debug)]
pub struct ExportReport {
    /// Per-collection counts, in export order.
    pub collections: Vec<ExportedCollection>,
    /// Degradations surfaced during the projection (never silent, IOP-022).
    pub warnings: Vec<String>,
    /// The written archive (`<out_dir>/vectorizer.vecdb`).
    pub vecdb_path: PathBuf,
    /// The written sidecar (`<out_dir>/vectorizer.vecidx`).
    pub vecidx_path: PathBuf,
    /// Total uncompressed bytes across all archive entries.
    pub total_bytes: u64,
}

/// One exported collection in the [`ExportReport`].
#[derive(Debug)]
pub struct ExportedCollection {
    /// Collection name (also the archive entry prefix).
    pub name: String,
    /// Live vectors written (tombstones excluded).
    pub vectors: usize,
    /// Uncompressed bytes of this collection's archive entries.
    pub bytes: u64,
}

fn zip_err(what: &str, e: impl std::fmt::Display) -> VecLiteError {
    VecLiteError::Io(std::io::Error::other(format!("vecdb export: {what}: {e}")))
}

fn now_epoch_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Rename the reserved `_text` payload key to the server's stored-text key
/// `content` (IOP-013). A pre-existing `content` key wins — user data is never
/// overwritten; the collision is surfaced as a warning instead.
fn project_payload(
    collection: &str,
    id: &str,
    payload: Option<Value>,
    auto_embed: bool,
    warnings: &mut Vec<String>,
) -> Option<Value> {
    let mut payload = payload?;
    if auto_embed
        && let Some(object) = payload.as_object_mut()
        && let Some(text) = object.remove("_text")
    {
        match object.entry("content".to_string()) {
            serde_json::map::Entry::Vacant(slot) => {
                slot.insert(text);
            }
            serde_json::map::Entry::Occupied(_) => {
                object.insert("_text".to_string(), text);
                warnings.push(format!(
                    "collection {collection:?}, id {id:?}: payload already has a \
                             \"content\" key; kept \"_text\" as-is instead of the server \
                             stored-text convention"
                ));
            }
        }
    }
    Some(payload)
}

/// Mirror of the server's per-vector normalization probe (`PersistedVector::
/// from`): `normalized` is a measured property, not a metric assumption.
fn is_normalized(vector: &[f32]) -> bool {
    let norm_squared: f32 = vector.iter().map(|x| x * x).sum();
    (norm_squared.sqrt() - 1.0).abs() <= 1e-6
}

struct EntrySink<'a> {
    zip: ZipWriter<File>,
    options: SimpleFileOptions,
    files: &'a mut Vec<VecdbFileEntry>,
    bytes: u64,
}

impl EntrySink<'_> {
    fn add(&mut self, path: String, file_type: &str, content: &[u8]) -> Result<()> {
        self.zip
            .start_file(&path, self.options)
            .map_err(|e| zip_err("start entry", e))?;
        self.zip.write_all(content)?;
        self.bytes += content.len() as u64;
        self.files.push(VecdbFileEntry {
            path,
            size: content.len() as u64,
            // The server's own in-memory writer records the uncompressed size
            // here; its docs call the field approximate. Mirror that.
            compressed_size: content.len() as u64,
            checksum: model::sha256_hex(content),
            file_type: file_type.to_string(),
        });
        Ok(())
    }
}

/// Export `db` (or a named subset) to a server data directory at `out_dir`
/// (IOP-010..013). Writes `vectorizer.vecdb` + `vectorizer.vecidx` atomically
/// (`.tmp` + rename, matching the server's own writer).
pub fn export_vecdb(db: &VecLite, out_dir: &Path, options: &ExportOptions) -> Result<ExportReport> {
    let names = match &options.collections {
        Some(subset) => {
            // Resolve early so a typo fails before any file is written; the
            // handle lookup also resolves aliases to their targets.
            let mut names = Vec::with_capacity(subset.len());
            for name in subset {
                let resolved = db.collection(name)?.name();
                if !names.contains(&resolved) {
                    names.push(resolved);
                }
            }
            names
        }
        None => db.list_collections(),
    };

    fs::create_dir_all(out_dir)?;
    let vecdb_path = out_dir.join(model::VECDB_FILE);
    let vecidx_path = out_dir.join(model::VECIDX_FILE);
    let tmp_vecdb = out_dir.join(format!("{}.tmp", model::VECDB_FILE));
    let tmp_vecidx = out_dir.join(format!("{}.tmp", model::VECIDX_FILE));

    // alias target -> alias names (IOP-011: aliases travel in the index
    // metadata; the server has no alias slot in the archive itself).
    let mut alias_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (alias, target) in db.aliases() {
        alias_map.entry(target).or_default().push(alias);
    }

    let epoch = now_epoch_s();
    let timestamp = model::rfc3339_utc(epoch);
    let mut warnings = Vec::new();
    let mut report_collections = Vec::new();
    let mut index_collections = Vec::new();
    let mut total_bytes = 0u64;

    let zip_options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let mut zip = ZipWriter::new(File::create(&tmp_vecdb)?);

    for name in &names {
        let handle = db.collection(name)?;
        let config = handle.config();
        // Auto-embed keeps the provider id verbatim (07-compat: shared ids);
        // BYO gets the server default plus a metadata record of the truth.
        let auto_embed = config.embedding_provider.is_some();
        if auto_embed {
            // Settle pending text refits so the archive carries exactly the
            // vectors a search here would score — the graduation gate compares
            // pre-export VecLite results against the imported server (§4).
            handle.refit_if_dirty()?;
        }
        if !auto_embed {
            warnings.push(format!(
                "collection {name:?}: BYO-vector collection exported with the server default \
                 embedding_provider \"bm25\" (the server has no provider-less collections); \
                 the .vecidx records the BYO origin for reverse imports"
            ));
        }

        let vecdb_config = super::config::to_vecdb_config(name, config, &mut warnings);
        let live = handle.live_points();
        let vector_count = live.len();
        let mut vectors = Vec::with_capacity(vector_count);
        for (id, vector, payload, _sparse) in live {
            let payload = project_payload(name, &id, payload, auto_embed, &mut warnings);
            let payload_json = match payload {
                Some(value) => Some(serde_json::to_string(&value).map_err(|e| {
                    VecLiteError::InvalidArgument(format!(
                        "collection {name:?}, id {id:?}: payload not serializable: {e}"
                    ))
                })?),
                None => None,
            };
            let normalized = is_normalized(&vector);
            vectors.push(VecdbVector {
                id,
                data: vector,
                payload_json,
                normalized,
            });
        }

        let mut files = Vec::new();
        let mut sink = EntrySink {
            zip,
            options: zip_options,
            files: &mut files,
            bytes: 0,
        };

        let store = VecdbStore {
            version: model::STORE_VERSION,
            collections: vec![VecdbCollection {
                name: name.clone(),
                config: Some(vecdb_config),
                vectors,
                hnsw_dump_basename: None,
            }],
        };
        let store_json = serde_json::to_vec(&store)
            .map_err(|e| VecLiteError::InvalidArgument(format!("serialize {name:?}: {e}")))?;
        sink.add(model::vector_store_entry(name), "vectors", &store_json)?;

        // `store.collections` was just built with exactly one element.
        let config_for_metadata = store.collections.into_iter().next().and_then(|c| c.config);
        if let Some(config) = config_for_metadata {
            let metadata = VecdbMetadata {
                name: name.clone(),
                config,
                created_at: Some(timestamp.clone()),
                modified_at: Some(timestamp.clone()),
                vector_count,
            };
            let metadata_json = serde_json::to_vec_pretty(&metadata).map_err(|e| {
                VecLiteError::InvalidArgument(format!("serialize {name:?} metadata: {e}"))
            })?;
            sink.add(model::metadata_entry(name), "metadata", &metadata_json)?;
        }

        if let Some(provider) = &config.embedding_provider
            && let Some(state) = handle.export_vocab_state()?
        {
            match super::vocab::to_server_tokenizer(provider, &state)? {
                Some(tokenizer) => {
                    sink.add(model::tokenizer_entry(name), "tokenizer", &tokenizer)?;
                }
                None => warnings.push(format!(
                    "collection {name:?}: provider {provider:?} has no server tokenizer \
                         form; the server re-fits its vocabulary from stored text"
                )),
            }
        }

        let bytes = sink.bytes;
        zip = sink.zip;
        total_bytes += bytes;

        // VecLite-only aspects ride in the index metadata (the server ignores
        // unknown keys; reverse imports restore them): aliases, declared
        // payload-index kinds (the server rebuilds indexes), BYO origin.
        let mut metadata = BTreeMap::new();
        if let Some(aliases) = alias_map.get(name) {
            metadata.insert(
                "veclite.aliases".to_string(),
                serde_json::to_string(aliases).unwrap_or_default(),
            );
        }
        if !config.payload_indexes.is_empty() {
            let declared: Vec<(String, String)> = config
                .payload_indexes
                .iter()
                .map(|(key, kind)| (key.clone(), format!("{kind:?}").to_lowercase()))
                .collect();
            metadata.insert(
                "veclite.payload_indexes".to_string(),
                serde_json::to_string(&declared).unwrap_or_default(),
            );
        }
        if !auto_embed {
            metadata.insert("veclite.auto_embed".to_string(), "false".to_string());
        }

        index_collections.push(VecdbCollectionIndex {
            name: name.clone(),
            files,
            vector_count,
            dimension: config.dimension,
            metadata,
        });
        report_collections.push(ExportedCollection {
            name: name.clone(),
            vectors: vector_count,
            bytes,
        });
    }

    zip.finish().map_err(|e| zip_err("finish archive", e))?;

    let compressed_size: u64 = index_collections
        .iter()
        .flat_map(|c| c.files.iter())
        .map(|f| f.compressed_size)
        .sum();
    let index = VecdbIndex {
        version: model::VECIDX_VERSION.to_string(),
        created_at: timestamp.clone(),
        updated_at: timestamp,
        collections: index_collections,
        total_size: total_bytes,
        compressed_size,
        compression_ratio: if total_bytes > 0 {
            compressed_size as f64 / total_bytes as f64
        } else {
            0.0
        },
    };
    let index_json = serde_json::to_string_pretty(&index)
        .map_err(|e| VecLiteError::InvalidArgument(format!("serialize .vecidx: {e}")))?;
    fs::write(&tmp_vecidx, index_json)?;

    // Atomic publication, matching the server's writer: both files land only
    // if everything above succeeded.
    fs::rename(&tmp_vecdb, &vecdb_path)?;
    fs::rename(&tmp_vecidx, &vecidx_path)?;

    Ok(ExportReport {
        collections: report_collections,
        warnings,
        vecdb_path,
        vecidx_path,
        total_bytes,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::options::{CollectionOptions, Metric, PayloadIndexKind};
    use crate::point::Point;

    fn temp_out(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("veclite-vecdb-{}-{name}", std::process::id()))
    }

    fn read_entry(vecdb: &Path, entry: &str) -> Vec<u8> {
        let file = File::open(vecdb).unwrap_or_else(|e| panic!("{e}"));
        let mut zip = zip::ZipArchive::new(file).unwrap_or_else(|e| panic!("{e}"));
        let mut entry = zip.by_name(entry).unwrap_or_else(|e| panic!("{e}"));
        let mut buffer = Vec::new();
        entry
            .read_to_end(&mut buffer)
            .unwrap_or_else(|e| panic!("{e}"));
        buffer
    }

    #[test]
    fn export_writes_server_shaped_archive_and_index() {
        let db = VecLite::memory();
        let docs = db
            .create_collection("docs", CollectionOptions::auto_embed("bm25", 64))
            .unwrap_or_else(|e| panic!("{e}"));
        docs.upsert_text("a", "the quick brown fox jumps over the lazy dog")
            .unwrap_or_else(|e| panic!("{e}"));
        docs.upsert_text("b", "a fast auburn fox leaps above a sleepy hound")
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
            Point::new("v1", vec![1.0, 2.0, 3.0]).payload(serde_json::json!({"lang": "en"})),
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let out = temp_out("export-shape");
        let _ = fs::remove_dir_all(&out);
        let report =
            export_vecdb(&db, &out, &ExportOptions::default()).unwrap_or_else(|e| panic!("{e}"));

        assert!(report.vecdb_path.exists());
        assert!(report.vecidx_path.exists());
        assert_eq!(report.collections.len(), 2);

        // The vector store entry parses as the server's PersistedVectorStore.
        let store_bytes = read_entry(&report.vecdb_path, "docs_vector_store.bin");
        let store: VecdbStore =
            serde_json::from_slice(&store_bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(store.version, 1);
        assert_eq!(store.collections.len(), 1);
        let docs_out = &store.collections[0];
        assert_eq!(docs_out.name, "docs");
        assert_eq!(docs_out.vectors.len(), 2);
        assert!(docs_out.hnsw_dump_basename.is_none());
        // `_text` became the server's stored-text key (IOP-013); cosine
        // ingest normalization makes the measured `normalized` flag true.
        for vector in &docs_out.vectors {
            let payload: Value = serde_json::from_str(
                vector
                    .payload_json
                    .as_deref()
                    .unwrap_or_else(|| panic!("payload expected")),
            )
            .unwrap_or_else(|e| panic!("{e}"));
            assert!(payload.get("content").is_some());
            assert!(payload.get("_text").is_none());
            assert!(vector.normalized);
        }
        let config = docs_out
            .config
            .as_ref()
            .unwrap_or_else(|| panic!("config expected"));
        assert_eq!(config.embedding_provider, "bm25");

        // Tokenizer entry exists and carries the bm25 vocabulary.
        let tokenizer: Value =
            serde_json::from_slice(&read_entry(&report.vecdb_path, "docs_tokenizer.json"))
                .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(tokenizer["type"], "bm25");
        assert!(
            tokenizer["vocabulary"]
                .as_object()
                .is_some_and(|m| !m.is_empty())
        );

        // The .vecidx parses, checksums match the entry bytes, and VecLite
        // aspects ride in metadata.
        let index: VecdbIndex = serde_json::from_slice(
            &fs::read(&report.vecidx_path).unwrap_or_else(|e| panic!("{e}")),
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(index.version, "1.0");
        let docs_index = index
            .collections
            .iter()
            .find(|c| c.name == "docs")
            .unwrap_or_else(|| panic!("docs index entry"));
        assert_eq!(docs_index.vector_count, 2);
        assert_eq!(docs_index.dimension, 64);
        assert_eq!(
            docs_index
                .metadata
                .get("veclite.aliases")
                .map(String::as_str),
            Some(r#"["latest"]"#)
        );
        for file in &docs_index.files {
            let bytes = read_entry(&report.vecdb_path, &file.path);
            assert_eq!(model::sha256_hex(&bytes), file.checksum, "{}", file.path);
            assert_eq!(bytes.len() as u64, file.size, "{}", file.path);
        }
        let vecs_index = index
            .collections
            .iter()
            .find(|c| c.name == "vecs")
            .unwrap_or_else(|| panic!("vecs index entry"));
        assert_eq!(
            vecs_index
                .metadata
                .get("veclite.auto_embed")
                .map(String::as_str),
            Some("false")
        );
        assert_eq!(
            vecs_index
                .metadata
                .get("veclite.payload_indexes")
                .map(String::as_str),
            Some(r#"[["lang","keyword"]]"#)
        );

        // BYO collections surface the provider-default warning (never silent).
        assert!(report.warnings.iter().any(|w| w.contains("BYO-vector")));

        let _ = fs::remove_dir_all(&out);
    }

    #[test]
    fn export_subset_resolves_aliases_and_rejects_unknown() {
        let db = VecLite::memory();
        db.create_collection("docs", CollectionOptions::new(2, Metric::Cosine))
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_alias("current", "docs")
            .unwrap_or_else(|e| panic!("{e}"));

        let out = temp_out("export-subset");
        let _ = fs::remove_dir_all(&out);
        let report = export_vecdb(
            &db,
            &out,
            &ExportOptions {
                collections: Some(vec!["current".to_string()]),
            },
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(report.collections.len(), 1);
        assert_eq!(report.collections[0].name, "docs");

        let missing = export_vecdb(
            &db,
            &out,
            &ExportOptions {
                collections: Some(vec!["nope".to_string()]),
            },
        );
        assert!(matches!(
            missing,
            Err(VecLiteError::CollectionNotFound(ref name)) if name == "nope"
        ));
        let _ = fs::remove_dir_all(&out);
    }
}
