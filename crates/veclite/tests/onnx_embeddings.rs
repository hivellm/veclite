//! Opt-in dense embeddings over ONNX Runtime (SPEC-005 §6). Compiled and run
//! only with `--features onnx`; the first run downloads the MiniLM weights
//! (the sole permitted network access, EMB-041), the air-gapped test then loads
//! them from a local path with no network.
//!
//!   cargo test -p hivellm-veclite --features onnx
//!
//! The `#[ignore]` fixture generator writes the file the base-build degradation
//! test (`onnx_degradation.rs`) reads:
//!   cargo test -p hivellm-veclite --features onnx -- --ignored write_degradation_fixture
#![cfg(feature = "onnx")]

use std::path::{Path, PathBuf};

use veclite::{CollectionOptions, OpenOptions, VecLite};

/// A stable temp cache dir for the downloaded model, shared across tests so the
/// weights download at most once per run.
fn model_cache() -> PathBuf {
    std::env::temp_dir().join("veclite-onnx-test-cache")
}

const MODEL: &str = "fastembed:all-MiniLM-L6-v2";
const DIM: usize = 384;

fn docs() -> Vec<(String, String, Option<serde_json::Value>)> {
    vec![
        (
            "cat".into(),
            "cats are small domesticated felines that purr".into(),
            None,
        ),
        (
            "dog".into(),
            "dogs are loyal domesticated canines that bark".into(),
            None,
        ),
        (
            "finance".into(),
            "the stock market rallied on strong quarterly earnings".into(),
            None,
        ),
        (
            "weather".into(),
            "a cold front brought heavy rain and thunderstorms".into(),
            None,
        ),
    ]
}

/// The Qdrant MiniLM export lays `model.onnx` + tokenizer files flat at its
/// snapshot root — the layout `fastembed:path:<dir>` expects.
const FLAT_MODEL: &str = "fastembed:Qdrant/all-MiniLM-L6-v2-onnx";

/// Ensure the flat-layout model is cached (download once) and return the on-disk
/// snapshot directory (`…/snapshots/<hash>/`) for the air-gapped path test.
fn ensure_snapshot_dir() -> PathBuf {
    // Constructing the provider through an honored cache dir downloads the
    // weights there (EMB-041); a throwaway open is enough to trigger it.
    let cache = model_cache();
    let _ = std::fs::create_dir_all(&cache);
    let dir = std::env::temp_dir().join("veclite-onnx-warm.veclite");
    let _ = std::fs::remove_file(&dir);
    {
        let db = VecLite::open_with(&dir, OpenOptions::new().model_cache_dir(&cache))
            .unwrap_or_else(|e| panic!("open: {e}"));
        db.create_collection("w", CollectionOptions::auto_embed(FLAT_MODEL, DIM))
            .unwrap_or_else(|e| panic!("warm create: {e}"));
    }
    let _ = std::fs::remove_file(&dir);

    let repo = cache.join("models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots");
    std::fs::read_dir(&repo)
        .unwrap_or_else(|e| panic!("no snapshots dir {repo:?}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.join("model.onnx").is_file())
        .unwrap_or_else(|| panic!("no snapshot with model.onnx under {repo:?}"))
}

#[test]
fn minilm_semantic_search() {
    let cache = model_cache();
    let path = std::env::temp_dir().join("veclite-onnx-e2e.veclite");
    let _ = std::fs::remove_file(&path);
    {
        let db = VecLite::open_with(&path, OpenOptions::new().model_cache_dir(&cache))
            .unwrap_or_else(|e| panic!("open: {e}"));
        let c = db
            .create_collection("docs", CollectionOptions::auto_embed(MODEL, DIM))
            .unwrap_or_else(|e| panic!("create: {e}"));
        c.upsert_text_batch(docs())
            .unwrap_or_else(|e| panic!("upsert_text: {e}"));

        // Dense neural embeddings place the meowing-pet query nearest the cat doc
        // and the market query nearest the finance doc — a lexical embedder would
        // miss both (no shared tokens).
        let pet = c
            .search_text("a pet that meows", 1)
            .unwrap_or_else(|e| panic!("search: {e}"));
        assert_eq!(pet[0].id, "cat", "expected cat, got {:?}", pet[0].id);

        let money = c
            .search_text("quarterly corporate profits beat expectations", 1)
            .unwrap_or_else(|e| panic!("search: {e}"));
        assert_eq!(
            money[0].id, "finance",
            "expected finance, got {:?}",
            money[0].id
        );

        // The embedder is 384-dim MiniLM.
        let Some(cat) = c.get("cat").unwrap_or_else(|e| panic!("{e}")) else {
            panic!("cat missing")
        };
        assert_eq!(cat.vector.len(), DIM);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn air_gapped_path_offline() {
    // Point a `fastembed:path:<dir>` provider at a local model directory. This
    // code path never contacts the network (EMB-041) — the model dir is all it
    // reads.
    let snap = ensure_snapshot_dir();
    let provider = format!("fastembed:path:{}", snap.display());

    let db = VecLite::memory();
    let c = db
        .create_collection("docs", CollectionOptions::auto_embed(&provider, DIM))
        .unwrap_or_else(|e| panic!("create (offline): {e}"));
    c.upsert_text_batch(docs())
        .unwrap_or_else(|e| panic!("upsert_text: {e}"));
    let hits = c
        .search_text("stormy rainy day", 1)
        .unwrap_or_else(|e| panic!("search: {e}"));
    assert_eq!(
        hits[0].id, "weather",
        "expected weather, got {:?}",
        hits[0].id
    );
}

#[test]
fn unknown_model_is_unsupported_provider() {
    let db = VecLite::memory();
    let result = db.create_collection(
        "x",
        CollectionOptions::auto_embed("fastembed:no-such-model", DIM),
    );
    let Err(err) = result else {
        panic!("unknown model must fail");
    };
    assert!(
        matches!(err, veclite::VecLiteError::UnsupportedProvider { .. }),
        "got {err:?}"
    );
}

/// Generate the fixture the base-build degradation test reads (EMB-023): a real
/// onnx collection with dense vectors, checkpointed to
/// `tests/fixtures/onnx_degradation.veclite`. Ignored by default (it writes into
/// the source tree and needs the model); run explicitly to regenerate.
#[test]
#[ignore = "regenerates the committed degradation fixture; run explicitly"]
fn write_degradation_fixture() {
    let cache = model_cache();
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/onnx_degradation.veclite");
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::remove_file(&out);
    let db = VecLite::open_with(&out, OpenOptions::new().model_cache_dir(&cache))
        .unwrap_or_else(|e| panic!("open: {e}"));
    let c = db
        .create_collection("docs", CollectionOptions::auto_embed(MODEL, DIM))
        .unwrap_or_else(|e| panic!("create: {e}"));
    c.upsert_text_batch(docs())
        .unwrap_or_else(|e| panic!("upsert_text: {e}"));
    db.checkpoint()
        .unwrap_or_else(|e| panic!("checkpoint: {e}"));
    drop(db); // release the file + advisory lock before touching the sidecar
    // Drop the (now-empty) WAL sidecar so only the committed image remains.
    let _ = std::fs::remove_file(out.with_extension("veclite-wal"));
    eprintln!("wrote fixture: {}", out.display());
}
