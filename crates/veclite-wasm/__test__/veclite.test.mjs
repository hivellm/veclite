// @veclite/wasm test suite (SPEC-012). Runs under `node --test`.
//
// Covers the JS facade end to end: CRUD + search, the serialize/deserialize
// interchange round-trip (WASM-010), and the OPFS backend — atomic save,
// reopen-after-reload, and autosave (WASM-011) — against an in-memory OPFS mock
// (real OPFS is browser/Deno-only). The cross-implementation byte contract
// (native file ↔ wasm) is proven by the native Rust tests in
// `crates/veclite/tests/image_interchange.rs`, which exercise the identical
// serialize/deserialize code this package compiles to wasm.

import { test } from 'node:test';
import assert from 'node:assert/strict';

import { memory, open, deserialize, ready, loadedVariant } from '../js/index.js';

// ── in-memory OPFS mock (navigator.storage.getDirectory) ─────────────────────

function installOpfsMock() {
  const files = new Map(); // name -> Uint8Array
  const fileHandle = (name) => ({
    async createWritable() {
      let buf = new Uint8Array(0);
      return {
        async write(bytes) {
          buf = bytes instanceof Uint8Array ? bytes.slice() : new Uint8Array(bytes);
        },
        async close() {
          files.set(name, buf);
        },
      };
    },
    async getFile() {
      const bytes = files.get(name) ?? new Uint8Array(0);
      return { async arrayBuffer() { return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength); } };
    },
    // Atomic rename used by opfsWriteAtomic.
    async move(dest) {
      files.set(dest, files.get(name));
      files.delete(name);
    },
  });
  const dir = {
    async getFileHandle(name, opts) {
      if (!files.has(name) && !(opts && opts.create)) {
        throw new Error('not found');
      }
      if (opts && opts.create && !files.has(name)) files.set(name, new Uint8Array(0));
      return fileHandle(name);
    },
    async removeEntry(name) {
      files.delete(name);
    },
  };
  // Node ships a read-only built-in `navigator`; override it with a configurable
  // property and restore the original on teardown.
  const original = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  Object.defineProperty(globalThis, 'navigator', {
    value: { storage: { getDirectory: async () => dir } },
    configurable: true,
    writable: true,
  });
  const uninstall = () => {
    if (original) Object.defineProperty(globalThis, 'navigator', original);
    else delete globalThis.navigator;
  };
  return { files, uninstall };
}

test('loads a wasm variant', async () => {
  const v = await ready();
  assert.ok(v === 'simd128' || v === 'fallback');
  assert.equal(loadedVariant(), v);
});

test('quickstart: CRUD, search, payload', async () => {
  const db = await memory();
  const c = await db.createCollection('docs', { dimension: 3, metric: 'euclidean' });
  await c.upsert('a', Float32Array.from([1, 0, 0]), { lang: 'en' });
  await c.upsert('b', Float32Array.from([0, 1, 0]));
  assert.equal(await c.len(), 2);

  const hits = await c.search(Float32Array.from([0.9, 0.1, 0]), { limit: 1, withPayload: true });
  assert.equal(hits.length, 1);
  assert.equal(hits[0].id, 'a');
  assert.equal(hits[0].payload.lang, 'en');

  const got = await c.get('a');
  assert.ok(got.vector instanceof Float32Array);
  assert.deepEqual(Array.from(got.vector), [1, 0, 0]);
  assert.equal(await c.get('missing'), null);
  assert.equal(await c.delete('a'), true);
});

test('serialize/deserialize round-trip preserves data and sparse lane (WASM-010)', async () => {
  const db = await memory();
  const c = await db.createCollection('docs', { dimension: 3, metric: 'cosine' });
  await c.upsert('a', Float32Array.from([1, 0, 0]), { lang: 'en' }, { indices: [5], values: [1] });
  await c.upsertBatch([
    { id: 'b', vector: Float32Array.from([0, 1, 0]) },
    { id: 'c', vector: Float32Array.from([0, 0, 1]), payload: { lang: 'pt' } },
  ]);
  await db.createAlias('documents', 'docs');

  const bytes = await db.serialize();
  // A valid .veclite v1 image: the `VECL` magic leads.
  assert.deepEqual(Array.from(bytes.slice(0, 4)), [...'VECL'].map((c) => c.charCodeAt(0)));

  const back = await deserialize(bytes);
  const via = await back.collection('documents'); // alias survived
  assert.equal(await via.len(), 3);
  const a = await via.get('a');
  assert.equal(a.payload.lang, 'en');

  const before = (await c.search(Float32Array.from([0.2, 0.9, 0.1]), { limit: 3 })).map((h) => h.id);
  const after = (await via.search(Float32Array.from([0.2, 0.9, 0.1]), { limit: 3 })).map((h) => h.id);
  assert.deepEqual(after, before);

  const hy = await via.hybridSearch({ sparse: { indices: [5], values: [1] }, limit: 3 });
  assert.ok(hy.some((h) => h.id === 'a'), 'sparse lane lost across round-trip');
});

test('OPFS: save + reload restores the database (WASM-011)', async () => {
  const mock = installOpfsMock();
  try {
    const db = await open({ opfs: 'app.veclite' });
    const c = await db.createCollection('docs', { dimension: 2, metric: 'euclidean' });
    await c.upsert('a', Float32Array.from([0, 0]), { k: 1 });
    await c.upsert('b', Float32Array.from([1, 1]));
    await db.save();
    // The atomic write left exactly the target file (temp was moved onto it).
    assert.ok(mock.files.has('app.veclite'));
    assert.ok(!mock.files.has('app.veclite.tmp'));
    await db.close();

    // Reopen from the persisted image.
    const db2 = await open({ opfs: 'app.veclite' });
    const c2 = await db2.collection('docs');
    assert.equal(await c2.len(), 2);
    assert.equal((await c2.get('a')).payload.k, 1);
  } finally {
    mock.uninstall();
  }
});

test('OPFS: autosave after N writes (WASM-011)', async () => {
  const mock = installOpfsMock();
  try {
    const db = await open({ opfs: 'notes.veclite', autosave: { afterWrites: 3 } });
    const c = await db.createCollection('n', { dimension: 1, metric: 'euclidean' });
    // createCollection counts as a write (1); two upserts reach the threshold (3).
    await c.upsert('a', Float32Array.from([1]));
    assert.ok(!mock.files.has('notes.veclite'), 'saved too early');
    await c.upsert('b', Float32Array.from([2]));
    assert.ok(mock.files.has('notes.veclite'), 'autosave did not fire at threshold');

    const reopened = await open({ opfs: 'notes.veclite' });
    assert.equal(await (await reopened.collection('n')).len(), 2);
  } finally {
    mock.uninstall();
  }
});

test('save() without OPFS rejects with a code', async () => {
  const db = await memory();
  await assert.rejects(() => db.save(), (e) => e.code === 'INVALID_ARGUMENT');
});

test('errors carry a code (WASM-021)', async () => {
  const db = await memory();
  await db.createCollection('v', { dimension: 3, metric: 'cosine' });
  await assert.rejects(
    () => db.createCollection('v', { dimension: 3 }),
    (e) => e.code === 'ALREADY_EXISTS',
  );
  await assert.rejects(
    () => db.collection('nope'),
    (e) => e.code === 'COLLECTION_NOT_FOUND',
  );
  const c = await db.collection('v');
  await assert.rejects(
    () => c.upsert('x', Float32Array.from([1, 2])),
    (e) => e.code === 'DIMENSION_MISMATCH',
  );
});
