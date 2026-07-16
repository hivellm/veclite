# Proposal: phase4i_node-prebuilds-conformance

## Why

phase4c delivered the Node binding core (crates/veclite-node): the full
Database/Collection surface (async + Sync twins), Float32Array in/out,
VecLiteError with code strings, close()/Closed lifecycle, .d.ts + tsc --strict,
and 7 behavioral tests green locally (quickstart, error mapping, event-loop
liveness, cross-process persistence). What remains is packaging, the platform
matrix, and cross-runtime/conformance proof — none of which affect the binding
logic, and all of which need CI (currently disabled — Actions quota):

1. Prebuild matrix (NODE-001): build veclite.<triple>.node for the FR-66
   platform set and publish the @veclite/<platform> optionalDependencies, with a
   loader that selects them; `npm install veclite` must never compile Rust.
2. Cross-runtime CI (acceptance 1/4): the conformance corpus + quickstart on
   Node 18/20/22 and Bun, from the built prebuilds on clean machines.
3. Shared conformance corpus runner (SPEC-015 §3): the Node runner over the
   YAML corpus, tolerance 1e-5, shared with the Rust/Python runners.
4. Zero-copy-out optimization (NODE-012): return hit vectors as external-buffer
   Float32Array views over the Rust allocation with a finalizer, instead of the
   current one-copy `Float32Array::new`.
5. onnx (@veclite/onnx) + the emitWarning-on-leaked-handle finalizer polish.

## What Changes

- napi build matrix + npm/ platform package scaffolding + loader.
- A Node conformance runner + the corpus wiring.
- External-buffer hit vectors with finalizers.

## Impact

- Affected specs: SPEC-010 NODE-001/002/012, acceptance 1/4; SPEC-015 §3; SPEC-016.
- Affected code: crates/veclite-node (build config, loader, zero-copy path), CI.
- Breaking change: NO (packaging + an internal zero-copy optimization).
- User benefit: `npm install` prebuilds on every platform, Bun support, a pinned
  cross-repo conformance guarantee, and lower-allocation hit vectors.
