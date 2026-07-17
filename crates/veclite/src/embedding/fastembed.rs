//! Opt-in dense neural embeddings over ONNX Runtime (SPEC-005 §6, EMB-040/041).
//! Compiled only behind the `onnx` feature and only on native targets (EMB-042);
//! base builds never pull `fastembed`/ORT/network crates (NFR-08).
//!
//! Two provider forms, both routed here from [`crate::embedding::build_provider`]:
//! - `fastembed:<model>` — a fastembed-supported model id (e.g.
//!   `all-MiniLM-L6-v2`); the ONNX weights download to the resolved model cache
//!   dir on first construction, the sole permitted network access in the product.
//! - `fastembed:path:<dir>` — a local model directory (`model.onnx` +
//!   tokenizer files); fully offline, for air-gapped use.
//!
//! A fastembed model is a fixed pretrained network — there is no trainable
//! vocabulary — so the [`Embedder`] state hooks are no-ops and the VOCAB segment
//! is empty (EMB-010); a reopened onnx collection re-derives nothing.

use std::path::Path;

use parking_lot::Mutex;

use fastembed::{
    EmbeddingModel, InitOptions, InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};

use crate::embedding::Embedder;
use crate::error::{Result, VecLiteError};

/// A dense embedder backed by a fastembed `TextEmbedding`. `embed` needs `&mut`
/// on the session, so it sits behind a `Mutex` to satisfy the `&self` trait
/// (the collection holds one shared instance).
pub struct OnnxEmbedder {
    inner: Mutex<TextEmbedding>,
    dim: usize,
}

/// Map a fastembed/anyhow error to a `VecLiteError` with context.
fn embed_err(context: &str, e: impl std::fmt::Display) -> VecLiteError {
    VecLiteError::InvalidArgument(format!("fastembed: {context}: {e}"))
}

/// The fastembed model ids VecLite recognizes, for the `UnsupportedProvider`
/// hint (the canonical `model_code`s; a caller may also pass the basename).
fn supported_model_names() -> Vec<String> {
    TextEmbedding::list_supported_models()
        .into_iter()
        .map(|m| format!("fastembed:{}", m.model_code))
        .collect()
}

/// Resolve a `fastembed:<model>` id to its enum, output dimension, and canonical
/// code. Matches the full `model_code` (`Qdrant/all-MiniLM-L6-v2-onnx`), its
/// basename (`all-MiniLM-L6-v2-onnx`), or the basename without an `-onnx`
/// suffix (`all-MiniLM-L6-v2`), case-insensitively.
fn resolve(name: &str) -> Result<(EmbeddingModel, usize)> {
    let want = name.to_ascii_lowercase();
    for m in TextEmbedding::list_supported_models() {
        let code = m.model_code.to_ascii_lowercase();
        let base = code.rsplit('/').next().unwrap_or(&code);
        if code == want || base == want || base.trim_end_matches("-onnx") == want {
            return Ok((m.model, m.dim));
        }
    }
    Err(VecLiteError::UnsupportedProvider {
        requested: format!("fastembed:{name}"),
        available: supported_model_names(),
    })
}

fn read_required(dir: &Path, file: &str) -> Result<Vec<u8>> {
    std::fs::read(dir.join(file)).map_err(|e| {
        VecLiteError::InvalidArgument(format!(
            "fastembed:path model dir {:?} is missing {file}: {e}",
            dir.display()
        ))
    })
}

