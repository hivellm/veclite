## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-004 §2/§4, SPEC-005 §4/§7; DAG T3.8–T3.10 and gate G3
- [ ] 1.2 upsert_text/upsert_texts: embed + store vector + _text payload in one WAL entry (EMB-020/022)
- [ ] 1.3 search_text: embed query with collection provider + search; onnx-absent rule (EMB-023)
- [ ] 1.4 Chunker port: ChunkOptions (max_chars 2048, overlap 128), UTF-8-safe boundaries (EMB-050)
- [ ] 1.5 Aliases create/delete/resolve; transparent name resolution in collection() (CORE-011)
- [ ] 1.6 scroll with offset_id cursor + stable slot order; filtered scroll (API-022, FLT-032)
- [ ] 1.7 search_batch rayon-parallel; stats() and info() (FR-08/13/35)
- [ ] 1.8 Run gate G3 conformance: filters + hybrid + text vs server shared corpus

## 2. Testing
- [ ] 2.1 Quickstart e2e test: open → auto-embed collection → upsert_text → search_text
- [ ] 2.2 Chunker UTF-8 fuzz (emoji, CJK, multi-byte) — no split code points, deterministic boundaries
- [ ] 2.3 Alias blue-green scenario: rename + alias swap keeps queries working
- [ ] 2.4 Scroll pagination totality: pages cover all live vectors exactly once

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass (gate G3 evidence attached)
