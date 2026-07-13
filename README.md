# VecLite

**Embedded, single-file, in-process vector database.**

> Reference point: VecLite follows the embedded-database philosophy popularized by SQLite — one linked library, one file, zero configuration.

VecLite is the in-process, single-file distribution of the [Vectorizer](https://github.com/hivellm/vectorizer) engine: HNSW search, quantization, hybrid dense+sparse retrieval, and payload filtering — as a library you link, not a server you run.

```python
import veclite

db = veclite.open("app.veclite")
docs = db.create_collection("docs", auto_embed="bm25", dimension=512)
docs.upsert_text("readme", open("README.md").read(), {"lang": "en"})
hits = docs.search_text("how do I configure logging", limit=5)
```

No server. No ports. No configuration. One file.

## Status

🚧 **Phase 0 — bootstrap.** The workspace, foundation types (`VecLiteError`, options with server-parity defaults), and CI gates exist; the engine lands phase by phase per the [DAG](docs/DAG.md).

### Development

```bash
cargo check --workspace                                        # diagnostics first
cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features                          # unit + doc tests
cargo build --workspace --target wasm32-unknown-unknown        # wasm32 gate (CORE-004)
```

Task backlog lives in `.rulebook/tasks/` (one task per context cycle, `phase0a…phase6c`); the implementation contract is below.

- **Implementation contract**: [`docs/specs/`](docs/specs/README.md) — [PRD](docs/PRD.md) (requirements & release criteria), [DAG](docs/DAG.md) (task dependency graph), and SPEC-001…016 (normative component specs).
- **Design rationale**: [`docs/vectorizer-lite/`](docs/vectorizer-lite/README.md), the original planning set:

1. [Vision and scope](docs/vectorizer-lite/01-vision-and-scope.md) — what's in, what's out, feature matrix vs Vectorizer
2. [Architecture](docs/vectorizer-lite/02-architecture.md) — crate layout, reuse of `vectorizer-core`, design decisions
3. [Core API](docs/vectorizer-lite/03-core-api.md) — Rust API (source of truth for all bindings)
4. [Storage format](docs/vectorizer-lite/04-storage-format.md) — single-file `.veclite`, WAL, crash safety
5. [Embeddings](docs/vectorizer-lite/05-embeddings.md) — BYO vectors, built-in BM25/TF-IDF, optional ONNX
6. [SDK bindings](docs/vectorizer-lite/06-sdk-bindings.md) — Python, Node.js, Go, C#, WASM (native, not network)
7. [Vectorizer compatibility](docs/vectorizer-lite/07-vectorizer-compatibility.md) — graduation path to the server
8. [Roadmap](docs/vectorizer-lite/08-roadmap.md) — phases to 1.0

## Relationship to Vectorizer

| | Vectorizer | VecLite |
|---|---|---|
| Model | client-server | embedded library |
| Storage | managed `data/` dir | single `.veclite` file |
| Access | REST / RPC / gRPC / MCP | function calls |
| Scale | replication, Raft, sharding | one process |
| Use | shared production infra | CLIs, apps, agents, edge, tests |

Same math, same defaults, same concepts — start embedded, graduate to the server when you need the network.

## License

Apache-2.0 (matching Vectorizer).
