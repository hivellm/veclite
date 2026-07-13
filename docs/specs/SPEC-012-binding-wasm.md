# SPEC-012 — WASM Package (`@veclite/wasm`)

| | |
|---|---|
| **Status** | Draft — OQ-3 (OPFS buffering strategy) must be resolved at task start |
| **Phase / tasks** | Phase 5 · T5.3 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-64, FR-65 |
| **Planning source** | [06-sdk-bindings.md §WASM](../vectorizer-lite/06-sdk-bindings.md) |

Requirement IDs `WASM-xxx`. Target: browsers, Deno, Bun, edge runtimes (Cloudflare Workers-class). Positioning: client-side semantic search over small/medium corpora (guideline ≤ ~500 k vectors), offline apps, extensions.

## 1. Build profile

- **WASM-001** `wasm32-unknown-unknown` via wasm-bindgen. Compile-time exclusions: file storage/pager (no filesystem), mmap, file locks, `rayon` (no threads by default), `onnx` (no ORT on wasm). Tier-1 (BYO vectors) and tier-2 (sparse providers) embeddings both work (EMB tiers).
- **WASM-002** SIMD via `simd128` (the vendored wasm kernels, CORE-001); a non-SIMD fallback build MUST exist for older runtimes, selected automatically by feature detection in the JS loader.
- **WASM-003** Single-threaded: no atomics/SharedArrayBuffer requirement in v1 (keeps COOP/COEP headers out of the adoption path).

## 2. Storage backends

| Backend | Availability | Semantics |
|---|---|---|
| In-memory | always | `VecLite.memory()` equivalent; lost on page unload |
| Bytes import/export | always | `db.serialize(): Uint8Array` / `VecLite.deserialize(bytes)` — the bytes are a **valid `.veclite` v1 file image** |
| OPFS | browsers with OPFS | persistent; explicit `save()` + optional autosave |

- **WASM-010** `serialize()` output MUST be readable by native VecLite (`open` on the written file) and vice versa — same format v1, no WASM-specific dialect. This is the interchange contract (edge/KV persistence, download/upload workflows).
- **WASM-011** OPFS backend (resolves OQ-3): the database operates on an in-memory image; `save()` checkpoint-serializes and writes atomically to OPFS (write temp + move). Optional `autosave: { afterWrites?: number, intervalMs?: number }`. Rationale: a sync core over an async storage API cannot page lazily without SharedArrayBuffer workers — full-image buffering is the v1 design; block-level paging is post-1.0.
- **WASM-012** Consequently, WASM database size is bounded by memory; the docs MUST state the sizing guidance and that `durability` semantics are: in-memory until `save()`/autosave (no WAL in WASM).

## 3. API surface

```ts
import { open, memory, deserialize } from "@veclite/wasm";

const db = await open({ opfs: "app.veclite", autosave: { afterWrites: 100 } }); // or memory()
const docs = await db.createCollection("docs", { dimension: 384, metric: "cosine" });
await docs.upsertBatch(points);                       // Float32Array in
const hits = await docs.search(query, { limit: 10, filter });
const notes = await db.createCollection("notes", { autoEmbed: "bm25", dimension: 512 });
await notes.upsertText("id", "text…", {});
const bytes = await db.serialize();                   // Uint8Array (valid .veclite file)
await db.save();                                      // OPFS backend
await db.close();
```

- **WASM-020** API mirrors SPEC-010 (camelCase, options objects) minus: `vacuum` (no-op returning immediately — compaction happens on `serialize`/`save`), file paths, `readOnly` file locks, `backgroundCheckpoint`. All methods are async (JS idiom) even though execution is synchronous inside the module.
- **WASM-021** Same error model as SPEC-010 (`VecLiteError` with `code`).

## 4. Size & performance budgets

- **WASM-030** Gzipped wasm bundle ≤ 3 MB (default providers included). CI enforces the budget.
- **WASM-031** 100 k × 384-dim corpus: p50 search < 15 ms in Chrome on the reference laptop profile (documented, benchmarked, not release-blocking for 1.0).

## 5. Acceptance criteria

1. Conformance corpus subset (everything except file-lock/mmap/vacuum/durability cases) green in headless Chrome + Node-wasm + Deno (gate G5).
2. Round-trip interchange test: native-written `.veclite` → `deserialize` in WASM → identical search results; WASM `serialize` → native `open` (WASM-010).
3. OPFS persistence test: write, reload page context, reopen, verify (WASM-011), including crash-between-autosaves losing at most the un-saved tail.
4. Bundle-size budget check in CI (WASM-030).
5. simd128 and fallback builds both pass the corpus.
