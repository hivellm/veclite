// Node binding behavioral tests (SPEC-010). Run with `node --test`.
import assert from 'node:assert/strict';
import test from 'node:test';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { rmSync } from 'node:fs';

import { execFileSync } from 'node:child_process';

import pkg from '../veclite.js';
const { open, openSync, memory, chunk, VecLiteError } = pkg;

// file:// URL to the package entry, importable from a child ESM process.
const PKG_URL = new URL('../veclite.js', import.meta.url).href;

const tmp = (n) => join(tmpdir(), `veclite-node-${process.pid}-${n}.veclite`);
const clean = (p) => {
  for (const f of [p, `${p}-wal`]) {
    try {
      rmSync(f);
    } catch {
      /* ignore */
    }
  }
};

test('quickstart: memory db, create, upsert, search (async + sync)', async () => {
  const db = memory();
  const docs = await db.createCollection('docs', {
    dimension: 3,
    metric: 'euclidean',
    quantizationBits: 0,
  });
  await docs.upsert('a', new Float32Array([1, 0, 0]), { lang: 'en' });
  await docs.upsert('b', new Float32Array([0, 1, 0]));
  assert.equal(docs.len(), 2);

  const hits = await docs.search(new Float32Array([0.9, 0.1, 0]), { limit: 1 });
  assert.equal(hits[0].id, 'a');
  assert.deepEqual(hits[0].payload, { lang: 'en' });

  // Sync twin returns identical ids/scores.
  const hitsSync = docs.searchSync(new Float32Array([0.9, 0.1, 0]), { limit: 1 });
  assert.equal(hitsSync[0].id, hits[0].id);
  assert.equal(hitsSync[0].score, hits[0].score);
});

test('Float32Array crosses in and out; hit vectors are Float32Array', async () => {
  const db = memory();
  const c = await db.createCollection('v', { dimension: 4, metric: 'euclidean', quantizationBits: 0 });
  await c.upsertBatch([
    { id: 'p0', vector: new Float32Array([0, 0, 0, 0]) },
    { id: 'p1', vector: new Float32Array([1, 1, 1, 1]) },
  ]);
  const hits = await c.search(new Float32Array([1, 1, 1, 1]), { limit: 1, withVector: true });
  assert.equal(hits[0].id, 'p1');
  assert.ok(hits[0].vector instanceof Float32Array, 'hit vector is a Float32Array');
  assert.deepEqual(Array.from(hits[0].vector), [1, 1, 1, 1]);
});

test('errors: VecLiteError with a stable code, async and sync twins identical', async () => {
  const db = memory();
  const c = await db.createCollection('e', { dimension: 3, quantizationBits: 0 });

  // Dimension mismatch — same code + message from the sync and async forms.
  const sync = (() => {
    try {
      c.upsertSync('x', new Float32Array([1, 2]));
    } catch (err) {
      return err;
    }
  })();
  const asyncErr = await c.upsert('x', new Float32Array([1, 2])).then(
    () => null,
    (err) => err,
  );
  assert.ok(sync instanceof VecLiteError);
  assert.ok(asyncErr instanceof VecLiteError);
  assert.equal(sync.code, 'DIMENSION_MISMATCH');
  assert.equal(asyncErr.code, 'DIMENSION_MISMATCH');
  assert.equal(sync.message, asyncErr.message);
  assert.match(sync.message, /expected 3/);

  // Missing collection → COLLECTION_NOT_FOUND.
  const nf = (() => {
    try {
      db.collection('ghost');
    } catch (err) {
      return err;
    }
  })();
  assert.equal(nf.code, 'COLLECTION_NOT_FOUND');

  // Unknown provider → UNSUPPORTED_PROVIDER (surfaced via createCollection).
  const up = await db.createCollection('t', { dimension: 8, autoEmbed: 'bm52' }).then(
    () => null,
    (err) => err,
  );
  assert.equal(up.code, 'UNSUPPORTED_PROVIDER');
});

