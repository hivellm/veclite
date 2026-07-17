// @veclite/wasm — the JS entry point for the WebAssembly binding (SPEC-012).
//
// Three layers over the wasm-bindgen core (`../pkg/veclite_core.js`):
//   1. A feature-detection loader (WASM-002) that picks the simd128 binary when
//      the runtime supports it, else the plain fallback, and loads the wasm
//      bytes the right way for Node / Deno / Bun / browsers.
//   2. An async, camelCase facade (WASM-020): every method returns a promise
//      even though execution inside the module is synchronous.
//   3. An OPFS backend (WASM-011): the database runs on an in-memory image;
//      `save()` checkpoint-serializes and writes it atomically (temp + move),
//      with optional autosave after N writes / every M ms.
//
// The bytes crossing `serialize()` / `deserialize()` are a valid `.veclite` v1
// file image, interchangeable with native VecLite (WASM-010).

import initCore, { WasmDb, chunk as coreChunk } from '../pkg/veclite_core.js';

// ── feature-detection loader (WASM-002) ──────────────────────────────────────

// Canonical simd128 probe (a module whose body uses `i8x16.splat`): it only
// validates on a runtime with WebAssembly SIMD enabled.
const SIMD_PROBE = new Uint8Array([
  0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 96, 0, 1, 123, 3, 2, 1, 0,
  10, 10, 1, 8, 0, 65, 0, 253, 15, 253, 98, 11,
]);

function simdSupported() {
  try {
    return WebAssembly.validate(SIMD_PROBE);
  } catch {
    return false;
  }
}

// Load the raw wasm bytes for `fileUrl` (a URL relative to this module). Node
// has no `fetch` of `file://`, so read from disk there; Deno/Bun/browsers fetch.
async function loadBytes(fileUrl) {
  const isNode =
    typeof process !== 'undefined' &&
    process.versions != null &&
    process.versions.node != null &&
    typeof Deno === 'undefined';
  if (isNode) {
    const { readFile } = await import('node:fs/promises');
    const { fileURLToPath } = await import('node:url');
    return readFile(fileURLToPath(fileUrl));
  }
  const resp = await fetch(fileUrl);
  return new Uint8Array(await resp.arrayBuffer());
}

let _ready = null;
let _variant = null;

// Initialize the wasm module exactly once (idempotent). Returns which variant
// (`"simd128"` | `"fallback"`) was loaded — useful for diagnostics/tests.
export function ready() {
  if (_ready == null) {
    _ready = (async () => {
      // An explicit override (`VECLITE_WASM_VARIANT=simd128|fallback`, or the
      // global `globalThis.VECLITE_WASM_VARIANT`) forces a variant — used by the
      // test suite to exercise both builds (WASM-002); otherwise auto-detect.
      const forced =
        (typeof process !== 'undefined' && process.env?.VECLITE_WASM_VARIANT) ||
        (typeof globalThis !== 'undefined' && globalThis.VECLITE_WASM_VARIANT) ||
        null;
      const simd = forced ? forced === 'simd128' : simdSupported();
      _variant = simd ? 'simd128' : 'fallback';
      const name = simd ? 'veclite_simd.wasm' : 'veclite_fallback.wasm';
      const url = new URL(`../pkg/${name}`, import.meta.url);
      await initCore({ module_or_path: await loadBytes(url) });
      return _variant;
    })();
  }
  return _ready;
}

/** Which wasm variant is loaded (`null` until `ready()` resolves). */
export function loadedVariant() {
  return _variant;
}

// ── error normalization ──────────────────────────────────────────────────────

// The core throws a JS `Error` already carrying a `code` string (WASM-021); this
// is a pass-through hook kept so every facade call funnels through one place.
function rethrow(err) {
  throw err;
}

// ── OPFS backend (WASM-011) ──────────────────────────────────────────────────

async function opfsRoot() {
  if (
    typeof navigator === 'undefined' ||
    navigator.storage == null ||
    typeof navigator.storage.getDirectory !== 'function'
  ) {
    const e = new Error('OPFS is not available in this runtime');
    e.code = 'INVALID_ARGUMENT';
    throw e;
  }
  return navigator.storage.getDirectory();
}

