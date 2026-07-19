# Specifications (normative)

VecLite is built against a normative spec set. These are the source of truth for
behavior; this book is the user-facing companion. Each spec carries a
Status/Phase/PRD header table.

- [Overview & PRD](../PRD.md) — requirements and release criteria
- [DAG](../DAG.md) — task dependency graph and gates
- [Spec index](../specs/README.md)

## Component specs

| Spec | Area |
|---|---|
| [SPEC-001](../specs/SPEC-001-core-engine.md) | Core engine (registry, CRUD, HNSW) |
| [SPEC-002](../specs/SPEC-002-storage-format.md) | `.veclite` v1 storage format (frozen) |
| [SPEC-003](../specs/SPEC-003-wal-durability.md) | WAL & durability |
| [SPEC-004](../specs/SPEC-004-rust-api.md) | Rust API (source of truth for bindings) |
| [SPEC-005](../specs/SPEC-005-embeddings.md) | Embeddings (auto-embed, providers) |
| [SPEC-006](../specs/SPEC-006-payload-filters.md) | Payload filters & indexes |
| [SPEC-007](../specs/SPEC-007-hybrid-search.md) | Hybrid dense+sparse search |
| [SPEC-008](../specs/SPEC-008-ffi-c-abi.md) | C ABI |
| [SPEC-009](../specs/SPEC-009-binding-python.md) | Python binding |
| [SPEC-010](../specs/SPEC-010-binding-node.md) | Node.js binding |
| [SPEC-011](../specs/SPEC-011-bindings-go-csharp.md) | Go & C# bindings |
| [SPEC-012](../specs/SPEC-012-binding-wasm.md) | WASM binding |
| [SPEC-013](../specs/SPEC-013-vecdb-interop.md) | `.vecdb` interop (graduation) |
| [SPEC-014](../specs/SPEC-014-cli.md) | `veclite` CLI |
| [SPEC-015](../specs/SPEC-015-testing-conformance.md) | Testing, conformance, benchmarks |
| [SPEC-016](../specs/SPEC-016-packaging-release.md) | Packaging, CI, release |

## Design rationale

The original planning set lives under
[`docs/vectorizer-lite/`](../vectorizer-lite/README.md): vision & scope,
architecture, core API, storage format, embeddings, SDK bindings, Vectorizer
compatibility, and the roadmap.
