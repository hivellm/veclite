# Proposal: phase3a_payload-filters

## Why
DAG T3.1–T3.3: real search needs metadata — payload storage, the keyword/int/float payload indexes, and the Qdrant-model filter evaluation whose semantics must match the Vectorizer server exactly (FR-24, FR-32, FR-33).

## What Changes
- Payload storage in PAYLOAD segments (MessagePack + LZ4), 16 MiB limit, reserved _-prefix keys (FLT-001..003)
- Payload indexes: Keyword/Integer/Float as value → roaring-bitmap postings; declared at creation or added later via PIDX_DECLARE WAL op (FLT-020/021)
- Filter model: must/should/must_not with Eq/In/Range/Exists + Nested composition; geo/nested-path rejected with InvalidArgument (FLT-010..012)
- Execution: selectivity-based pre-filter vs post-filter with adaptive over-fetch; identical results either way (FLT-030/031)
- Filtered scroll (FLT-032); query builder filter slot activated

## Impact
- Affected specs: SPEC-006 (all), SPEC-004 §5
- Affected code: crates/veclite/src/filter/, storage PIDX integration
- Breaking change: NO
- User benefit: filtered semantic search with server-identical semantics, fast when indexed and correct when not