async function opfsRead(name) {
  const root = await opfsRoot();
  let handle;
  try {
    handle = await root.getFileHandle(name, { create: false });
  } catch {
    return null; // no existing image → start empty
  }
  const file = await handle.getFile();
  return new Uint8Array(await file.arrayBuffer());
}

// Atomic write: stage to `<name>.tmp`, then move it onto `name` (OPFS `move`
// is an atomic rename). A crash before the move leaves the previous image
// intact (WASM-011).
async function opfsWriteAtomic(name, bytes) {
  const root = await opfsRoot();
  const tmpName = `${name}.tmp`;
  const tmp = await root.getFileHandle(tmpName, { create: true });
  const w = await tmp.createWritable();
  await w.write(bytes);
  await w.close();
  if (typeof tmp.move === 'function') {
    await tmp.move(name);
  } else {
    // Fallback for runtimes without FileSystemFileHandle.move: overwrite in
    // place (a narrower crash window, but the only option there).
    const dest = await root.getFileHandle(name, { create: true });
    const dw = await dest.createWritable();
    await dw.write(bytes);
    await dw.close();
    await root.removeEntry(tmpName).catch(() => {});
  }
}

// ── Collection facade ────────────────────────────────────────────────────────

class Collection {
  constructor(coreColl, db) {
    this._c = coreColl;
    this._db = db;
  }

  async len() {
    return this._c.len();
  }
  async isEmpty() {
    return this._c.isEmpty();
  }

  async upsert(id, vector, payload, sparse) {
    try {
      this._c.upsert(id, toF32(vector), payload, sparse);
    } catch (e) {
      rethrow(e);
    }
    await this._db._noteWrite();
  }

  async upsertBatch(points) {
    try {
      this._c.upsertBatch(points.map((p) => ({ ...p, vector: Array.from(p.vector) })));
    } catch (e) {
      rethrow(e);
    }
    await this._db._noteWrite(points.length);
  }

  async upsertText(id, text, payload) {
    try {
      this._c.upsertText(id, text, payload);
    } catch (e) {
      rethrow(e);
    }
    await this._db._noteWrite();
  }

  async refit() {
    try {
      this._c.refit();
    } catch (e) {
      rethrow(e);
    }
  }

  async search(query, options) {
    try {
      return withF32Vectors(this._c.search(toF32(query), options ?? {}));
    } catch (e) {
      return rethrow(e);
    }
  }

  async searchText(query, options) {
    try {
      return withF32Vectors(this._c.searchText(query, options ?? {}));
    } catch (e) {
      return rethrow(e);
    }
  }

  async hybridSearch(options) {
    try {
      return withF32Vectors(this._c.hybridSearch(options ?? {}));
    } catch (e) {
      return rethrow(e);
    }
  }

  async scroll(options) {
    try {
      const page = this._c.scroll(options ?? {});
      page.points = withF32Vectors(page.points);
      return page;
    } catch (e) {
      return rethrow(e);
    }
  }

  async get(id) {
    try {
      const p = this._c.get(id);
      return p == null ? null : withF32Vector(p);
    } catch (e) {
      return rethrow(e);
    }
  }

  async delete(id) {
    let existed;
    try {
      existed = this._c.delete(id);
    } catch (e) {
      return rethrow(e);
    }
    await this._db._noteWrite();
    return existed;
  }

  async stats() {
    return this._c.stats();
  }
}

// Return output vectors as `Float32Array` (the core serializes them as plain
// arrays); mirrors the SPEC-010 hit shape.
function withF32Vector(hit) {
  if (hit != null && Array.isArray(hit.vector)) hit.vector = Float32Array.from(hit.vector);
  return hit;
}
function withF32Vectors(hits) {
  for (const h of hits) withF32Vector(h);
  return hits;
}
function toF32(v) {
  return v instanceof Float32Array ? v : Float32Array.from(v);
}

// ── Database facade ──────────────────────────────────────────────────────────

