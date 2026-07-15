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
  reopening the same path in-process.
- TypeScript definitions ship in the package (`tsc --strict`-clean, NODE-003).

## Building locally

```bash
npm install
npm run build:debug      # napi build → veclite.<platform>.node + index.{js,d.ts}
node --test __test__/    # behavioral tests
```

`npm run build` produces the release artifact. Cross-platform prebuilds, the
`@veclite/*` platform packages, Bun/Deno CI, and the shared conformance corpus
run are tracked in `phase4i_node-prebuilds-conformance`.

## License

Apache-2.0.
