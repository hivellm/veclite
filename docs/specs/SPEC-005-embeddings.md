# SPEC-005 — Embeddings & Text Pipeline

| | |
|---|---|
| **Status** | Implemented (phase3b + phase3f): Embedder trait + the four default sparse providers; auto-embed `upsert_text`/`search_text`; the chunker. phase3f added: incremental vocabulary (`Embedder::add_document`, EMB-030) with VOCAB-segment persistence and exact crash replay (checkpoint state + per-document WAL folding, refit journaling a full snapshot — EMB-032); `register_embedder` per database instance with deferred binding on reopen (EMB-011); the `svd` provider behind the `svd` feature (vendored, ndarray replaced by a plain Vec matrix); the EMB-023 deferral mechanism (`fastembed:*` collections open, vector ops work, text ops fail with the remedy); and the server parity corpus (fixtures generated from the unmodified Vectorizer provider sources, enforced at 1e-5 — acceptance 1). phase5c added the opt-in `onnx` feature: the `fastembed:<model>` / `fastembed:path:<dir>` providers over ONNX Runtime (fastembed 5.x), model download honoring `OpenOptions::model_cache_dir` (EMB-041) with a fully-offline local-path form, wasm32 excluded unconditionally (EMB-042), and heavy distribution artifacts (feature-forwarding in veclite-ffi/-py/-node + the Go `veclite_onnx` tag, wired into a dormant release workflow) that base packages never depend on (EMB-040, REL-021). Verified: MiniLM semantic search + air-gapped path e2e (`--features onnx`), and the base-build degradation (an onnx file opens, vector ops work, text ops fail `UnsupportedProvider`). |
| **Phase / tasks** | Phase 3, 5 · T3.5, T3.6, T3.8, T3.9, T5.4 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-36, FR-40–47 |
| **Planning source** | [05-embeddings.md](../vectorizer-lite/05-embeddings.md) |

Requirement IDs `EMB-xxx`.

## 1. Tiers (priority order)

1. **BYO vectors** — the primary path; zero embedding machinery involved (FR-40).
2. **Built-in sparse/lexical providers** — pure Rust, default build: `bm25`, `tfidf`, `bow`, `char_ngram` (+ `svd` behind feature).
3. **Optional dense neural** — `onnx` feature via `fastembed` (never in the default build).

Provider names are shared with the Vectorizer server so `embedding_provider` strings survive the graduation path (SPEC-013).

## 2. Provider matrix (normative)

| Provider id | Feature | Deps | Dimension | Notes |
|---|---|---|---|---|
| `bm25` | default | none | configurable, default 512 | k1 = 1.5, b = 0.75 (server parity); **the default auto-embed provider** |
| `tfidf` | default | none | vocab-sized | |
| `bow` | default | none | vocab-sized | |
| `char_ngram` | default | none | configurable | typo-tolerant lexical |
| `svd` | `svd` | `ndarray` | configurable | TF-IDF + truncated SVD |
| `fastembed:<model>` | `onnx` | `fastembed 5.x` (ONNX Runtime) | model-defined | `fastembed:path:<dir>` for air-gapped |
| *(none)* | — | — | any | BYO vectors |

- **EMB-001** Deliberately excluded (MUST NOT be ported): candle models, OpenAI/remote HTTP embeddings, and the server's hash-placeholder BERT/MiniLM providers.
- **EMB-002** Adding a provider id is a minor change; changing an existing provider's scoring/output for the same input+state is **breaking** (conformance corpus pins scores).

## 3. The `Embedder` trait

```rust
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, VecLiteError>;
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, VecLiteError>;
    fn dimension(&self) -> usize;
    fn export_state(&self) -> Result<Vec<u8>, VecLiteError>;   // opaque; empty if stateless
    fn import_state(&mut self, state: &[u8]) -> Result<(), VecLiteError>;
}
```

- **EMB-010** Synchronous and object-safe. State encoding is provider-private but MUST be stable across releases (it lives in VOCAB segments); a provider that changes its state format MUST version it internally and read all previous versions.
- **EMB-011** `register_embedder(name, Box<dyn Embedder>)` registers a custom provider **per Database instance** (never global). Registering a built-in name → `AlreadyExists`. Collections referencing a registered name fail with `UnsupportedProvider` when reopened in a process that hasn't re-registered it — the error message MUST say so.

## 4. Auto-embed collections

