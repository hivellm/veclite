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

module.exports = {
  open: wrap(native.open),
  openSync: wrap(native.openSync),
  memory: native.memory, // infallible
  Database: native.Database,
  Collection: native.Collection,
  VecLiteError,
};
