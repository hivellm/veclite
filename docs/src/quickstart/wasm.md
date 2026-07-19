# WASM quickstart (browser)

Client-side vector search with no server and no backend: the pure-Rust engine
compiled to WebAssembly, plus the portable `.veclite` image codec.

```bash
npm install @hivehub/veclite-wasm
```

The distinctive move is the last step — an image serialized in the browser opens
byte-for-byte in native VecLite (`serialize` / `deserialize`, WASM-010). Write a
collection on the client, ship the bytes, read them on the server. Every method
is `async` (the wasm module loads on demand).

```javascript
{{#include ../../../crates/veclite-wasm/examples/quickstart.mjs}}
```

Run it under Node against the built package:

```bash
cd crates/veclite-wasm
bash build-pkg.sh                    # builds the wasm artifact next to js/
node examples/quickstart.mjs
```

In a browser, `open({ opfs: 'app', autosave: { afterWrites: 50 } })` persists
the image to the Origin Private File System and `save()` writes it atomically.
See [SPEC-012](../../specs/SPEC-012-binding-wasm.md) for OPFS, the SIMD variants,
and sizing guidance.
