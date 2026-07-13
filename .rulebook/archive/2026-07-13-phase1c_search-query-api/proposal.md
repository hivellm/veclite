# Proposal: phase1c_search-query-api

## Why
DAG T1.4: with CRUD and HNSW in place, the public search surface must exist — plain k-NN plus the query builder that every binding will mirror. This is the API users touch most (FR-30, FR-31).

## What Changes
- Collection::search(vector, limit) returning Vec<Hit> ordered per CORE-035 (desc similarity / asc distance)
- query() builder: limit, ef_search per-query override, with_payload (default true), with_vector (default false); filter slot stubbed until phase3a
- limit=0 rejected as InvalidArgument; limit > live count returns all live (API-031)
- Builders hold no locks until run() (API-030)

## Impact
- Affected specs: SPEC-004 §4–5, SPEC-001 CORE-035
- Affected code: crates/veclite/src/{collection,query}.rs
- Breaking change: NO
- User benefit: the 5-line quickstart search path works end to end in memory
