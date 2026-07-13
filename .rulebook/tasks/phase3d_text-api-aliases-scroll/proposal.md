# Proposal: phase3d_text-api-aliases-scroll

## Why
DAG T3.8–T3.10 close gate G3: the text-first API (upsert_text/search_text) makes the quickstart real, and aliases/scroll/search_batch/stats/info complete the API surface that freezes next phase (FR-08, FR-12, FR-13, FR-25, FR-35, FR-36, FR-47).

## What Changes
- upsert_text/upsert_texts + search_text on auto-embed collections; _text stored in PAYLOAD (EMB-020..022)
- Chunker utility port from file_loader/chunker.rs: UTF-8-safe, sentence/word boundaries, deterministic (EMB-050/051)
- Aliases create/delete/resolve (CORE-011, API §2); scroll with stable slot ordering (API-022); search_batch rayon-parallel (FR-35)
- stats() and info() structs (FR-08, FR-13)
- Gate G3: filter + hybrid + text conformance vs server on the shared corpus

## Impact
- Affected specs: SPEC-004 §2/§4, SPEC-005 §4/§7
- Affected code: crates/veclite/src/{collection,chunk}.rs, alias registry
- Breaking change: NO
- User benefit: the README 5-line quickstart works end to end; API complete for the phase-4 freeze
