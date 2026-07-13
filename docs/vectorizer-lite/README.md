# VecLite — Embedded Vectorizer ("SQLite for vector search")

> Planning documentation for the VecLite spinoff. Source project: [hivellm/vectorizer](https://github.com/hivellm/vectorizer) (v3.5.0, Rust edition 2024).

## What is VecLite

VecLite is an **embedded, in-process vector database library** extracted from Vectorizer, following the same philosophy that SQLite has toward client-server databases:

| | Vectorizer (server) | VecLite (embedded) |
|---|---|---|
| Deployment | Standalone server process (REST/RPC/gRPC/MCP on ports 15002/15503) | Library linked into the host application |
| Access | Network SDKs (TCP + MessagePack, HTTP fallback) | Direct function calls (native bindings / FFI) |
| Storage | `data/` directory managed by the server | Single `.veclite` file owned by the application |
| Auth / TLS / RBAC | Yes | None (process boundary is the security boundary) |
| Replication / cluster / sharding | Yes (Raft, master-replica) | None |
| Concurrency model | Multi-client, multi-tenant | Single process, multi-thread |
| Runtime | Tokio async, dashboard, monitoring | Sync core, zero background services by default |
| Target use | Production services, shared infrastructure | CLIs, desktop/mobile apps, edge, tests, notebooks, agents |

**One-line pitch**: `pip install veclite` / `npm install veclite` / `cargo add veclite` and you have semantic search over a single file — no server, no Docker, no ports, no configuration.

## Why

1. **Friction**: today the smallest possible Vectorizer deployment is a server binary + config + port + client SDK. Many consumers (CLI tools, agent runtimes, test suites, desktop apps) need exactly one process and one file.
2. **Reach**: an embedded library runs where a server can't — serverless functions, CI, WASM/edge, mobile.
3. **Funnel**: VecLite is the on-ramp; when an application outgrows a single process it "graduates" to Vectorizer server with a documented migration path (same concepts, compatible data).

## Document index

| Doc | Content |
|---|---|
| [01-vision-and-scope.md](01-vision-and-scope.md) | Goals, non-goals, feature matrix (what's in, what's out, why) |
| [02-architecture.md](02-architecture.md) | Crate layout, module design, code reuse strategy vs the Vectorizer workspace |
| [03-core-api.md](03-core-api.md) | Rust core API design (`VecLite`, `Collection`, options, errors) |
| [04-storage-format.md](04-storage-format.md) | Single-file `.veclite` format, WAL sidecar, mmap, durability model |
| [05-embeddings.md](05-embeddings.md) | Embedding strategy: pure-Rust built-ins, optional ONNX, bring-your-own-vectors |
| [06-sdk-bindings.md](06-sdk-bindings.md) | Native bindings: Python, Node.js, Go, C#, WASM — FFI architecture |
| [07-vectorizer-compatibility.md](07-vectorizer-compatibility.md) | Interop with Vectorizer server: shared crates, data import/export, graduation path |
| [08-roadmap.md](08-roadmap.md) | Phased roadmap, milestones, versioning policy |

## Guiding principles (inherited from SQLite's playbook)

1. **Zero-configuration** — no config file required; `VecLite::open("path")` with sane defaults is a complete setup.
2. **Single file** — the entire database (collections, vectors, indexes, payloads) lives in one file the user can copy, email, or commit to object storage.
3. **In-process** — no IPC, no serialization boundary on the hot path; a search is a function call.
4. **Small dependency surface** — the default build is pure Rust: no ONNX runtime, no tokio, no OpenSSL, no protobuf.
5. **Reliable format** — versioned on-disk format with checksums; older library versions refuse newer formats gracefully; newer versions always read older formats.
6. **The full engine, not a toy** — HNSW, quantization, hybrid search, and payload filtering are the same algorithms as the server, extracted, not reimplemented.

## Status

Planning phase. No code exists yet. This documentation is the design contract for the initial implementation.
