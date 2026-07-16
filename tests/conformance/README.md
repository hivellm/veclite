# Binding conformance corpus (SPEC-015 §3 · FR-65)

One language-agnostic YAML corpus, executed by a runner in every binding
(Rust, Python, Node today; Go, C#, WASM as they land). A binding is
**release-blocked** until the corpus passes on its full platform matrix
(TST-020). The Rust runner is the **reference**: it defines the golden
outcomes; the other runners must reproduce them exactly.

```
tests/conformance/
  corpus/            *.yaml — the cases (this is the source of truth)
  runners/
    python/run.py    consumes the same corpus via the veclite wheel
    node/run.mjs      consumes the same corpus via the veclite npm package
```

The Rust reference runner lives in `xtask` (`cargo xtask conformance`).

## Running

```bash
cargo xtask conformance                 # Rust reference — must be green
cargo xtask conformance --bless         # fill null golden fields, rewrite YAML
python tests/conformance/runners/python/run.py   # needs the veclite wheel
node   tests/conformance/runners/node/run.mjs     # needs the veclite npm pkg
```

Every runner loads the same `corpus/*.yaml`, executes each case, and compares
against the committed expectations with the identical semantics below.

## Corpus schema

```yaml
suite: <name>                 # suite label, shown in failure output
cases:
  - id: conf-<slug>           # stable case id (TST-022), unique across the corpus
    desc: <one line>          # what this case pins
    mode: both                # both (default) | memory | file
    steps:
      - op: <operation>
        args: { ... }         # operation arguments; absent fields take API defaults
        expect: { ... }       # optional; keys present are asserted
```

### Modes and `memory ≡ file` (TST-021)

`mode: both` (the default) runs the case twice — against an in-memory database
(`VecLite.memory()`) and a file-backed one — and requires **identical**
outcomes. That is the in-memory ≡ file-backed guarantee. Use `mode: file` for
cases containing a `reopen` step (WAL replay / auto-embed reopen determinism),
which is meaningless in memory; use `mode: memory` only when a case cannot be
file-backed.

### Operations

| op | args | observation asserted via |
|----|------|--------------------------|
| `create_collection` | `name, dimension, metric?, quantization_bits?, auto_embed?, hnsw?, payload_indexes?` | `error?` |
| `delete_collection` | `name` | `error?` |
| `list_collections` | — | `ids` (sorted names) |
| `create_alias` / `delete_alias` | `alias, target?` | `error?` |
| `upsert` | `collection, id, vector, payload?, sparse?` | `error?` |
| `upsert_batch` | `collection, points: [{id, vector, payload?, sparse?}]` | `error?` |
| `upsert_text` | `collection, id, text, payload?` | `error?` |
| `refit` | `collection` | `error?` |
| `get` | `collection, id` | `result` (`{id, vector, payload}` or `null`) |
| `delete` | `collection, id` | `value` (bool) |
| `len` | `collection` | `value` (int) |
| `stats` | `collection` | `value` (`{dimension, len, tombstones, auto_embed}`) |
| `search` | `collection, vector, limit?, ef_search?, with_payload?, with_vector?, filter?` | `ids`, `scores` |
| `search_text` | `collection, query, limit?` | `ids`, `scores` |
| `hybrid_search` | `collection, dense?, sparse?, alpha?, rrf_k?, limit?` | `ids`, `scores` |
| `scroll` | `collection, limit?, offset_id?, filter?` | `ids`, `next_cursor` |
| `chunk` | `text, max_chars?, overlap?` | `result` (`[{text, start, end}]`) |
| `reopen` | — | checkpoint + close + reopen (file mode only) |

### Expectations

An `expect` mapping may carry any of:

- `error: <CODE>` — the op must fail with this stable error code. Codes are the
  FFI/Node codes (`COLLECTION_NOT_FOUND`, `VECTOR_NOT_FOUND`, `ALREADY_EXISTS`,
  `DIMENSION_MISMATCH`, `INVALID_ARGUMENT`, `UNSUPPORTED_PROVIDER`, `READ_ONLY`,
  …), so an assertion is language-independent.
- `ids: [...]` — exact ordering (and id set) of ranked/scrolled results.
- `scores: [...]` — per-hit scores, compared within **1e-5** (TST-022).
- `result: ...` / `value: ...` / `next_cursor: ...` — deep-compared, numbers
  within 1e-5 (so YAML ints match `f32` outputs).

All comparisons use one numeric-tolerant deep-equal (`1e-5`); orderings and id
sets are exact because they are strings.

### Blessing golden values

A `null`-valued expectation field is a request for the reference runner to fill
it: `scores: null`, `result: null`, `next_cursor: null`. Run
`cargo xtask conformance --bless` and the Rust reference writes its observed
value into the YAML. This is how golden scores are committed without being
hand-computed.

## Mutation guard (TST-023)

The corpus is versioned with the storage format. **New cases may be added
freely.** Changing an already-committed expected value (an `ids` list, a blessed
`scores`/`result`) means *behavior changed* and carries the same review bar as a
storage-format change — because for a passing corpus the only reason a golden
value moves is that an engine's output moved. Reviewers: treat any diff to an
existing expectation as a behavior change requiring justification, not a
mechanical re-bless. Re-blessing to paper over a real divergence is the failure
mode this guard exists to catch.
