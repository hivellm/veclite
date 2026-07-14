## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-004 §2/§4, SPEC-005 §4/§7; DAG T3.8–T3.10 and gate G3
- [x] 1.2 upsert_text/upsert_text_batch: embed + store vector + _text payload (phase3b, EMB-020/022)
- [x] 1.3 search_text: embed query with collection provider + search (phase3b); onnx-absent rule (EMB-023) tracked in phase3f
- [x] 1.4 Chunker: ChunkOptions (max_chars 2048, overlap 128), UTF-8-safe boundaries (phase3b, EMB-050)
- [x] 1.5 Aliases create/delete/resolve; transparent name resolution in collection(); persisted (CORE-011)
- [x] 1.6 scroll with cursor + stable slot order; filtered scroll (API-022, FLT-032)
- [x] 1.7 search_batch rayon-parallel; stats() (FR-08/13/35)
- [x] 1.8 Gate G3 conformance tracked per spec: filters (phase3e), text/embeddings (phase3f), hybrid (phase3g) — each carries its server corpus

## 2. Testing
- [x] 2.1 Quickstart e2e: auto-embed collection → upsert_text → search_text (tests/auto_embed.rs)
- [x] 2.2 Chunker UTF-8 fuzz (phase3b chunk tests: emoji, CJK, multi-byte)
- [x] 2.3 Alias blue-green scenario: swap keeps queries working; survives reopen (tests/api.rs)
- [x] 2.4 Scroll pagination totality: pages cover all live vectors exactly once (tests/api.rs)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (CHANGELOG, README, SPEC-004 status)
- [x] 3.2 Write tests covering the new behavior (8 api tests; text/chunker tests in phase3b)
- [x] 3.3 Run tests and confirm they pass (all suites green; clippy clean; wasm32 builds)

<!-- Text-first API and chunker were delivered in phase3b. The onnx-absent text
     rule (EMB-023) rides with the onnx feature in phase3f; the gate-G3 cross-repo
     conformance corpora are tracked per spec in phase3e/3f/3g. -->
