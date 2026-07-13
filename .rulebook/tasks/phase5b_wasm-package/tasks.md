## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-012 in full; DAG T5.3
- [ ] 1.2 wasm-bindgen crate with compile-time exclusions (no fs/mmap/locks/rayon/onnx) (WASM-001)
- [ ] 1.3 simd128 build + fallback build + JS loader feature detection (WASM-002)
- [ ] 1.4 serialize()/deserialize(): byte-identical .veclite v1 file images (WASM-010)
- [ ] 1.5 OPFS backend: in-memory image + atomic save (temp + move) + autosave options (WASM-011)
- [ ] 1.6 JS API surface per SPEC-010 conventions minus excluded ops; vacuum no-op (WASM-020)
- [ ] 1.7 Bundle-size budget check in CI: <= 3 MB gzipped (WASM-030)

## 2. Testing
- [ ] 2.1 Conformance subset green in headless Chrome + Node-wasm + Deno
- [ ] 2.2 Interchange round-trip: native file → deserialize → identical results; serialize → native open (WASM-010)
- [ ] 2.3 OPFS persistence across page-context reload; crash between autosaves loses at most unsaved tail
- [ ] 2.4 simd128 and fallback builds both pass the corpus

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation (sizing guidance per WASM-012)
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