- **EMB-020** `CollectionOptions::auto_embed(provider, dimension)` records `embedding_provider` in the CONFIG segment. Reopening re-instantiates the provider and imports VOCAB state: a `.veclite` file MUST search identically on any machine with no network (FR-42). `auto_embed` selects the default metric; a caller wanting another one chains `CollectionOptions::metric(m)`. An auto-embed collection MUST be created with the metric the caller asked for — silently substituting the default persists a collection that differs from the request (the defect fixed in `phase6d`).
- **EMB-024** A query sharing no term with the vocabulary embeds to the zero vector. The text entry points MUST report that as an empty result set, not an error: it is an ordinary "nothing matched", and the caller supplied neither a vector nor a metric (API-023).
- **EMB-021** Fail-fast rules (no silent coercion — server issue #306 lesson):
  - unknown provider at creation → `UnsupportedProvider { requested, available }`; MUST NOT fall back to bm25;
  - provider native dimension ≠ requested dimension → `DimensionMismatch` at creation;
  - text ops (`upsert_text`/`search_text`) on a BYO collection → `InvalidArgument`;
  - vector ops on auto-embed collections are allowed (power users may mix), but the vector MUST match the collection dimension.
- **EMB-022** Original text is stored for auto-embed collections in PAYLOAD under the reserved key `_text` (enables `refit`, `reindex`, and sparse rebuild). User payloads MUST NOT use keys starting with `_` (reserved namespace; `InvalidArgument`). BYO collections store no text.
- **EMB-023** `onnx`-provider collections opened on a build without `onnx`: open succeeds; vector-level reads/searches work; the first **text** operation fails with `UnsupportedProvider` (API-051).

## 5. Vocabulary lifecycle (trainable sparse providers)

- **EMB-030** `upsert_text` updates vocabulary/document-frequency tables in memory and journals a `VOCAB_UPDATE` WAL entry. Entries MAY be coalesced (one delta per checkpoint), but recovery MUST reproduce the exact in-memory state (SPEC-003 acceptance 1 covers vocab ops).
- **EMB-031** Incremental IDF is approximate by design. `collection.refit()` recomputes the vocabulary from all stored `_text` and re-embeds every document — explicit, potentially slow, never automatic. `refit` on a collection with missing `_text` (imported BYO) → `InvalidArgument`.
- **EMB-032** After `refit`, previously returned scores MAY change; the operation journals as a full VOCAB snapshot + re-upsert batches (atomic per batch).

## 6. `onnx` feature

- **EMB-040** Pulls `fastembed` → ONNX Runtime. MUST ship as separate distribution artifacts (`veclite[onnx]`, `@veclite/onnx`, `VecLite.Onnx`, Go build tag `veclite_onnx`) so base installs stay lean (SPEC-016).
- **EMB-041** Model resolution: `fastembed:<model>` downloads to `OpenOptions::model_cache_dir` (default: platform cache dir) **only when that provider is explicitly constructed** — this is the sole permitted network access in the entire product, and `fastembed:path:<dir>` MUST work fully offline.
- **EMB-042** wasm32 builds exclude `onnx` unconditionally.

## 7. Chunker utility

```rust
use veclite::chunk::{Chunker, ChunkOptions};
let chunks = Chunker::new(ChunkOptions { max_chars: 2048, overlap: 128, ..Default::default() })
    .chunk(&text);   // Vec<Chunk { text, byte_range }>
```

- **EMB-050** Port of `file_loader/chunker.rs`: UTF-8-safe (never splits a code point), prefers sentence then word boundaries, honors `max_chars` and `overlap`. Pure function of its input — no file discovery, no watchers, no format conversion.
- **EMB-051** Chunk boundaries for a given (text, options) MUST be deterministic and are pinned by the conformance corpus (bindings expose the same chunker).

## 8. Acceptance criteria

1. Provider score parity vs the server on the shared corpus: identical scores within 1e-5 for bm25/tfidf/bow/char_ngram given identical state (T3.5).
2. Reopen test: build an auto-embed collection, close, reopen → `search_text` results identical (EMB-020).
3. Fail-fast matrix (EMB-021) covered by unit tests, incl. the no-silent-fallback assertions.
4. `refit` correctness: post-refit scores equal a from-scratch rebuild on the same corpus (T3.6).
5. Chunker UTF-8 fuzz (multi-byte, emoji, CJK) — no panics, no split code points (T3.9).
6. `onnx` e2e behind the feature: MiniLM embed + search, plus air-gapped local-path test (T5.4).
