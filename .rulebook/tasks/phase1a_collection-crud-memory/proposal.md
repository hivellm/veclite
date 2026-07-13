# Proposal: phase1a_collection-crud-memory

## Why
DAG T1.1 + T1.5: the engine needs the collection registry and vector CRUD before any indexing or search exists, plus the in-memory mode that tests and agent workloads depend on (FR-02, FR-10, FR-20–23).

## What Changes
- Collection registry (DashMap) with create/get/delete/rename; name/id validation (CORE-010/011, CORE-020..022)
- Vector CRUD: upsert/upsert_batch/get/delete/delete_batch/len with dimension and NaN/Inf rejection (CORE-012/013), Cosine ingest normalization (CORE-014)
- Point/Hit/SparseVector data model structs
- VecLite::memory() — identical API, no file, no WAL

## Impact
- Affected specs: SPEC-001 §3–4, SPEC-004 §1/§4
- Affected code: crates/veclite/src/{database,collection,registry}.rs
- Breaking change: NO
- User benefit: working CRUD engine; property tests guarantee model-state equivalence