test('event loop stays live during a bulk async index (NODE-011)', async () => {
  const db = memory();
  const c = await db.createCollection('bulk', { dimension: 16, metric: 'euclidean', quantizationBits: 0 });

  // A concurrent timer must keep firing while a big async upsert runs off the
  // JS thread. Record inter-tick gaps; none should stall near the whole batch.
  const gaps = [];
  let last = performance.now();
  const timer = setInterval(() => {
    const now = performance.now();
    gaps.push(now - last);
    last = now;
  }, 5);

  const points = Array.from({ length: 4000 }, (_, i) => ({
    id: `k${i}`,
    vector: new Float32Array(16).fill(i % 97),
  }));
  await c.upsertBatch(points);
  // let a few more ticks land, then stop
  await new Promise((r) => setTimeout(r, 40));
  clearInterval(timer);

  assert.ok(c.len() === 4000);
  assert.ok(gaps.length >= 3, `timer should have ticked during the work (got ${gaps.length})`);
  const maxGap = Math.max(...gaps);
  assert.ok(maxGap < 250, `event loop stalled ${maxGap.toFixed(1)}ms — async work blocked it`);
});

test('file db: persistence across a real process boundary', async () => {
  const path = tmp('persist');
  const snap = tmp('snap');
  clean(path);
  clean(snap);
  // Write in a child process so ALL handles (db + collections) drop and the
  // advisory lock releases when it exits — the honest cross-process test
  // (in-process GC timing would otherwise race `Locked`).
  const writer = `
    import pkg from ${JSON.stringify(PKG_URL)};
    const { openSync } = pkg;
    const db = openSync(${JSON.stringify(path)}, { durability: 'full' });
    const c = db.createCollectionSync('docs', { dimension: 2, metric: 'euclidean', quantizationBits: 0 });
    c.upsertBatchSync([
      { id: 'a', vector: new Float32Array([0, 0]), payload: { n: 1 } },
      { id: 'b', vector: new Float32Array([9, 9]) },
    ]);
    await db.snapshot(${JSON.stringify(snap)});
    await db.close();
  `;
  execFileSync(process.execPath, ['--input-type=module', '-e', writer], { stdio: 'pipe' });

  // Reopen in this process: the lock is free, the data is durable.
  const db = openSync(path);
  const c = db.collection('docs');
  assert.equal(c.len(), 2);
  const got = c.get('a');
  assert.deepEqual(Array.from(got.vector), [0, 0]);
  assert.deepEqual(got.payload, { n: 1 });
  await db.vacuum();
  const page = c.scroll({ limit: 10 });
  assert.equal(page.points.length, 2);
  await db.close();
  clean(path);
  clean(snap);
});

test('close() releases the handle: later ops reject with CLOSED (NODE-013)', async () => {
  const path = tmp('closed');
  clean(path);
  const db = await open(path);
  await db.createCollection('c', { dimension: 2, quantizationBits: 0 });
  await db.close();
  await db.close(); // idempotent
  const err = (() => {
    try {
      db.collection('c');
    } catch (e) {
      return e;
    }
  })();
  assert.ok(err instanceof VecLiteError);
  assert.equal(err.code, 'CLOSED');
  clean(path);
});

test('filters, hybrid, and auto-embed text lane', async () => {
  const db = memory();
  const c = await db.createCollection('f', {
    dimension: 2,
    metric: 'euclidean',
    quantizationBits: 0,
    payloadIndexes: [['lang', 'keyword']],
  });
  await c.upsertBatch([
    { id: 'en1', vector: new Float32Array([0, 0]), payload: { lang: 'en' } },
    { id: 'pt1', vector: new Float32Array([1, 0]), payload: { lang: 'pt' } },
    { id: 'en2', vector: new Float32Array([2, 0]), payload: { lang: 'en' } },
  ]);
  const en = await c.search(new Float32Array([0, 0]), {
    limit: 10,
    filter: { must: [{ key: 'lang', match: { value: 'en' } }] },
  });
  assert.deepEqual(en.map((h) => h.id).sort(), ['en1', 'en2']);

  // Hybrid: dense-only degenerates to plain search.
  const hy = c.hybridSearch({ dense: new Float32Array([1, 0]), limit: 1 });
  assert.equal(hy[0].id, 'pt1');

  // Auto-embed text lane.
  const t = await db.createCollection('t', { dimension: 64, autoEmbed: 'bm25' });
  await t.upsertText('cats', 'cats are small furry animals that meow');
  await t.upsertText('cars', 'cars are fast vehicles with engines');
  const st = await t.searchText('furry animals that meow', { limit: 1 });
  assert.equal(st[0].id, 'cats');
  // refit rebuilds the vocabulary without error.
  t.refit();
  assert.equal((await t.searchText('furry animals that meow', { limit: 1 }))[0].id, 'cats');
});

