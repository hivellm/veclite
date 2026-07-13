## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-005 §6, SPEC-016 §3 (heavy artifacts); DAG T5.4
- [ ] 1.2 onnx cargo feature + fastembed provider construction with model-id parsing (fastembed:<model> / fastembed:path:<dir>)
- [ ] 1.3 model_cache_dir resolution honoring OpenOptions; download only at explicit provider construction (EMB-041)
- [ ] 1.4 Graceful degradation on non-onnx builds per EMB-023
- [ ] 1.5 Heavy packages: veclite-onnx wheel extra, @veclite/onnx, VecLite.Onnx, Go tag artifact wired into the release workflow (REL-021)
- [ ] 1.6 wasm32 unconditional exclusion (EMB-042)

## 2. Testing
- [ ] 2.1 MiniLM e2e behind the feature: embed + search quality smoke
- [ ] 2.2 Air-gapped test: fastembed:path with local model dir, network disabled
- [ ] 2.3 Degradation test: onnx-created file on default build — vector search works, text op fails typed
- [ ] 2.4 Base-package independence: default install has no ONNX artifacts in its tree

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
