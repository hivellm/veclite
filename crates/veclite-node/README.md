# veclite (Node.js)

Node.js binding for [VecLite](https://github.com/hivellm/veclite) — an embedded,
single-file, in-process vector database. Binds the Rust crate directly via
[napi-rs](https://napi.rs) (SPEC-010).

```ts
import { open, memory } from "veclite";

const db = await open("app.veclite", { durability: "normal" });
const docs = await db.createCollection("docs", { dimension: 384, metric: "cosine" });

await docs.upsert("id-1", new Float32Array(vec), { lang: "en" });
const hits = await docs.search(new Float32Array(query), {
  limit: 10,
  filter: { must: [{ key: "lang", match: { value: "en" } }] },
});

await db.close();
```

## Design

- **Async by default** (NODE-011): every heavy operation (`open`,
  `createCollection`, `upsert*`, `search`, `searchText`, `checkpoint`,
  `snapshot`, `vacuum`, `close`) runs off the JS thread on tokio's blocking
  pool, so the event loop never stalls. Each has a `*Sync` twin
  (`openSync`, `searchSync`, `upsertSync`, …) for CLIs and scripts.
- **`Float32Array`** crosses in for `search`/`upsert`/`upsertBatch`; the sync
  twins read it zero-copy. Hit vectors come back as `Float32Array` (NODE-012).
- **Errors** reject/throw a single `VecLiteError extends Error` with a stable
  string `code` (`"DIMENSION_MISMATCH"`, `"COLLECTION_NOT_FOUND"`, …) and the
  exact Rust message (NODE-020).
- **`close()`** flushes and drops the handle so the advisory file lock releases
  immediately; later operations reject with `code: "CLOSED"` (NODE-013). A
  still-referenced `Collection` keeps the file open, so drop collections before
  reopening the same path in-process. A file-backed database garbage-collected
  **without** `close()` emits a `process` warning (`code:
  "VECLITE_HANDLE_LEAK"`) — call `close()` to avoid it.
- **Zero-copy hit vectors** (NODE-012): returned vectors are `Float32Array`
  views backed by an *external* `ArrayBuffer` over the Rust allocation (with a
  finalizer), not a V8-heap copy.
- TypeScript definitions ship in the package (`tsc --strict`-clean, NODE-003).

## Install (prebuilt — no toolchain)

`npm install veclite` pulls a prebuilt native addon: the main package ships the
loader + types, and npm resolves the matching `veclite-<platform>` package
(`veclite-linux-x64-gnu`, `veclite-darwin-arm64`, `veclite-win32-x64-msvc`, …)
by `os`/`cpu` via `optionalDependencies` (NODE-001). Installing never compiles
Rust. Prebuilds cover the FR-66 platform set; runs on **Node 18/20/22** and
**Bun**.

## Runtimes

Node.js ≥ 18 and Bun are both supported and gated by the shared conformance
corpus (SPEC-015 §3). `close()` releases the file lock inline (not on a deferred
task) so an immediate same-path reopen works on every host.

## ONNX embedders

The base addon excludes the ONNX/`fastembed:*` provider family to stay small
(EMB-040). Those providers ship as a separate optional `@veclite/onnx` addon,
this binding built `napi build --features onnx` (pulling ONNX Runtime), which
depends on the exact-version base `veclite` (REL-021). Without it, `fastembed:*`
providers report `UNSUPPORTED_PROVIDER`; an ONNX-created file still opens and
serves vector operations — only text operations fail (EMB-023).

## Building locally

```bash
npm install
npm run build:debug            # napi build → veclite.<platform>.node + index.{js,d.ts}
npm test                       # behavioral + leak/zero-copy tests (node --test)
bash clean_install_e2e.sh      # pack + install prebuild in a fresh project, run quickstart
```

`npm run build` produces the release artifact. `npx napi create-npm-dir -t .`
regenerates the `npm/<platform>/` package templates from `napi.triples`.

## License

Apache-2.0.
