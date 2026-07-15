## 1. Implementation
- [ ] 1.1 napi prebuild matrix (FR-66 triples) + @veclite/<platform> optionalDependencies + loader selection (NODE-001); no-toolchain install check
- [ ] 1.2 Zero-copy-out: hit vectors as external-buffer Float32Array views over the Rust allocation with a finalizer (NODE-012)
- [ ] 1.3 Node conformance runner over the shared YAML corpus (SPEC-015 §3, tolerance 1e-5)
- [ ] 1.4 Leaked-handle finalizer with process.emitWarning; @veclite/onnx split (NODE-002/013, EMB-040)

## 2. Testing
- [ ] 2.1 Conformance corpus + quickstart green on Node 18/20/22 + Bun from prebuilds (acceptance 1/4)
- [ ] 2.2 Zero-copy-out allocation-tracking proof (NODE-012)
- [ ] 2.3 Clean-machine `npm install veclite` runs the quickstart with no Rust toolchain (acceptance 2)

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
