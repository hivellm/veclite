# VecLite

**Embedded, single-file, in-process vector database.**

VecLite is the in-process distribution of the
[Vectorizer](https://github.com/hivellm/vectorizer) engine: HNSW search,
quantization, hybrid dense+sparse retrieval, and payload filtering — as a
library you link, not a server you run. It follows the embedded-database
philosophy popularized by SQLite: one linked library, one file, zero
configuration.

```python
import veclite

db = veclite.Database.open("app.veclite")
docs = db.create_collection("docs", 512, embedding_provider="bm25")
docs.upsert_text("readme", open("README.md").read())
hits = docs.search_text("how do I configure logging", limit=5)
```

No server. No ports. No configuration. One file.

## What's in this book

- **Quickstarts** — a runnable, self-contained first program in each of the six
  supported languages (Rust, Python, Node.js, Go, C#, and WASM in the browser).
  Every quickstart here is the *executed sample*: `cargo xtask docs` runs each
  one, so a sample can never drift from the API (REL-041).
- **Guides** — the graduation path to the Vectorizer server and back, and the
  `veclite` command-line tool.
- **Reference** — sizing and limits, the frozen `.veclite` v1 storage format,
  the versioning and format-stability policy, and the benchmark report.

## When to reach for VecLite

VecLite is the right tool when you want vector search **inside your process**:
a CLI, a desktop or mobile app, an agent, an edge deployment, a test fixture, or
a browser tab. It is not a server — there is no network, no auth, no
replication. When you outgrow one process, the [graduation
guide](guides/graduation.md) moves your data to the Vectorizer server with
scoring intact, and the [reverse guide](guides/reverse-migration.md) brings a
slice back offline.

| | Vectorizer | VecLite |
|---|---|---|
| Model | client-server | embedded library |
| Storage | managed `data/` dir | single `.veclite` file |
| Access | REST / RPC / gRPC / MCP | function calls |
| Scale | replication, Raft, sharding | one process |
| Use | shared production infra | CLIs, apps, agents, edge, tests |

Same math, same defaults, same concepts — start embedded, graduate to the server
when you need the network.

## Status

Pre-1.0, built phase by phase against a normative
[spec set](specs.md). The `.veclite` format v1 is **frozen** — files written
today stay readable across every future 1.x release (see [format
stability](reference/versioning.md)).