class Database {
  constructor(coreDb, opts) {
    this._db = coreDb;
    this._opfs = opts?.opfs ?? null;
    this._autosave = opts?.autosave ?? null;
    this._writesSinceSave = 0;
    this._timer = null;
    if (this._opfs && this._autosave?.intervalMs) {
      this._timer = setInterval(() => {
        this.save().catch(() => {});
      }, this._autosave.intervalMs);
      // Do not keep a Node/Bun process alive just for the autosave timer.
      if (typeof this._timer?.unref === 'function') this._timer.unref();
    }
  }

  async createCollection(name, options) {
    let coll;
    try {
      coll = this._db.createCollection(name, options ?? {});
    } catch (e) {
      return rethrow(e);
    }
    await this._noteWrite();
    return new Collection(coll, this);
  }

  async collection(name) {
    try {
      return new Collection(this._db.collection(name), this);
    } catch (e) {
      return rethrow(e);
    }
  }

  async listCollections() {
    return this._db.listCollections();
  }

  async deleteCollection(name) {
    try {
      this._db.deleteCollection(name);
    } catch (e) {
      rethrow(e);
    }
    await this._noteWrite();
  }

  async createAlias(alias, target) {
    try {
      this._db.createAlias(alias, target);
    } catch (e) {
      rethrow(e);
    }
    await this._noteWrite();
  }

  async deleteAlias(alias) {
    try {
      this._db.deleteAlias(alias);
    } catch (e) {
      rethrow(e);
    }
    await this._noteWrite();
  }

  formatVersion() {
    return this._db.formatVersion();
  }

  // Compaction happens on serialize/save; a no-op here (WASM-020).
  async vacuum() {}

  /** The `.veclite` v1 file image (WASM-010), as a `Uint8Array`. */
  async serialize() {
    try {
      return this._db.serialize();
    } catch (e) {
      return rethrow(e);
    }
  }

  /** Checkpoint-serialize and write atomically to OPFS (WASM-011). */
  async save() {
    if (!this._opfs) {
      const e = new Error('save() requires an OPFS-backed database (open({ opfs }))');
      e.code = 'INVALID_ARGUMENT';
      throw e;
    }
    const bytes = this._db.serialize();
    await opfsWriteAtomic(this._opfs, bytes);
    this._writesSinceSave = 0;
  }

  /** Flush (if OPFS-backed) and release the timer. Idempotent. */
  async close() {
    if (this._timer != null) {
      clearInterval(this._timer);
      this._timer = null;
    }
    if (this._opfs && this._writesSinceSave > 0) {
      await this.save();
    }
  }

  // Internal: count a mutation and autosave after the write threshold.
  async _noteWrite(n = 1) {
    if (!this._opfs || !this._autosave) return;
    this._writesSinceSave += n;
    const after = this._autosave.afterWrites;
    if (after && this._writesSinceSave >= after) {
      await this.save();
    }
  }
}

// ── top-level API (SPEC-012 §3) ──────────────────────────────────────────────

/** An ephemeral in-memory database (lost on unload). */
export async function memory() {
  await ready();
  return new Database(WasmDb.memory(), {});
}

/**
 * Open a database. With `{ opfs: "name" }` it loads the persisted image from
 * OPFS (or starts empty) and enables `save()`/autosave; otherwise it is an
 * in-memory database identical to `memory()`.
 */
export async function open(options) {
  await ready();
  const opts = options ?? {};
  if (opts.opfs) {
    const existing = await opfsRead(opts.opfs);
    const core = existing ? WasmDb.fromBytes(existing) : WasmDb.memory();
    return new Database(core, opts);
  }
  return new Database(WasmDb.memory(), opts);
}

/** Load a database from a `.veclite` v1 file image (WASM-010). */
export async function deserialize(bytes) {
  await ready();
  try {
    return new Database(WasmDb.fromBytes(bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes)), {});
  } catch (e) {
    return rethrow(e);
  }
}

/** Split text into overlapping, UTF-8-safe chunks (SPEC-005 §7). */
export async function chunk(text, maxChars, overlap) {
  await ready();
  return coreChunk(text, maxChars, overlap);
}
