// Package entry (SPEC-010). Loads the napi-generated binding (`index.js`,
// which picks the right platform `.node`) and adds the one piece the native
// layer cannot express directly: a `VecLiteError` with a string `code`.
//
// The native methods encode failures as `"<CODE><message>"` in the error
// reason (napi maps the `code` field to a fixed `Status` string, so we carry
// our own code in the message). This wrapper splits that back out (NODE-020):
// one idiomatic error class + `err.code`, `err.message` identical to Rust.

'use strict';

const native = require('./index.js');

const SEP = String.fromCharCode(1); // U+0001, the native code/message separator

/** Error thrown/rejected by every VecLite operation (NODE-020). */
class VecLiteError extends Error {
  constructor(code, message) {
    super(message);
    this.name = 'VecLiteError';
    /** @type {string} stable machine-readable code, e.g. "DIMENSION_MISMATCH". */
    this.code = code;
  }
}

function convert(err) {
  if (err && typeof err.message === 'string') {
    const i = err.message.indexOf(SEP);
    if (i !== -1) {
      return new VecLiteError(err.message.slice(0, i), err.message.slice(i + 1));
    }
  }
  return err;
}

// Wrap a native function so its throw / promise-rejection surfaces as a
// VecLiteError. Both sync twins (throw) and async methods (reject) are covered.
function wrap(fn) {
  return function wrapped(...args) {
    let out;
    try {
      out = fn.apply(this, args);
    } catch (err) {
      throw convert(err);
    }
    if (out && typeof out.then === 'function') {
      return out.then(undefined, (err) => {
        throw convert(err);
      });
    }
    return out;
  };
}

// Patch every method on the native classes in place — instances returned by
// the factories/`collection()` inherit the wrapped prototype methods.
for (const Cls of [native.Database, native.Collection]) {
  for (const name of Object.getOwnPropertyNames(Cls.prototype)) {
    if (name === 'constructor') continue;
    const desc = Object.getOwnPropertyDescriptor(Cls.prototype, name);
    if (desc && typeof desc.value === 'function') {
      Cls.prototype[name] = wrap(desc.value);
    }
  }
}

// ── leaked-handle warning (NODE-013) ────────────────────────────────────────
// A file-backed Database holds an advisory file lock and defers a close-time
// checkpoint until close(). If it is garbage-collected without close(), warn —
// the same discipline Node uses for unclosed file descriptors. Memory databases
// hold no external resource, so they are not tracked. Each tracked db carries an
// unregister token; close() removes it so a properly-closed handle never warns.
const registry =
  typeof FinalizationRegistry === 'function'
    ? new FinalizationRegistry((path) => {
        process.emitWarning(
          `VecLite database "${path}" was garbage-collected without close(). ` +
            `Call db.close() to release the file lock and run the close-time ` +
            `checkpoint deterministically (NODE-013).`,
          { code: 'VECLITE_HANDLE_LEAK' },
        );
      })
    : null;

const tokens = new WeakMap();

// Track a file-backed handle so a leak warns; return it unchanged.
function track(db, path) {
  if (registry && db && typeof db === 'object') {
    const token = {};
    registry.register(db, path, token);
    tokens.set(db, token);
  }
  return db;
}

// Stop tracking a handle (called from close(), and idempotent).
function untrack(db) {
  if (registry) {
    const token = tokens.get(db);
    if (token !== undefined) {
      registry.unregister(token);
      tokens.delete(db);
    }
  }
}

// close() must un-track before releasing, so a closed handle never warns.
const nativeClose = native.Database.prototype.close;
native.Database.prototype.close = function close(...args) {
  untrack(this);
  return nativeClose.apply(this, args);
};

// Register file-backed databases from the two path-taking factories.
const openWrapped = wrap(native.open);
const openSyncWrapped = wrap(native.openSync);

module.exports = {
  open: async function open(path, ...rest) {
    return track(await openWrapped(path, ...rest), path);
  },
  openSync: function openSync(path, ...rest) {
    return track(openSyncWrapped(path, ...rest), path);
  },
  memory: native.memory, // infallible; in-memory, no external resource to leak
  chunk: native.chunk, // infallible, pure (SPEC-005 §7)
  Database: native.Database,
  Collection: native.Collection,
  VecLiteError,
};
