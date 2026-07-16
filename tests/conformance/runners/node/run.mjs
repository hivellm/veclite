#!/usr/bin/env node
// Node.js binding conformance runner (SPEC-015 §3, TST-020..023).
//
// Loads the shared YAML corpus and the committed golden.json and drives them
// through the `veclite` npm package (or the local build via VECLITE_NODE),
// reproducing the Rust reference runner's observations exactly (orderings/ids
// exact, scores within 1e-5). Exits non-zero on any divergence.
//
// Run with `node --expose-gc run.mjs [corpus_dir]` so `reopen` can drop the
// native handle deterministically before re-opening the file.

import { readFileSync, readdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import yaml from 'js-yaml';

const TOL = 1e-5;
const here = dirname(fileURLToPath(import.meta.url));

// Resolve the binding: the installed package by default, or a local build when
// VECLITE_NODE points at a veclite.js (used by the repo's local gate).
async function loadVeclite() {
  const override = process.env.VECLITE_NODE;
  const spec = override
    ? pathToFileURL(resolve(override)).href
    : pathToFileURL(resolve(here, '../../../../crates/veclite-node/veclite.js')).href;
  try {
    const mod = await import('veclite');
    return mod.default ?? mod;
  } catch {
    const mod = await import(spec);
    return mod.default ?? mod;
  }
}

const veclite = await loadVeclite();

// ── numeric-tolerant comparison (mirrors the Rust/Python runners) ────────────

function eqTol(a, b) {
  if (typeof a === 'boolean' || typeof b === 'boolean') return a === b;
  if (typeof a === 'number' && typeof b === 'number') return Math.abs(a - b) <= TOL;
  if (Array.isArray(a) && Array.isArray(b)) {
    return a.length === b.length && a.every((x, i) => eqTol(x, b[i]));
  }
  if (isObj(a) && isObj(b)) {
    const ak = Object.keys(a), bk = Object.keys(b);
    return ak.length === bk.length && ak.every((k) => k in b && eqTol(a[k], b[k]));
  }
  return a === b;
}

function matchesSubset(want, got) {
  if (isObj(want) && isObj(got)) {
    return Object.entries(want).every(([k, v]) => k in got && matchesSubset(v, got[k]));
  }
  if (Array.isArray(want) && Array.isArray(got)) {
    return want.length === got.length && want.every((x, i) => matchesSubset(x, got[i]));
  }
  if (typeof want === 'boolean' || typeof got === 'boolean') return want === got;
  if (typeof want === 'number' && typeof got === 'number') return Math.abs(want - got) <= TOL;
  return want === got;
}

const isObj = (x) => x !== null && typeof x === 'object' && !Array.isArray(x);

// ── operation dispatch (produces the shared observation shapes) ──────────────

const hitsObs = (hits) => ({ ids: hits.map((h) => h.id), scores: hits.map((h) => h.score) });

function createCollection(db, a) {
  db.createCollectionSync(a.name, {
    dimension: a.dimension,
    metric: a.metric ?? 'cosine',
    quantizationBits: a.quantization_bits,
    autoEmbed: a.auto_embed,
  });
  return {};
}

function getObs(coll, a) {
  const p = coll.get(a.id);
  if (p == null) return { result: null };
  return { result: { id: p.id, vector: Array.from(p.vector ?? []), payload: p.payload ?? null } };
}

function statsObs(coll) {
  const s = coll.stats();
  return { value: { dimension: s.dimension, len: s.len, tombstones: s.tombstones, auto_embed: s.autoEmbed } };
}

function searchObs(coll, a) {
  return hitsObs(coll.searchSync(Float32Array.from(a.vector), {
    limit: a.limit,
    efSearch: a.ef_search,
    withPayload: a.with_payload,
    withVector: a.with_vector,
    filter: a.filter,
  }));
}

function hybridObs(coll, a) {
  return hitsObs(coll.hybridSearch({
    dense: a.dense ? Float32Array.from(a.dense) : undefined,
    sparse: a.sparse,
    alpha: a.alpha,
    rrfK: a.rrf_k,
    limit: a.limit,
  }));
}

function scrollObs(coll, a) {
  const page = coll.scroll({ limit: a.limit, offsetId: a.offset_id, filter: a.filter });
  return { ids: page.points.map((p) => p.id), next_cursor: page.nextCursor ?? null };
}

function chunkObs(a) {
  return { result: veclite.chunk(a.text, a.max_chars, a.overlap).map((c) => ({ text: c.text, start: c.start, end: c.end })) };
}

function codeOf(err) {
  return err && typeof err.code === 'string' ? err.code : 'ERROR';
}

// Run one op, returning its canonical observation ({error: CODE} on failure).
function execute(db, op, a) {
  try {
    switch (op) {
      case 'create_collection': return createCollection(db, a);
      case 'delete_collection': db.deleteCollection(a.name); return {};
      case 'list_collections': return { ids: db.listCollections() };
      case 'create_alias': db.createAlias(a.alias, a.target); return {};
      case 'delete_alias': db.deleteAlias(a.alias); return {};
      // Pass `undefined` (not `null`) for absent payload/sparse so napi maps
      // them to `None`; a JS `null` would become `Some(Value::Null)`.
      case 'upsert': db.collection(a.collection).upsertSync(a.id, Float32Array.from(a.vector), a.payload, a.sparse); return {};
      case 'upsert_batch': {
        const pts = a.points.map((p) => ({ id: p.id, vector: Float32Array.from(p.vector), payload: p.payload, sparse: p.sparse }));
        db.collection(a.collection).upsertBatchSync(pts);
        return {};
      }
      case 'upsert_text': db.collection(a.collection).upsertTextSync(a.id, a.text, a.payload); return {};
      case 'refit': db.collection(a.collection).refit(); return {};
      case 'get': return getObs(db.collection(a.collection), a);
      case 'delete': return { value: db.collection(a.collection).delete(a.id) };
      case 'len': return { value: db.collection(a.collection).len() };
      case 'stats': return statsObs(db.collection(a.collection));
      case 'search': return searchObs(db.collection(a.collection), a);
      case 'search_text': return hitsObs(db.collection(a.collection).searchTextSync(a.query, { limit: a.limit ?? 10 }));
      case 'hybrid_search': return hybridObs(db.collection(a.collection), a);
      case 'scroll': return scrollObs(db.collection(a.collection), a);
      case 'chunk': return chunkObs(a);
      default: throw new Error(`unknown op ${op}`);
    }
  } catch (err) {
    if (err && typeof err.code === 'string') return { error: codeOf(err) };
    throw err;
  }
}

// ── case execution ───────────────────────────────────────────────────────────

function openDb(path) {
  return path == null ? veclite.memory() : veclite.openSync(path, { durability: 'full' });
}

// Drop leaked native handles: force GC and drain a macrotask so napi
// finalizers run (V8 GC is not synchronous like Python refcounting), releasing
// the advisory file lock before a reopen or the next file-backed case.
// Force finalization of leaked per-op Collection handles (which keep file
// mappings — and thus the OS lock — alive) so an immediate same-path reopen
// succeeds. Node exposes `global.gc` under --expose-gc; Bun exposes `Bun.gc`.
function forceGc() {
  if (typeof Bun !== 'undefined' && typeof Bun.gc === 'function') Bun.gc(true);
  else if (global.gc) global.gc();
}

async function drainHandles() {
  forceGc();
  await new Promise((r) => setImmediate(r));
  forceGc();
}

async function runCase(caseDef, golden) {
  const mode = caseDef.mode ?? 'both';
  const modes = { both: [null, true], memory: [null], file: [true] }[mode];
  const expected = golden[caseDef.id];
  const errors = [];
  for (const fileMode of modes) errors.push(...(await runCaseInMode(caseDef, fileMode, expected)));
  return errors;
}

async function runCaseInMode(caseDef, fileMode, golden) {
  const errors = [];
  let dir = null, path = null;
  if (fileMode) {
    dir = mkdtempSync(join(tmpdir(), 'veclite-conf-'));
    path = join(dir, 'db.veclite');
  }
  let db;
  try {
    db = openDb(path);
  } catch (err) {
    if (dir) rmSync(dir, { recursive: true, force: true });
    return [`${caseDef.id}: open (${fileMode ? 'file' : 'memory'}): ${err.code ?? err.message}`];
  }
  try {
    let idx = 0;
    for (let i = 0; i < caseDef.steps.length; i++) {
      const step = caseDef.steps[i];
      const op = step.op;
      const where = `${caseDef.id}: step ${i} \`${op}\``;

      if (op === 'reopen') {
        if (path == null) { errors.push(`${where}: reopen requires file mode`); break; }
        // Close the db (checkpoint + lock release, NODE-013) and drain leaked
        // per-op Collection handles before re-opening; acked writes also
        // survive via WAL replay under `durability: full`.
        await db.close();
        db = null;
        await drainHandles();
        db = openDb(path);
        continue;
      }

      const obs = execute(db, op, step.args ?? {});

      const expect = step.expect;
      if (expect != null) {
        for (const [key, want] of Object.entries(expect)) {
          if (key === 'error') {
            if (obs.error !== want) errors.push(`${where}: expected error ${want}, got ${JSON.stringify(obs)}`);
          } else if (!matchesSubset(want, obs[key])) {
            errors.push(`${where}: \`${key}\`: expected ${JSON.stringify(want)}, got ${JSON.stringify(obs[key])}`);
          }
        }
      } else if ('error' in obs) {
        errors.push(`${where}: unexpected error ${obs.error}`);
      }

      if (golden != null) {
        if (idx >= golden.length) errors.push(`${where}: golden has no entry (re-bless)`);
        else if (!eqTol(golden[idx], obs)) errors.push(`${where}: golden mismatch: ${JSON.stringify(golden[idx])} != ${JSON.stringify(obs)}`);
      }
      idx++;
    }
  } finally {
    if (db && fileMode) {
      try { await db.close(); } catch { /* already closed */ }
    }
    db = null;
    await drainHandles();
    if (dir) rmSync(dir, { recursive: true, force: true });
  }
  return errors;
}

async function main() {
  const corpus = process.argv[2] ?? resolve(here, '../../corpus');
  const golden = JSON.parse(readFileSync(join(corpus, 'golden.json'), 'utf8'));
  const files = readdirSync(corpus).filter((f) => f.endsWith('.yaml')).sort();
  if (files.length === 0) {
    console.error(`[conformance:node] no *.yaml under ${corpus}`);
    process.exit(1);
  }

  let total = 0, failed = 0;
  for (const f of files) {
    const suite = yaml.load(readFileSync(join(corpus, f), 'utf8'));
    for (const caseDef of suite.cases) {
      total++;
      const errs = await runCase(caseDef, golden);
      if (errs.length) {
        failed++;
        for (const e of errs) console.error(`[conformance:node] FAIL ${e}`);
      }
    }
  }

  if (failed) {
    console.error(`[conformance:node] ${failed}/${total} cases FAILED`);
    process.exit(1);
  }
  console.error(`[conformance:node] PASS — ${total} cases across ${files.length} files`);
}

await main();
