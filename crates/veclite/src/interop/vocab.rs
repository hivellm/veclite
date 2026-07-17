//! Embedding-vocabulary translation between VecLite `Embedder::export_state`
//! blobs and the server's `<collection>_tokenizer.json` convention (IOP-011).
//!
//! Both sides persist the same provider state — the server wraps it in a
//! `{"type": "<kind>", ...fields}` JSON document (see the server's
//! `save_vocabulary_json` impls), while VecLite's `export_state` is the bare
//! provider struct. The field names already agree (vendored providers,
//! ADR-0001); this module adds/strips the `type` tag, maps the provider ids
//! that differ (`bow` ↔ `bagofwords`, `char_ngram` ↔ `charngram`), projects
//! away VecLite-only bookkeeping the server would reject, and restores
//! VecLite-only fields (BM25 `k1`/`b`) at their server-parity values.

use serde_json::{Map, Value, json};

use crate::error::{Result, VecLiteError};

/// The server tokenizer `type` tag for a VecLite provider id, or `None` when
/// the server has no tokenizer form for it (`svd` server-side persists no
/// vocabulary; `fastembed:<model>` is stateless).
pub(crate) fn tokenizer_type_for(provider: &str) -> Option<&'static str> {
    match provider {
        "bm25" => Some("bm25"),
        "tfidf" => Some("tfidf"),
        "bow" => Some("bagofwords"),
        "char_ngram" => Some("charngram"),
        _ => None,
    }
}

fn corrupt(provider: &str, what: &str) -> VecLiteError {
    VecLiteError::Corrupt(format!("vecdb tokenizer ({provider}): {what}"))
}

fn state_object(provider: &str, state: &[u8]) -> Result<Map<String, Value>> {
    match serde_json::from_slice::<Value>(state) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(corrupt(provider, "state is not a JSON object")),
        Err(e) => Err(corrupt(provider, &format!("unreadable state: {e}"))),
    }
}

fn take_fields(
    provider: &str,
    mut source: Map<String, Value>,
    fields: &[&str],
    into: &mut Map<String, Value>,
) -> Result<()> {
    for field in fields {
        let value = source
            .remove(*field)
            .ok_or_else(|| corrupt(provider, &format!("missing field {field:?}")))?;
        into.insert((*field).to_string(), value);
    }
    Ok(())
}

/// Project a VecLite `export_state` blob to the server tokenizer JSON for
/// `provider`. `Ok(None)` when the provider has no server tokenizer form.
pub(crate) fn to_server_tokenizer(provider: &str, state: &[u8]) -> Result<Option<Vec<u8>>> {
    let Some(kind) = tokenizer_type_for(provider) else {
        return Ok(None);
    };
    let source = state_object(provider, state)?;
    let mut out = Map::new();
    out.insert("type".to_string(), json!(kind));
    // Exactly the fields each server `save_vocabulary_json` writes — extras
    // (BM25 `k1`/`b`, TF-IDF incremental tables) are VecLite bookkeeping the
    // server never persists.
    let fields: &[&str] = match provider {
        "bm25" => &[
            "dimension",
            "vocabulary",
            "doc_freq",
            "doc_lengths",
            "avg_doc_length",
            "total_docs",
        ],
        "tfidf" => &["dimension", "vocabulary", "idf_weights"],
        "bow" => &["dimension", "vocabulary"],
        "char_ngram" => &["dimension", "n", "ngram_map"],
        _ => return Ok(None),
    };
    take_fields(provider, source, fields, &mut out)?;
    serde_json::to_vec_pretty(&Value::Object(out))
        .map(Some)
        .map_err(|e| corrupt(provider, &format!("serialize: {e}")))
}

