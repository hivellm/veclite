# Proposal: phase4c_node-binding

## Why
DAG T4.4: Node is the second priority ecosystem. The binding must never block the event loop (async-by-default over the libuv threadpool) while offering sync twins for CLIs, with Float32Array zero-copy (FR-62).

## What Changes
- crates/veclite-node: napi-rs direct binding, Node >= 18, per-platform @veclite/* optionalDependencies prebuilds (NODE-001)
- camelCase API mirroring SPEC-004; options objects with SPEC-004 defaults (NODE-010)
- Async via napi AsyncTask for every op; *Sync twins (NODE-011)
- Float32Array zero-copy in/out with external-buffer finalizers (NODE-012)
- Handle lifecycle: idempotent close, pending-op settlement, leak warnings (NODE-013)
- VecLiteError extends Error with code strings mirroring FFI constants (NODE-020)
- TypeScript definitions generated and shipped (NODE-003)

## Impact
- Affected specs: SPEC-010 (all)
- Affected code: crates/veclite-node/ (new), npm package scaffolding
- Breaking change: NO
- User benefit: npm install veclite → quickstart with a never-blocked event loop
