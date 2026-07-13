# 05 — Embeddings

## Philosophy

Three tiers, in order of expected real-world usage:

1. **Bring-your-own-vectors (primary).** Most production users embed with their own model (OpenAI, Cohere, local sentence-transformers, whatever) and hand VecLite finished vectors. This path has zero embedding machinery in VecLite and is the fastest.
2. **Built-in sparse/lexical providers (default build).** Pure-Rust, dependency-free, instant: BM25, TF-IDF, bag-of-words, char-n-gram. Good enough for keyword-ish semantic search, RAG over docs, and the hybrid-search sparse lane. This is what makes the 5-line quickstart work offline.
3. **Optional dense neural models (`onnx` feature).** `fastembed` (ONNX Runtime) for real dense embeddings (e.g., `all-MiniLM-L6-v2`, 384-dim) when the user opts into the heavy dependency.

This mirrors the server's provider architecture (`crates/vectorizer/src/embedding/`) with the same provider names, so collections keep meaningful `embedding_provider` strings across the graduation path.

## Provider matrix

| Provider | Feature | Deps | Dim | Vectorizer source | Notes |
|---|---|---|---|---|---|
| `bm25` | default | none | configurable (default 512) | `providers/bm25.rs` | k1=1.5, b=0.75 (server parity); **default provider** |
| `tfidf` | default | none | vocab-sized | `providers/tfidf.rs` | |
| `bow` | default | none | vocab-sized | `providers/bag_of_words.rs` | |
| `char_ngram` | default | none | configurable | `providers/char_ngram.rs` | typo-tolerant lexical |
| `svd` | `svd` | `ndarray` | configurable | `providers/svd.rs` | TF-IDF + truncated SVD |
| `fastembed:<model>` | `onnx` | `fastembed 5.x` → ONNX Runtime | model-defined (e.g. 384) | `providers/fastembed.rs` | model download on first use OR local path for air-gapped |
| *(none — BYO vectors)* | — | — | any | — | primary path |

Deliberately **not** ported: candle models (`real-models`), OpenAI HTTP embeddings, hash-placeholder BERT/MiniLM (`bert.rs`/`minilm.rs` placeholders would mislead embedded users — the server keeps them for API-compat reasons that don't apply here).

## The provider trait (sync, object-safe)

```rust
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, VecLiteError>;
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VecLiteError>;
    fn dimension(&self) -> usize;
    /// Serialized trainable state (vocabulary, idf table, SVD basis). Empty for stateless.
    fn export_state(&self) -> Result<Vec<u8>, VecLiteError>;
    fn import_state(&mut self, state: &[u8]) -> Result<(), VecLiteError>;
}
```

Simplified from the server's `EmbeddingProvider` (async removed; `save/load_vocabulary_json` generalized into opaque state so providers choose their own encoding).

**Custom providers**: `db.register_embedder("my-model", Box<dyn Embedder>)` — lets a host app plug an in-process model (e.g., its own ONNX session) behind the same auto-embed API. Registration is per-`Database` instance, never global.

## Auto-embed collections

```rust
let notes = db.create_collection("notes", CollectionOptions::auto_embed("bm25", 512))?;
notes.upsert_text("id", "some text", json!({}))?;   // embed + store in one call
notes.search_text("query", 10)?;                      // embed query + search
```

Rules (encoding the lessons of server issue #306 — no silent coercion):

1. Unknown provider name at `create_collection` → `UnsupportedProvider { requested, available }`. Never fall back to BM25 silently.
2. Provider native dimension conflicting with requested dimension → `DimensionMismatch` at creation time.
3. The collection records `embedding_provider` in its CONFIG segment; reopening the file re-instantiates the provider and imports its VOCAB state. A `.veclite` file created with `bm25` searches identically on any machine, with no network.
4. Collections with `onnx` providers open on a build without the `onnx` feature → error `UnsupportedProvider` at first text operation (vector-level reads/searches still work — the stored vectors don't need the model).

### Vocabulary lifecycle (sparse providers)

BM25/TF-IDF are trained on the corpus. VecLite handles this incrementally:

- `upsert_text` updates the vocabulary/document-frequency tables in memory and journals a `VOCAB_UPDATE` WAL entry (batched per checkpoint, not per doc).
- IDF drift: incremental updates approximate; `collection.refit()` recomputes the vocabulary from all stored texts and re-embeds (explicit, potentially slow — documented, never automatic).
- Original text is **stored** for auto-embed collections (in the PAYLOAD segment under a reserved `_text` key) precisely so `refit()`/`reindex()` are possible and hybrid sparse indexes can rebuild. BYO-vector collections store no text.

## Hybrid search and sparse vectors

The BM25 provider doubles as the sparse lane for hybrid search (server parity: `hybrid_search` with RRF from `rrf 0.1` semantics):

- Auto-embed BM25 collections maintain a SPARSE postings segment; `hybrid_query()` fuses dense + sparse with RRF (`alpha` balances).
- BYO users can supply explicit `SparseVector { indices, values }` per point for the sparse lane, with dense vectors in the main lane.

## Chunking utility

Ported from `file_loader/chunker.rs` (UTF-8-safe, sentence/word-boundary aware):

```rust
use veclite::chunk::{Chunker, ChunkOptions};
let chunks = Chunker::new(ChunkOptions { max_chars: 2048, overlap: 128, ..Default::default() })
    .chunk(&document_text);
```

Pure utility — no file discovery, no watchers, no format conversion. Users feed it strings.

## `onnx` feature details

- Adds `fastembed` (which pulls ONNX Runtime binaries). Binding packages ship this as a **separate artifact** (`veclite-onnx` extra / optional dependency) so the base install stays small — see [06-sdk-bindings.md](06-sdk-bindings.md).
- Model resolution: `fastembed:all-MiniLM-L6-v2` downloads to a cache dir (`OpenOptions::model_cache_dir`) on first use; `fastembed:path:/models/minilm` loads a local directory for air-gapped deployments.
- WASM builds exclude `onnx` unconditionally (no ORT in wasm32); tier 1 and 2 both work in WASM.