/// Rebuild a VecLite `import_state` blob from a server tokenizer document.
/// `Ok(None)` when the document's `type` doesn't belong to `provider` (the
/// caller degrades with a warning rather than importing a foreign vocabulary).
pub(crate) fn from_server_tokenizer(provider: &str, tokenizer: &Value) -> Result<Option<Vec<u8>>> {
    let Some(kind) = tokenizer_type_for(provider) else {
        return Ok(None);
    };
    let Some(doc) = tokenizer.as_object() else {
        return Err(corrupt(provider, "tokenizer is not a JSON object"));
    };
    match doc.get("type").and_then(Value::as_str) {
        Some(t) if t == kind => {}
        _ => return Ok(None),
    }
    let mut state = doc.clone();
    state.remove("type");
    match provider {
        // Server-parity constants the server never persists (its BM25 also
        // hard-codes them); VecLite's strict deserializer requires them.
        "bm25" => {
            state.entry("k1").or_insert(json!(1.5));
            state.entry("b").or_insert(json!(0.75));
            // Old server snapshots may predate some statistics fields; the
            // server loader defaults them, so mirror that leniency.
            state.entry("doc_freq").or_insert(json!({}));
            state.entry("doc_lengths").or_insert(json!([]));
            state.entry("avg_doc_length").or_insert(json!(0.0));
            state.entry("total_docs").or_insert(json!(0));
        }
        "tfidf" | "bow" | "char_ngram" => {}
        _ => return Ok(None),
    }
    serde_json::to_vec(&Value::Object(state))
        .map(Some)
        .map_err(|e| corrupt(provider, &format!("serialize: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::Embedder;

    #[test]
    fn bm25_state_round_trips_through_server_tokenizer() {
        let mut bm25 = crate::embedding::bm25::Bm25::new(64);
        bm25.fit(&["the quick brown fox", "jumps over the lazy dog"])
            .unwrap_or_else(|e| panic!("{e}"));
        let state = bm25.export_state().unwrap_or_else(|e| panic!("{e}"));

        let tokenizer = to_server_tokenizer("bm25", &state)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("bm25 must have a tokenizer form"));
        let doc: Value = serde_json::from_slice(&tokenizer).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc["type"], "bm25");
        assert!(doc.get("k1").is_none(), "k1 is VecLite-only bookkeeping");
        assert!(doc["vocabulary"].as_object().is_some_and(|m| !m.is_empty()));

        let restored_state = from_server_tokenizer("bm25", &doc)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("matching type must restore"));
        let mut restored = crate::embedding::bm25::Bm25::new(64);
        restored
            .import_state(&restored_state)
            .unwrap_or_else(|e| panic!("{e}"));
        // Identical scoring after the round trip (server-parity contract).
        let a = bm25.embed("quick fox").unwrap_or_else(|e| panic!("{e}"));
        let b = restored
            .embed("quick fox")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(a, b);
    }

    #[test]
    fn provider_id_mapping_covers_renames() {
        assert_eq!(tokenizer_type_for("bow"), Some("bagofwords"));
        assert_eq!(tokenizer_type_for("char_ngram"), Some("charngram"));
        assert_eq!(tokenizer_type_for("svd"), None);
        assert_eq!(tokenizer_type_for("fastembed:xyz"), None);
    }

    #[test]
    fn foreign_tokenizer_type_is_rejected_not_imported() {
        let doc =
            json!({"type": "tfidf", "dimension": 8, "vocabulary": {"a": 0}, "idf_weights": [1.0]});
        let out = from_server_tokenizer("bm25", &doc).unwrap_or_else(|e| panic!("{e}"));
        assert!(out.is_none());
    }

    #[test]
    fn minimal_server_bm25_snapshot_gets_parity_defaults() {
        let doc = json!({"type": "bm25", "dimension": 8, "vocabulary": {"a": 0}});
        let state = from_server_tokenizer("bm25", &doc)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("must restore"));
        let mut bm25 = crate::embedding::bm25::Bm25::new(8);
        bm25.import_state(&state).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(bm25.vocabulary_size(), 1);
    }

    #[test]
    fn tfidf_projection_drops_incremental_tables() {
        let mut tfidf = crate::embedding::tfidf::TfIdf::new(16);
        tfidf
            .fit(&["alpha beta", "beta gamma"])
            .unwrap_or_else(|e| panic!("{e}"));
        let state = tfidf.export_state().unwrap_or_else(|e| panic!("{e}"));
        let tokenizer = to_server_tokenizer("tfidf", &state)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("tfidf must have a tokenizer form"));
        let doc: Value = serde_json::from_slice(&tokenizer).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc["type"], "tfidf");
        assert!(doc.get("doc_frequencies").is_none());
        assert!(doc.get("total_docs").is_none());

        let back = from_server_tokenizer("tfidf", &doc)
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("must restore"));
        let mut restored = crate::embedding::tfidf::TfIdf::new(16);
        restored
            .import_state(&back)
            .unwrap_or_else(|e| panic!("{e}"));
        let a = tfidf.embed("alpha gamma").unwrap_or_else(|e| panic!("{e}"));
        let b = restored
            .embed("alpha gamma")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(a, b);
    }
}