test('sparse-lane hybrid: per-point sparse vectors set the fused ranking', () => {
  const db = memory();
  const c = db.createCollectionSync('h', { dimension: 1, metric: 'euclidean', quantizationBits: 0 });
  c.upsertSync('x', new Float32Array([1]), null, { indices: [0], values: [1] });
  c.upsertSync('y', new Float32Array([2]), null, { indices: [0], values: [2] });
  c.upsertSync('z', new Float32Array([3]), null, { indices: [0], values: [3] });
  const hits = c.hybridSearch({
    dense: new Float32Array([0]),
    sparse: { indices: [0], values: [1] },
    alpha: 0.1,
    rrfK: 60,
    limit: 3,
  });
  assert.deepEqual(hits.map((h) => h.id), ['z', 'y', 'x']);
});

test('createAlias / deleteAlias resolve and remove a target', () => {
  const db = memory();
  db.createCollectionSync('real', { dimension: 2, metric: 'euclidean', quantizationBits: 0 });
  db.collection('real').upsertSync('a', new Float32Array([1, 0]));
  db.createAlias('nick', 'real');
  assert.equal(db.collection('nick').len(), 1);
  db.deleteAlias('nick');
  const err = (() => {
    try {
      db.collection('nick');
    } catch (e) {
      return e;
    }
  })();
  assert.equal(err.code, 'COLLECTION_NOT_FOUND');
});

test('chunk: pure UTF-8-safe splitting', () => {
  assert.deepEqual(chunk('hello world', 100, 0), [{ text: 'hello world', start: 0, end: 11 }]);
  const many = chunk('one two three four five six', 10, 2);
  assert.ok(many.length >= 2);
  assert.ok(many.every((c) => c.start < c.end));
});

// An explicit metric used to be discarded whenever `autoEmbed` was also given:
// the binding reached for CollectionOptions::auto_embed, which takes no metric
// and falls back to the default, persisting the wrong metric silently.
test('metric survives an autoEmbed collection', () => {
  const db = memory();
  const plain = db.createCollectionSync('plain', { dimension: 4, metric: 'euclidean' });
  const auto = db.createCollectionSync('auto', {
    dimension: 4,
    metric: 'euclidean',
    autoEmbed: 'bm25',
  });
  const dflt = db.createCollectionSync('default', { dimension: 4 });

  assert.equal(plain.stats().metric, 'euclidean');
  assert.equal(auto.stats().metric, 'euclidean');
  assert.equal(dflt.stats().metric, 'cosine');
});

// A query with no term in the vocabulary embeds to the zero vector — an
// ordinary "nothing matched", which used to surface as an error naming a metric
// and a vector the caller never supplied.
test('searchText with unknown terms returns no hits', () => {
  const db = memory();
  const docs = db.createCollectionSync('docs', { dimension: 64, autoEmbed: 'bm25' });
  docs.upsertTextSync('a', 'the quick brown fox');

  assert.deepEqual(docs.searchTextSync('zzzz qqqq nonexistentterm', { limit: 3 }), []);
  assert.deepEqual(
    docs.searchTextSync('quick fox', { limit: 3 }).map((h) => h.id),
    ['a'],
  );
});
