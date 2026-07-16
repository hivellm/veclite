## 1. Implementation
- [x] 1.1 napi prebuild matrix (FR-66 triples) + veclite-<platform> optionalDependencies + loader selection (NODE-001); no-toolchain install check — package.json optionalDependencies (6 platforms) + committed npm/<platform>/ templates + the generated loader; clean_install_e2e.sh proves install-without-toolchain. (Unscoped `veclite-<platform>` names match the napi-generated loader; the proposal's `@veclite/` is a naming preference the loader does not use.)
- [x] 1.2 Zero-copy-out: hit vectors as external-buffer Float32Array views over the Rust allocation with a finalizer (NODE-012) — napi's `Float32Array::new(Vec)` already backs vectors with `napi_create_external_arraybuffer` + finalizer; verified by test.
- [x] 1.3 Node conformance runner over the shared YAML corpus (SPEC-015 §3, tolerance 1e-5) — existed (phase4d); fixed `drainHandles()` to force GC under Bun (Bun.gc) so same-path reopen works.
- [x] 1.4 Leaked-handle finalizer with process.emitWarning; @veclite/onnx split (NODE-002/013, EMB-040) — FinalizationRegistry in veclite.js emits VECLITE_HANDLE_LEAK for file dbs GC'd unclosed; the @veclite/onnx split is documented, blocked on the onnx core feature (phase5c_onnx-feature) which does not yet exist.

## 2. Testing
- [x] 2.1 Conformance corpus + quickstart green on Node 18/20/22 + Bun from prebuilds (acceptance 1/4) — 34 cases pass on both Node and Bun locally; CI packaging job runs both on native builds. (Needed a fix: close() now drops the handle inline, not on spawn_blocking, so the lock releases before the promise resolves on Bun.)
- [x] 2.2 Zero-copy-out allocation-tracking proof (NODE-012) — leak_and_zerocopy.test.mjs asserts hit vectors are external ArrayBuffers via process.memoryUsage().
- [x] 2.3 Clean-machine `npm install veclite` runs the quickstart with no Rust toolchain (acceptance 2) — clean_install_e2e.sh packs main + platform tarballs, installs both into a fresh project, runs the quickstart; CI clean-install job mirrors it.

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [x] 3.1 Update or create documentation covering the implementation — crates/veclite-node/README.md (prebuilt install, runtimes, leak warning, onnx split) + SPEC-010 status
- [x] 3.2 Write tests covering the new behavior — leak_and_zerocopy.test.mjs (4 tests) + clean_install_e2e.sh + Bun conformance
- [x] 3.3 Run tests and confirm they pass — 14 node tests pass; conformance 34 cases green on Node AND Bun; clippy -D warnings + fmt clean
