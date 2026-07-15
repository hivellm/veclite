# SPEC-006 — Payloads, Payload Indexes & Filters

| | |
|---|---|
| **Status** | Implemented (phase3a + phase3e): filter model + server-parity semantics + declared **and runtime** payload indexes (`create_payload_index`, journaled as PIDX_DECLARE and sealed as PIDX segments — declarations survive crash and reopen) + the FLT-030 planner (selective pre-filter / HNSW over-fetch post-filter with adaptive growth and an exact-scan fallback, property-tested identical to the scan baseline). Filtered `scroll` (FLT-032) shipped with phase3d. |
| **Phase / tasks** | Phase 3 · T3.1–T3.3 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-24, FR-32, FR-33 |
| **Planning source** | [01-vision §feature matrix](../vectorizer-lite/01-vision-and-scope.md), server sources `db/payload_index.rs`, `models/qdrant/filter.rs`, `filter_processor.rs` |

Requirement IDs `FLT-xxx`. The filter data model is the extract-and-adapt of the server's Qdrant-style model — semantics MUST match the server (conformance gate G3).

## 1. Payloads

- **FLT-001** A payload is one JSON value (`serde_json::Value`), stored per vector, ≤ 16 MiB compressed (STG limits). Bindings expose native dicts/objects; the FFI codec is MessagePack or JSON by caller flag (SPEC-008).
- **FLT-002** Keys beginning with `_` are reserved (`_text` — EMB-022). Nested objects/arrays are stored verbatim and returned intact; **filtering** in v1 addresses only top-level keys (nested paths like `a.b` are post-1.0, PRD P2).
- **FLT-003** Payload is replaced wholesale on upsert (no partial merge in v1).

## 2. Filter model

```rust
pub struct Filter {           // all three clauses optional; empty filter matches everything
    pub must: Vec<Condition>,      // AND
    pub should: Vec<Condition>,    // OR  (≥ 1 must hold if the clause is non-empty)
    pub must_not: Vec<Condition>,  // NAND (none may hold)
}
pub enum Condition {
    Eq     { key: String, value: MatchValue },          // keyword / bool / integer exact
    In     { key: String, values: Vec<MatchValue> },
    Range  { key: String, gt/gte/lt/lte: Option<f64> }, // numeric
    Exists { key: String },
    Nested(Box<Filter>),                                 // boolean composition of clauses
}
```

- **FLT-010** Combination semantics (server parity): a point matches iff **all** `must` hold AND (**any** `should` holds, when `should` is non-empty) AND **no** `must_not` holds. `Condition::Nested` allows arbitrary boolean trees; JSON shape matches the server's Qdrant-style filters so filter documents are portable (SPEC-013).
- **FLT-011** Type semantics: `Eq` on strings is exact (case-sensitive); on numbers, integer-vs-float equality follows JSON number equality; on booleans, exact. `Range` applies to numeric values only; a non-numeric stored value simply doesn't match (never an error). Missing key ⇒ condition doesn't match (except `Exists`, which is precisely the missing-key test; `Exists` matches JSON `null` values — key presence, not truthiness).
- **FLT-012** Geo conditions and nested-path keys are **not** in v1: parsing a filter document containing them → `InvalidArgument` naming the unsupported feature (never silently ignored).

## 3. Payload indexes

- **FLT-020** Kinds: `Keyword` (string exact-match), `Integer` (i64), `Float` (f64). Declared at collection creation (`CollectionOptions::payload_index`) or later (`create_payload_index`, journaled as `PIDX_DECLARE` — WAL op 8). Late creation builds the index by scanning existing payloads.
- **FLT-021** Index structure: `value → roaring bitmap of slots` (keyword: hashed dictionary; int/float: sorted value list for range scans). Persisted as PIDX segments (STG §3.1); rebuilt incrementally in memory from WAL replay.
- **FLT-022** Indexes are an **accelerator, not a gate**: filtering on an unindexed key MUST work via payload scan (slower, correct). Results MUST be identical with and without the index — property-tested.

## 4. Filtered search execution

- **FLT-030** Strategy (adapted from the server's `filter_processor`): estimate filter selectivity from indexes; **pre-filter** (build the candidate bitmap, then HNSW search restricted to it) when the candidate set is small; otherwise **post-filter** (HNSW over-fetch, then apply the filter) with adaptive over-fetch growth until `limit` results or candidates are exhausted.
- **FLT-031** The choice of strategy MUST NOT affect the result set for a deterministic corpus (only latency). Conformance fixes the expected results, not the strategy.
- **FLT-032** `scroll` accepts the same `Filter` (filtered pagination).

## 5. Acceptance criteria (gate G3)

1. Semantics corpus: a table of (payload set, filter document, expected ids) shared with the server repo — identical outcomes in both engines (FR-73).
2. Index/scan equivalence property test (FLT-022).
3. Unsupported-feature filters (geo, nested paths) rejected with `InvalidArgument` (FLT-012).
4. Filtered-search recall: with selective filters (< 1 % matches), filtered top-k equals brute-force filtered top-k on the test corpus (pre-filter path correctness).
5. Reserved-key enforcement (`_`-prefix) unit tests.
