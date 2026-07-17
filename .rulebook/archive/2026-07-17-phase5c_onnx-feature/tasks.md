## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-005 §6, SPEC-016 §3 (heavy artifacts); DAG T5.4 — plus a build+runtime feasibility check (fastembed 5.16 + ort 2.0-rc build here; MiniLM downloads and embeds at 384-dim)
- [x] 1.2 onnx cargo feature + fastembed provider construction with model-id parsing (fastembed:<model> / fastembed:path:<dir>) — `embedding/fastembed.rs::OnnxEmbedder` (Mutex<TextEmbedding>); `named` resolves via `list_supported_models()` (model_code / basename / -onnx-trimmed), `from_path` builds a `UserDefinedEmbeddingModel` (mean pooling) and probes the dim; `build_provider_with` routes `fastembed:*`
- [x] 1.3 model_cache_dir resolution honoring OpenOptions; download only at explicit provider construction (EMB-041) — `DatabaseInner.model_cache_dir` from `OpenOptions`, threaded to `build_provider_with` at create/open; `named` passes it to `InitOptions::with_cache_dir`; `path:` reads only the local dir (no network)
- [x] 1.4 Graceful degradation on non-onnx builds per EMB-023 — automatic: `fastembed:*` returns `UnsupportedProvider` off-feature → the lenient load path parks a `Missing` embedder, so the collection opens and serves vector ops while text ops fail typed
- [x] 1.5 Heavy packages: veclite-onnx wheel extra, @veclite/onnx, VecLite.Onnx, Go tag artifact wired into the release workflow (REL-021) — `onnx` feature-forwarding (`["veclite/onnx"]`) in veclite-ffi/-py/-node; Go `veclite_onnx` build tag (onnx.go / onnx_default.go, `OnnxBuild` const); dormant `veclite-release-onnx.yml` builds each heavy artifact `--features onnx` depending on the exact base version; READMEs updated
- [x] 1.6 wasm32 unconditional exclusion (EMB-042) — `fastembed` declared only in the non-wasm target table; the module + provider routing are `#[cfg(all(feature="onnx", not(target_arch="wasm32")))]`; wasm build verified clean

## 2. Testing
- [x] 2.1 MiniLM e2e behind the feature: embed + search quality smoke — `minilm_semantic_search` (`--features onnx`): a meowing-pet query ranks the cat doc, a profits query ranks the finance doc (dense-only matches a lexical embedder misses)
- [x] 2.2 Air-gapped test: fastembed:path with local model dir, network disabled — `air_gapped_path_offline`: `fastembed:path:<snapshot dir>`, a code path that never contacts the network, returns the semantically-nearest doc
- [x] 2.3 Degradation test: onnx-created file on default build — vector search works, text op fails typed — `onnx_degradation.rs` (base build) loads an onnx-created fixture (copied to temp): vector reads/search + BYO upsert work, `search_text`/`upsert_text` fail `UnsupportedProvider`
- [x] 2.4 Base-package independence: default install has no ONNX artifacts in its tree — `veclite`/`veclite-ffi` default `cargo tree` has no fastembed/ort/onnxruntime/hf-hub/network crates; the veclite-checks deny-list extended to catch a leak (NFR-08)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation — SPEC-005 status; module/provider doc comments; Go/C#/Node READMEs updated from "blocked" to the shipped opt-in tier; release-workflow header
- [x] 3.2 Write tests covering the new behavior — onnx_embeddings.rs (e2e + air-gapped + unknown-model + fixture generator) and onnx_degradation.rs (base-build degradation)
- [x] 3.3 Run tests and confirm they pass — onnx e2e 3/3 + degradation on both builds; full default `cargo test` green; clippy `-D warnings` clean on base and `--features onnx`; fmt clean; wasm32 builds
