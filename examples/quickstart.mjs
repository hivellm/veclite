// VecLite Node.js quickstart (SPEC-010). Doubles as the clean-machine install
// proof (REL-020): it imports the installed `veclite` npm package — a prebuilt
// native addon, no Rust toolchain — and exercises the core flow. Exits non-zero
// on any surprise.
//
// Run: `npm install veclite && node examples/quickstart.mjs`

import { strict as assert } from 'node:assert';
import { tmpdir } from 'node:os';
import { mkdtempSync } from 'node:fs';
import { join } from 'node:path';
// veclite ships as CommonJS; default-import then destructure (works from ESM).
import veclite from 'veclite';
const { openSync } = veclite;

// A durable single-file database — no server, no config (FR-01/02).
const path = join(mkdtempSync(join(tmpdir(), 'veclite-')), 'app.veclite');
const db = openSync(path);

// BYO-vector collection, cosine metric.
const docs = db.createCollectionSync('docs', { dimension: 3, metric: 'cosine', quantizationBits: 0 });
docs.upsertSync('a', Float32Array.from([1, 0, 0]), { lang: 'en' });
docs.upsertSync('b', Float32Array.from([0, 1, 0]), { lang: 'fr' });
docs.upsertSync('c', Float32Array.from([0.9, 0.1, 0]), { lang: 'en' });

// k-NN search with a payload filter (SPEC-006).
const hits = docs.searchSync(Float32Array.from([1, 0, 0]), {
  limit: 2,
  filter: { must: [{ key: 'lang', match: { value: 'en' } }] },
});
const ids = hits.map((h) => h.id);
assert.deepEqual(ids, ['a', 'c']);

// An auto-embed (BM25) collection: text in, ranked ids out (SPEC-005).
const notes = db.createCollectionSync('notes', { dimension: 128, autoEmbed: 'bm25' });
notes.upsertTextSync('n1', 'the quick brown fox');
notes.upsertTextSync('n2', 'a lazy sleeping dog');
assert.ok(notes.searchTextSync('quick fox', { limit: 2 }).length >= 1);

await db.close();
console.log(`veclite: quickstart OK (${ids.join(', ')})`);
