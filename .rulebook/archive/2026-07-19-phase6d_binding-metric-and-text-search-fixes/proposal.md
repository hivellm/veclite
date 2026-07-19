# Proposal: phase6d_binding-metric-and-text-search-fixes

## Why

Two defects found by dogfooding the published 0.1.1 wheel — indexing this
repository's own `docs/` corpus and asking it questions a maintainer would ask.
Neither is caught by the conformance corpus, because both concern behaviour the
corpus never exercises: it always supplies in-vocabulary queries, and never sets
`metric` together with an embedding provider.

### Defect 1 — `metric` silently discarded when a provider is set

Creating a collection with an embedding provider drops the caller's `metric` and
forces the default (cosine), with no error and no warning. Confirmed on disk via
`veclite inspect`:

```
euclid_puro       dim 4, metric euclidean   <- honoured
euclid_provider   dim 4, metric cosine      <- euclidean was requested
```

Cause: every binding takes the `auto_embed` convenience constructor, which calls
`CollectionOptions::new(dimension, Metric::default())` and never reads the metric
the caller passed. The Rust API itself is fine — `CollectionOptions` exposes
`metric` as a field, so there is no correctness reason to force cosine.

The caller gets a different collection than requested, and the metric is
persisted, so it cannot be corrected without recreating the collection.

### Defect 2 — `search_text` raises on an out-of-vocabulary query

```python
c.search_text("zzz unknownterm qqq", limit=3)
# InvalidArgument: zero query vector is not allowed with the cosine metric
```

A query whose terms are absent from the vocabulary embeds to the zero vector and
trips the cosine guard in `collection.rs`. That guard is correct for `search()`,
where an explicitly all-zero query is a programming error — cosine is undefined
there. It is wrong on the text path: "no term matched" is an ordinary search
outcome and should yield an empty result set. The message also leaks vectors and
metrics at a caller who supplied neither.

Any application whose user types an unindexed word gets an exception instead of
"no results".

## What Changes

1. Each binding builds `CollectionOptions` from the requested metric and then
   attaches the embedding provider, instead of using the `auto_embed`
   constructor that hardcodes the default.
2. `search_text` (and the hybrid text lane) returns an empty result set when the
   embedded query is the zero vector, rather than surfacing the cosine guard.
   `search()` keeps the guard unchanged — an explicit zero vector stays an error.
3. README notes that `bm25` is lexical, so natural-language questions want the
   dense `onnx` tier. Measured on 48 files of `docs/`: 3/10 at rank 1 for
   natural-language phrasing, 4/10 (8/10 within the top three files) for keyword
   phrasing. That is BM25 working as designed; the gap is that the flagship
   example pairs a natural-language question with the lexical default.

## Impact

- Affected specs: SPEC-004 (Rust API — text-search semantics), SPEC-005
  (embeddings — zero-vector outcome), SPEC-009/010/011 (Python/Node/C# bindings),
  SPEC-008 (C ABI, which the Go and C# bindings inherit)
- Affected code: `crates/veclite/src/collection.rs`,
  `crates/veclite-py/src/lib.rs`, `crates/veclite-ffi/src/lib.rs`,
  `crates/veclite-node/src/lib.rs`, `README.md`
- Breaking change: NO for defect 1 (callers start getting the metric they asked
  for; anyone who passed a non-default metric with a provider was silently
  ignored and is not relying on it). Behavioural change for defect 2 —
  `search_text` stops raising and returns `[]`. Callers catching the exception
  keep working; nobody can depend on an error they never wanted.
- User benefit: collections are created as requested rather than silently
  altered, and a search for an unindexed word returns no results instead of
  crashing the caller.
