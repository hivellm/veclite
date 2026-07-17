# @veclite/wasm

WebAssembly build of [VecLite](https://github.com/hivellm/veclite) — an embedded,
single-file vector database — for **browsers, Deno, Bun, and edge runtimes**
(Cloudflare Workers-class). Client-side semantic search over small/medium corpora
(guideline ≤ ~500k vectors), offline apps, and extensions — no server, no native
addon (SPEC-012).

```js
import { open, memory, deserialize } from '@veclite/wasm';

// In-memory, or OPFS-persistent in a browser:
const db = await open({ opfs: 'app.veclite', autosave: { afterWrites: 100 } });

const docs = await db.createCollection('docs', { dimension: 384, metric: 'cosine' });
await docs.upsertBatch(points);                         // { id, vector, payload?, sparse? }[]
const hits = await docs.search(query, { limit: 10, filter });

const bytes = await db.serialize();                      // a valid .veclite file image
await db.save();                                         // atomic OPFS checkpoint
await db.close();
```

Every method is a `Promise` (WASM-020), even though execution inside the module
is synchronous.

## Design

- **Build profile** (WASM-001): compiled from the Rust crate's `wasm32` profile —
  the in-memory engine (collection registry, CRUD, brute-force exact search,
  quantization) plus the portable `.veclite` v1 image codec. No filesystem, mmap,
  file locks, threads, or ONNX (all target-gated off wasm32 in the core).
- **SIMD + fallback** (WASM-002): two binaries ship — a `simd128` build and a
  plain fallback. The loader feature-detects `WebAssembly` SIMD and picks the
  right one automatically; force a variant with `VECLITE_WASM_VARIANT=simd128|fallback`
  or `globalThis.VECLITE_WASM_VARIANT` (used by the test matrix).
- **Storage backends**:

  | Backend | Availability | Semantics |
  |---|---|---|
  | In-memory | always | `memory()`; lost on page unload |
  | Bytes import/export | always | `db.serialize()` / `deserialize(bytes)` — a valid `.veclite` v1 file image |
  | OPFS | browsers/Deno with OPFS | persistent; explicit `save()` + optional autosave |

- **Interchange** (WASM-010): `serialize()` produces a byte-for-byte valid
  `.veclite` v1 file — open it with native VecLite (Rust/Node/Python/Go/C#), and
  a native file's bytes load through `deserialize()`. Same format, no wasm dialect.
- **Errors** (WASM-021): every rejection is an `Error` with a `code` string (the
  SPEC-010 codes, e.g. `COLLECTION_NOT_FOUND`, `DIMENSION_MISMATCH`, `ALREADY_EXISTS`).

## Sizing & durability (WASM-012)

The wasm database operates on a **full in-memory image** — there is no lazy
paging (a synchronous core over an async storage API cannot page without
`SharedArrayBuffer` workers; block-level paging is post-1.0). Two consequences:

- **Size is bounded by memory.** Budget roughly the raw vector bytes plus
  overhead: `N × dimension × 4` bytes for `float32` vectors, plus payloads and a
  brute-force scan structure. The positioning guideline is **≤ ~500k vectors**;
  a 100k × 384-dim corpus searches in a few milliseconds. Beyond that, prefer the
  native server or the mmap-backed native library.
- **Durability is explicit.** Writes live in memory until `save()` (or autosave)
  serializes the whole image to OPFS. **There is no WAL in wasm** — a crash
  between saves loses the writes since the last save. Tune `autosave.afterWrites`
  / `autosave.intervalMs` to bound that window. `vacuum()` is a no-op:
  compaction happens as part of `serialize`/`save`.

Search on wasm is an **exact brute-force scan** (no HNSW — `hnsw_rs` is
native-only, ADR-0002); results are identical to the native engine, and payload
filters evaluate by scan.

## Building

`./build-pkg.sh` builds both wasm binaries, generates the shared wasm-bindgen
glue, and writes `pkg/`. It needs `wasm-bindgen-cli` version-matched to the
`wasm-bindgen` crate in `Cargo.toml`, plus the `wasm32-unknown-unknown` target;
`wasm-opt` (binaryen) is used for the final size squeeze if present.

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <match Cargo.toml>
./build-pkg.sh --check      # build pkg/ and fail if any wasm exceeds the 3 MB gzip budget (WASM-030)
```

The gzipped binaries are well under the **3 MB** budget (WASM-030) — about
185 KB each. `pkg/` is committed (like the Node binding's prebuilds) so the
package installs and the tests run without a Rust+wasm toolchain.

## Testing

```bash
node --test __test__/veclite.test.mjs                    # facade + OPFS (mock) + interchange
VECLITE_WASM_VARIANT=fallback node --test __test__/veclite.test.mjs
node ../../tests/conformance/runners/wasm/run.mjs        # shared corpus (gate G5)
```

The conformance runner drives the shared corpus (SPEC-015 §3) through the same
observations as the Rust/Node/Python/Go/C# runners; the wasm binding runs the
memory-mode subset (file-lock/mmap/durability cases are skipped, SPEC-012 §5),
with `reopen` mapped to a `serialize`→`deserialize` round-trip.

## License

Apache-2.0.
