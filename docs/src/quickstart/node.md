# Node.js quickstart

Install the package — a prebuilt native addon, no Rust toolchain:

```bash
npm install @hivehub/veclite
```

The program below opens a durable single-file database, does a filtered k-NN
search over `Float32Array` vectors, and a text search over an offline BM25
auto-embed collection. Both sync and async APIs are available (`openSync` /
`open`); this uses the sync surface.

```javascript
{{#include ../../../examples/quickstart.mjs}}
```

Run it:

```bash
node examples/quickstart.mjs
# veclite: quickstart OK (a, c)
```

See [SPEC-010](../../specs/SPEC-010-binding-node.md) for the full surface
(prebuilds, the conformance suite, and the async methods).
