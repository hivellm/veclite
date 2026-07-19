// @hivehub/veclite-wasm quickstart (SPEC-012). Client-side vector search with
// no server and no backend: the pure-Rust engine compiled to wasm, plus the
// portable `.veclite` image codec. The distinctive move is the last step — an
// image serialized here in JS opens byte-for-byte in native VecLite (WASM-010).
//
// Runs under Node against the built package (`js/index.js` + the wasm artifact
// from `bash build-pkg.sh`). `cargo xtask docs` builds the package if
// `wasm-pack` is available, else skips this quickstart. Exits non-zero on any
// surprise.

import assert from 'node:assert/strict';
import { memory, deserialize, ready } from '../js/index.js';

// Which SIMD variant loaded (simd128 where the browser supports it, else a
// scalar fallback) — same engine, same results.
console.log(`veclite-wasm variant: ${await ready()}`);

// An in-memory database — no file, no server (the browser has neither).
const db = await memory();

// BYO-vector collection, cosine metric.
const docs = await db.createCollection('docs', { dimension: 3, metric: 'cosine' });
await docs.upsert('a', Float32Array.from([1, 0, 0]), { lang: 'en' });
await docs.upsert('b', Float32Array.from([0, 1, 0]), { lang: 'fr' });
await docs.upsert('c', Float32Array.from([0.9, 0.1, 0]), { lang: 'en' });

// k-NN search with a payload filter (SPEC-006).
const hits = await docs.search(Float32Array.from([1, 0, 0]), {
  limit: 2,
  filter: { must: [{ key: 'lang', match: { value: 'en' } }] },
});
assert.deepEqual(hits.map((h) => h.id), ['a', 'c']);

// Auto-embed (BM25): text in, ranked ids out — offline, no model download.
const notes = await db.createCollection('notes', { dimension: 128, autoEmbed: 'bm25' });
await notes.upsertText('n1', 'the quick brown fox');
await notes.upsertText('n2', 'a lazy sleeping dog');
assert.ok((await notes.searchText('quick fox', { limit: 2 })).length >= 1);

// The interchange contract (WASM-010): serialize to a `.veclite` v1 image and
// reopen it. This same byte stream opens in native VecLite — write in the
// browser, read on the server.
const image = await db.serialize();
assert.deepEqual(Array.from(image.slice(0, 4)), [...'VECL'].map((ch) => ch.charCodeAt(0)));
const reopened = await deserialize(image);
const back = await reopened.collection('docs');
assert.equal(await back.len(), 3);

console.log(`veclite-wasm: quickstart OK (${hits.map((h) => h.id).join(', ')}, image ${image.length} bytes)`);
