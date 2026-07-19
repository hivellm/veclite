# SPEC-013 — Vectorizer Interop (`.vecdb` import/export, graduation path)

| | |
|---|---|
| **Status** | Implemented (phase5d) — `veclite::interop` behind the `vecdb-interop` feature; graduation gate automated by `cargo xtask graduation` (shared corpus `tests/compat/vecdb/`, mirrored as `crates/vectorizer/tests/compat/veclite/` in the server repo) |
| **Phase / tasks** | Phase 5 · T5.5, T5.6 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-70, FR-72, FR-73; NFR-04 |
| **Planning source** | [07-vectorizer-compatibility.md](../vectorizer-lite/07-vectorizer-compatibility.md) |

Requirement IDs `IOP-xxx`. Behind cargo feature `vecdb-interop`; surfaced to users via the CLI (SPEC-014).

## 1. Compatibility contract (what is shared)

| Layer | Shared | Mechanism |
|---|---|---|
| Distance metrics, HNSW params, quantization encodings, SIMD, compression | identical | vendored byte-identical code (ADR-0001) verified by the shared conformance corpus |
| Collection config semantics | identical | `CollectionOptions` ↔ server `CollectionConfig` 1:1 |
| Embedding provider ids + vocabulary state | identical | same provider ids; state translates both ways |
| Filter model | v1 subset | SPEC-006 (no geo/nested) |
| On-disk file | **different** | `.veclite` vs `.vecdb`+`.vecidx`; logical import/export bridges |
| Wire protocol | n/a | VecLite has none |

- **IOP-001** Quantized vector blocks MUST translate losslessly in both directions (byte-identical vendored encodings, CORE-041 — no de-quantize/re-quantize round trip).
- **IOP-002** Behavior divergence between VecLite and the server for identical (config, data, query) is a **bug in one of them**. The shared conformance corpus (`tests/compat/`) runs in both repos' CI against golden results; exceptions MUST be documented in [07-compat](../vectorizer-lite/07-vectorizer-compatibility.md) with rationale (currently none).

## 2. Export (`.veclite` → `.vecdb`) — graduation path

- **IOP-010** `export` produces a `.vecdb` archive + `.vecidx` index that the server's `StorageReader`/`StorageMigrator` accepts, in the server's **Compact** layout (current format; Legacy is import-only).
- **IOP-011** Exported per collection: config (dimension, metric, HNSW params, quantization, compression), all live vectors (quantized reps preserved, IOP-001), payloads, declared payload-index kinds (server rebuilds the indexes), aliases, embedding provider id + vocabulary state (BM25 collections keep identical scoring server-side). Tombstoned data is never exported.
- **IOP-012** HNSW graph: exported when the server version supports graph import (negotiated by `.vecidx` metadata field); otherwise omitted — the server rebuilds. Either way is correct; the overlap gate (§4) is the arbiter.
- **IOP-013** Scope options: whole database or a named subset of collections. Auto-embed `_text` payload entries export as the server's stored-text convention.

## 3. Import (`.vecdb` → `.veclite`) — reverse path

- **IOP-020** Import MUST read **both** server layouts via `detect_format`: Compact `.vecdb` and Legacy `*_vector_store.bin`.
- **IOP-021** Collection selection: `--collections a,b` subsets; default all.
- **IOP-022** Server-only aspects degrade **with warnings, never silent, never fatal** — except encryption:
  | Server aspect | Import behavior |
  |---|---|
  | Owner/tenant metadata | dropped, warning |
  | Sharded collections | merged into one, warning |
  | Graph edges/relationships | dropped, warning (until VecLite grows graph support) |
  | Encrypted payloads | **refused** with a clear error (cannot decrypt) |
  | Server-only embedding providers (candle, OpenAI) | collection imported as **BYO-vector**: vectors + payloads intact, text re-embedding disabled, `origin_provider` recorded in CONFIG for later graduation |
- **IOP-023** Unsupported filter-model features in imported payload-index declarations (geo/nested) are dropped with warnings; the payload data itself is preserved verbatim.

## 4. Acceptance criteria (gate G5, task T5.6)

1. **Graduation round-trip**: standard benchmark corpus → VecLite → `export` → live Vectorizer server `import` → same top-10 queries: overlap ≥ 0.99 (NFR-04); BM25 text queries score-identical within 1e-5.
2. **Reverse round-trip**: server corpus → `import` → VecLite searches match server results at the same gate; re-export → server re-import is stable (no drift on a second cycle).
3. **Legacy layout**: at least one archived Legacy `.vecdb` fixture imports correctly.
4. **Degradation matrix** (IOP-022) covered by fixtures for each row, asserting warnings and the encrypted-refusal error.
5. Shared conformance corpus wired into both repos' CI (IOP-002).
