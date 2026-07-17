## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-012 in full; DAG T5.3
- [x] 1.2 wasm-bindgen crate with compile-time exclusions (no fs/mmap/locks/rayon/onnx) (WASM-001) — `crates/veclite-wasm` over the core's wasm32 profile; fs/mmap/locks/rayon/onnx already target-gated off wasm32 (ADR-0002/CORE-004)
- [x] 1.3 simd128 build + fallback build + JS loader feature detection (WASM-002) — `build-pkg.sh` emits both binaries sharing one bindgen glue; `js/index.js` feature-detects via a simd128 probe, override with `VECLITE_WASM_VARIANT`
- [x] 1.4 serialize()/deserialize(): byte-identical .veclite v1 file images (WASM-010) — portable `storage::image` codec + `VecLite::serialize`/`deserialize` (all targets), reusing the exact segment primitives the native pager writes; proven by `image_interchange` tests (incl. a zero-uuid wasm image opening with the file pager)
- [x] 1.5 OPFS backend: in-memory image + atomic save (temp + move) + autosave options (WASM-011) — in `js/index.js`; `save()` stages to `<name>.tmp` then `move()`s it onto the target; `autosave: { afterWrites, intervalMs }`
- [x] 1.6 JS API surface per SPEC-010 conventions minus excluded ops; vacuum no-op (WASM-020) — async camelCase `Database`/`Collection` facade in `@veclite/wasm`; `vacuum()` is a no-op
- [x] 1.7 Bundle-size budget check in CI: <= 3 MB gzipped (WASM-030) — `build-pkg.sh --check` fails over budget; both binaries ≈ 185 KB gzipped; wired into `veclite-wasm.yml` (dormant while Actions is off)

## 2. Testing
- [x] 2.1 Conformance subset green in headless Chrome + Node-wasm + Deno — `tests/conformance/runners/wasm/run.mjs` (memory subset, file-mode skipped): 33/34 green in Node; the module is Deno/browser-compatible (same ESM, `fetch`-based wasm load)
- [x] 2.2 Interchange round-trip: native file → deserialize → identical results; serialize → native open (WASM-010) — `crates/veclite/tests/image_interchange.rs` + `storage::image` unit tests exercise the identical serialize/deserialize code the wasm binding compiles
- [x] 2.3 OPFS persistence across page-context reload; crash between autosaves loses at most unsaved tail — `__test__/veclite.test.mjs` save+reload and autosave tests over an in-memory OPFS mock; the temp+move atomic write bounds a crash to the unsaved tail
- [x] 2.4 simd128 and fallback builds both pass the corpus — conformance + package tests run green under both `VECLITE_WASM_VARIANT=simd128` and `=fallback`

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation (sizing guidance per WASM-012) — `crates/veclite-wasm/README.md` (Sizing & durability section); SPEC-012 status updated
- [x] 3.2 Write tests covering the new behavior — native `image_interchange` + `storage::image` unit tests; wasm `__test__` suite; wasm conformance runner
- [x] 3.3 Run tests and confirm they pass — full workspace `cargo test` green (227 unit + integration incl. header-drift); wasm `__test__` 7/7 on both variants; wasm conformance 33/34 on both variants; native clippy `-D warnings` clean; fmt clean
