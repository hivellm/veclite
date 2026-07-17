#!/usr/bin/env node
// WASM binding conformance runner (SPEC-012 §5, SPEC-015 §3, gate G5).
//
// Drives the shared YAML corpus + golden.json through `@veclite/wasm` and checks
// it reproduces the Rust reference observations exactly (orderings/ids exact,
// scores within 1e-5). The wasm binding is memory-only, so this runs the subset
// the spec scopes to it: every `memory`/`both` case, skipping `file`-mode cases
// (file locks / mmap / durability — SPEC-012 §5.1). A `reopen` step maps to a
// serialize→deserialize round-trip (the wasm persistence equivalent, WASM-010).
//
// Runs under Node and Deno; the same module also loads in browsers. Set
// VECLITE_WASM_VARIANT=simd128|fallback to pin the wasm build (WASM-002); the
// default auto-detects. Exits non-zero on any divergence.

import { readFileSync, readdirSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import yaml from 'js-yaml';

const TOL = 1e-5;
const here = dirname(fileURLToPath(import.meta.url));

const veclite = await import(
  process.env.VECLITE_WASM ??
    pathToFileURL(resolve(here, '../../../../crates/veclite-wasm/js/index.js')).href
);
const variant = await veclite.ready();

// ── numeric-tolerant comparison (mirrors the Rust/Node runners) ──────────────

const isObj = (x) => x !== null && typeof x === 'object' && !Array.isArray(x);

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

// ── operation dispatch (produces the shared observation shapes) ──────────────

const hitsObs = (hits) => ({ ids: hits.map((h) => h.id), scores: hits.map((h) => h.score) });

async function getObs(coll, a) {
  const p = await coll.get(a.id);
  if (p == null) return { result: null };
  return { result: { id: p.id, vector: Array.from(p.vector ?? []), payload: p.payload ?? null } };
}

async function statsObs(coll) {
  const s = await coll.stats();
  return { value: { dimension: s.dimension, len: s.len, tombstones: s.tombstones, auto_embed: s.autoEmbed } };
}

async function searchObs(coll, a) {
  return hitsObs(await coll.search(Float32Array.from(a.vector), {
    limit: a.limit,
    efSearch: a.ef_search,
    withPayload: a.with_payload,
    withVector: a.with_vector,
    filter: a.filter,
  }));
}

async function hybridObs(coll, a) {
  // `hybridSearch` takes an options object that crosses via serde (unlike the
  // typed-array fast path of `search`); pass `dense` as a plain number[] so it
  // deserializes to a JSON array.
  return hitsObs(await coll.hybridSearch({
    dense: a.dense ?? undefined,
    sparse: a.sparse,
    alpha: a.alpha,
    rrfK: a.rrf_k,
    limit: a.limit,
  }));
}

async function scrollObs(coll, a) {
  const page = await coll.scroll({ limit: a.limit, offsetId: a.offset_id, filter: a.filter });
  return { ids: page.points.map((p) => p.id), next_cursor: page.nextCursor ?? null };
}

async function chunkObs(a) {
  const spans = await veclite.chunk(a.text, a.max_chars, a.overlap);
  return { result: spans.map((c) => ({ text: c.text, start: c.start, end: c.end })) };
}

function codeOf(err) {
  return err && typeof err.code === 'string' ? err.code : 'ERROR';
}

// Run one op, returning its canonical observation ({error: CODE} on failure).
async function execute(db, op, a) {
  try {
    switch (op) {
      case 'create_collection':
        await db.createCollection(a.name, {
          dimension: a.dimension,
          metric: a.metric ?? 'cosine',
          quantizationBits: a.quantization_bits,
          autoEmbed: a.auto_embed,
        });
        return {};
      case 'delete_collection': await db.deleteCollection(a.name); return {};
      case 'list_collections': return { ids: await db.listCollections() };
      case 'create_alias': await db.createAlias(a.alias, a.target); return {};
      case 'delete_alias': await db.deleteAlias(a.alias); return {};
      case 'upsert':
        await (await db.collection(a.collection)).upsert(a.id, Float32Array.from(a.vector), a.payload, a.sparse);
        return {};
      case 'upsert_batch': {
        const pts = a.points.map((p) => ({ id: p.id, vector: Float32Array.from(p.vector), payload: p.payload, sparse: p.sparse }));
        await (await db.collection(a.collection)).upsertBatch(pts);
        return {};
      }
      case 'upsert_text': await (await db.collection(a.collection)).upsertText(a.id, a.text, a.payload); return {};
      case 'refit': await (await db.collection(a.collection)).refit(); return {};
      case 'get': return await getObs(await db.collection(a.collection), a);
      case 'delete': return { value: await (await db.collection(a.collection)).delete(a.id) };
      case 'len': return { value: await (await db.collection(a.collection)).len() };
      case 'stats': return await statsObs(await db.collection(a.collection));
      case 'search': return await searchObs(await db.collection(a.collection), a);
      case 'search_text': return hitsObs(await (await db.collection(a.collection)).searchText(a.query, { limit: a.limit ?? 10 }));
      case 'hybrid_search': return await hybridObs(await db.collection(a.collection), a);
      case 'scroll': return await scrollObs(await db.collection(a.collection), a);
      case 'chunk': return await chunkObs(a);
      default: throw new Error(`unknown op ${op}`);
    }
  } catch (err) {
    if (err && typeof err.code === 'string') return { error: codeOf(err) };
    throw err;
  }
}

// ── case execution (memory mode only) ────────────────────────────────────────

async function runCase(caseDef, golden) {
  // The wasm binding is memory-only: skip file-scoped cases (SPEC-012 §5.1).
  if ((caseDef.mode ?? 'both') === 'file') return { skipped: true, errors: [] };

  const expected = golden[caseDef.id];
  const errors = [];
  let db = await veclite.memory();
  let idx = 0;
  for (let i = 0; i < caseDef.steps.length; i++) {
    const step = caseDef.steps[i];
    const op = step.op;
    const where = `${caseDef.id}: step ${i} \`${op}\``;

    if (op === 'reopen') {
      // The wasm persistence equivalent: serialize the image and load it back
      // (WASM-010). Exercises the full-image codec round-trip.
      const bytes = await db.serialize();
      db = await veclite.deserialize(bytes);
      continue;
    }

    const obs = await execute(db, op, step.args ?? {});

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

    if (expected != null) {
      if (idx >= expected.length) errors.push(`${where}: golden has no entry (re-bless)`);
      else if (!eqTol(expected[idx], obs)) errors.push(`${where}: golden mismatch: ${JSON.stringify(expected[idx])} != ${JSON.stringify(obs)}`);
    }
    idx++;
  }
  return { skipped: false, errors };
}

async function main() {
  const corpus = process.argv[2] ?? resolve(here, '../../corpus');
  const golden = JSON.parse(readFileSync(join(corpus, 'golden.json'), 'utf8'));
  const files = readdirSync(corpus).filter((f) => f.endsWith('.yaml')).sort();
  if (files.length === 0) {
    console.error(`[conformance:wasm] no *.yaml under ${corpus}`);
    process.exit(1);
  }

  let total = 0, failed = 0, skipped = 0;
  for (const f of files) {
    const suite = yaml.load(readFileSync(join(corpus, f), 'utf8'));
    for (const caseDef of suite.cases) {
      total++;
      const { skipped: sk, errors } = await runCase(caseDef, golden);
      if (sk) { skipped++; continue; }
      if (errors.length) {
        failed++;
        for (const e of errors) console.error(`[conformance:wasm] FAIL ${e}`);
      }
    }
  }

  if (failed) {
    console.error(`[conformance:wasm] ${failed}/${total} cases FAILED`);
    process.exit(1);
  }
  console.error(
    `[conformance:wasm] PASS — ${total - skipped}/${total} cases (${skipped} file-mode skipped) ` +
    `across ${files.length} files [variant: ${variant}]`,
  );
}

await main();