/// Locate the ONNX weights in a local model dir: `model.onnx` if present, else
/// the first `*.onnx` file (some exports name it differently).
fn read_onnx(dir: &Path) -> Result<Vec<u8>> {
    let preferred = dir.join("model.onnx");
    if preferred.is_file() {
        return read_required(dir, "model.onnx");
    }
    let entries = std::fs::read_dir(dir).map_err(|e| {
        VecLiteError::InvalidArgument(format!(
            "fastembed:path model dir {:?} is unreadable: {e}",
            dir.display()
        ))
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("onnx") {
            return std::fs::read(&path)
                .map_err(|e| embed_err(&format!("reading {}", path.display()), e));
        }
    }
    Err(VecLiteError::InvalidArgument(format!(
        "fastembed:path model dir {:?} contains no .onnx file",
        dir.display()
    )))
}

impl OnnxEmbedder {
    /// Construct from a fastembed model id, downloading to `cache_dir` (EMB-041)
    /// — or the fastembed default cache when `None`. Fails with
    /// `UnsupportedProvider` for an unknown id, and `InvalidArgument` when the
    /// collection's declared `dimension` does not match the model's output.
    pub(crate) fn named(
        model: &str,
        dimension: usize,
        cache_dir: Option<&Path>,
    ) -> Result<OnnxEmbedder> {
        let (embedding_model, dim) = resolve(model)?;
        if dimension != dim {
            return Err(VecLiteError::InvalidArgument(format!(
                "fastembed:{model} produces {dim}-dim vectors, but the collection \
                 declared dimension {dimension}"
            )));
        }
        let mut opts = InitOptions::new(embedding_model).with_show_download_progress(false);
        if let Some(dir) = cache_dir {
            opts = opts.with_cache_dir(dir.to_path_buf());
        }
        let te = TextEmbedding::try_new(opts).map_err(|e| embed_err("loading model", e))?;
        Ok(OnnxEmbedder {
            inner: Mutex::new(te),
            dim,
        })
    }

    /// Construct from a local model directory (`model.onnx` + `tokenizer.json` +
    /// `config.json` + `special_tokens_map.json` + `tokenizer_config.json`),
    /// fully offline (EMB-041). The output dimension is probed once and must
    /// match the collection's declared `dimension`.
    pub(crate) fn from_path(dir: &str, dimension: usize) -> Result<OnnxEmbedder> {
        let base = Path::new(dir);
        let onnx = read_onnx(base)?;
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: read_required(base, "tokenizer.json")?,
            config_file: read_required(base, "config.json")?,
            special_tokens_map_file: read_required(base, "special_tokens_map.json")?,
            tokenizer_config_file: read_required(base, "tokenizer_config.json")?,
        };
        // Mean pooling is the sentence-transformers default (MiniLM/BGE/…); a
        // model needing another strategy would carry it in its own packaging.
        let model =
            UserDefinedEmbeddingModel::new(onnx, tokenizer_files).with_pooling(Pooling::Mean);
        let mut te = TextEmbedding::try_new_from_user_defined(model, InitOptionsUserDefined::new())
            .map_err(|e| embed_err("loading local model", e))?;
        // Probe the true output dimension; the collection must have declared it.
        let probe = te
            .embed(vec!["dimension probe"], None)
            .map_err(|e| embed_err("probing model dimension", e))?;
        let dim = probe.first().map_or(0, Vec::len);
        if dim != dimension {
            return Err(VecLiteError::InvalidArgument(format!(
                "fastembed:path:{dir} produces {dim}-dim vectors, but the collection \
                 declared dimension {dimension}"
            )));
        }
        Ok(OnnxEmbedder {
            inner: Mutex::new(te),
            dim,
        })
    }
}

impl Embedder for OnnxEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut te = self.inner.lock();
        let out = te
            .embed(vec![text], None)
            .map_err(|e| embed_err("embedding text", e))?;
        out.into_iter()
            .next()
            .ok_or_else(|| embed_err("embedding text", "no vector returned"))
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut te = self.inner.lock();
        te.embed(texts, None)
            .map_err(|e| embed_err("embedding batch", e))
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    // A pretrained ONNX model has no trainable vocabulary: fit/add_document are
    // no-ops and the exported state is empty (EMB-010) — a reopened collection
    // reuses the same fixed model, so nothing is re-derived.
    fn fit(&mut self, _corpus: &[&str]) -> Result<()> {
        Ok(())
    }

    fn export_state(&self) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }

    fn import_state(&mut self, _state: &[u8]) -> Result<()> {
        Ok(())
    }
}
