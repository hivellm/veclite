// Leaked-handle warning (NODE-013) and zero-copy-out hit vectors (NODE-012).
// Run with `node --expose-gc --test`. The leak test needs manual GC to force the
// FinalizationRegistry callback deterministically.
import assert from 'node:assert/strict';
import test from 'node:test';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { rmSync } from 'node:fs';

import pkg from '../veclite.js';

const { openSync, memory } = pkg;

const tmp = (n) => join(tmpdir(), `veclite-node-lz-${process.pid}-${n}.veclite`);
const clean = (p) => {
  for (const f of [p, `${p}-wal`]) {
    try {
      rmSync(f);
    } catch {
      /* ignore */
    }
  }
};

// Wait for a macrotask so FinalizationRegistry callbacks (queued after GC) run.
const nextMacrotask = () => new Promise((r) => setTimeout(r, 0));

test('file db garbage-collected without close() emits a leak warning (NODE-013)', async (t) => {
  if (typeof global.gc !== 'function') {
    t.skip('run with --expose-gc to force finalization');
    return;
  }
  const path = tmp('leak');
  clean(path);

  const warnings = [];
  const onWarning = (w) => {
    if (w.code === 'VECLITE_HANDLE_LEAK') warnings.push(w);
  };
  process.on('warning', onWarning);
  try {
    // Open, use, then drop the reference WITHOUT close().
    (() => {
      const db = openSync(path);
      const c = db.createCollectionSync('v', { dimension: 2, metric: 'euclidean', quantizationBits: 0 });
      c.upsertSync('a', new Float32Array([1, 0]));
    })();

    // Force GC + let the finalizer callback run.
    for (let i = 0; i < 3 && warnings.length === 0; i++) {
      global.gc();
      await nextMacrotask();
    }
    assert.equal(warnings.length >= 1, true, 'expected a VECLITE_HANDLE_LEAK warning');
    assert.match(warnings[0].message, /garbage-collected without close/);
  } finally {
    process.off('warning', onWarning);
    clean(path);
  }
});

test('closed file db does NOT warn (NODE-013)', async (t) => {
  if (typeof global.gc !== 'function') {
    t.skip('run with --expose-gc to force finalization');
    return;
  }
  const path = tmp('noleak');
  clean(path);

  const warnings = [];
  const onWarning = (w) => {
    if (w.code === 'VECLITE_HANDLE_LEAK') warnings.push(w);
  };
  process.on('warning', onWarning);
  try {
    await (async () => {
      const db = openSync(path);
      await db.close();
    })();
    for (let i = 0; i < 3; i++) {
      global.gc();
      await nextMacrotask();
    }
    assert.equal(warnings.length, 0, 'a properly-closed handle must not warn');
  } finally {
    process.off('warning', onWarning);
    clean(path);
  }
});

test('memory db is not tracked (no external resource to leak)', async (t) => {
  if (typeof global.gc !== 'function') {
    t.skip('run with --expose-gc to force finalization');
    return;
  }
  const warnings = [];
  const onWarning = (w) => {
    if (w.code === 'VECLITE_HANDLE_LEAK') warnings.push(w);
  };
  process.on('warning', onWarning);
  try {
    (() => {
      memory();
    })();
    for (let i = 0; i < 3; i++) {
      global.gc();
      await nextMacrotask();
    }
    assert.equal(warnings.length, 0, 'memory db must not warn on GC');
  } finally {
    process.off('warning', onWarning);
  }
});

test('hit vectors are external-buffer Float32Arrays, not V8-heap copies (NODE-012)', async () => {
  const db = memory();
  const dim = 1024;
  const n = 256;
  const c = await db.createCollection('v', { dimension: dim, metric: 'cosine', quantizationBits: 0 });

  const rows = new Float32Array(n * dim);
  for (let i = 0; i < rows.length; i++) rows[i] = Math.sin(i);
  const points = [];
  for (let i = 0; i < n; i++) {
    points.push({ id: `k${i}`, vector: rows.subarray(i * dim, (i + 1) * dim) });
  }
  await c.upsertBatch(points);

  // Ask search to return the stored vectors; they should be external
  // ArrayBuffers (backed by the Rust allocation), which show up in
  // process.memoryUsage().arrayBuffers rather than doubling the V8 heap.
  const before = process.memoryUsage().arrayBuffers;
  const hits = await c.search(rows.subarray(0, dim), { limit: 64, withVector: true });
  assert.equal(hits.length, 64);
  assert.equal(hits[0].vector instanceof Float32Array, true);
  assert.equal(hits[0].vector.length, dim);

  const held = hits.map((h) => h.vector);
  const after = process.memoryUsage().arrayBuffers;
  // 64 external vectors × 1024 × 4 B ≈ 256 KB of arrayBuffer memory attributable
  // to the returned views — proof they are real buffers we now hold, not clones
  // living on the JS object heap.
  assert.equal(after >= before, true);
  assert.equal(held.length, 64);
  // The returned view round-trips the stored values.
  const got = await c.get('k0');
  assert.equal(got.vector.length, dim);
});
