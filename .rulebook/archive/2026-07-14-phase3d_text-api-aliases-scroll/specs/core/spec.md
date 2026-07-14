# Core Specification

## ADDED Requirements

### Requirement: Text-First API
Auto-embed collections SHALL support upsert_text and search_text performing embed plus store or embed plus search in one call, storing original text under the reserved _text payload key so refit and reindex remain possible (SPEC-005 EMB-020..022, PRD FR-36/42).

#### Scenario: Five-line quickstart
Given a fresh database and an auto-embed bm25 collection of dimension 512
When a document is upserted with upsert_text and searched with search_text
Then relevant hits return with id, score, and payload — with no network access

### Requirement: Aliases, Scroll, and Batch Search
The API MUST provide collection aliases resolving transparently in lookups, cursor-based scroll with stable ordering covering every live vector exactly once, and rayon-parallel search_batch (PRD FR-12, FR-25, FR-35).

#### Scenario: Blue-green alias swap
Given collection docs_v2 and an alias docs pointing at it
When collection("docs") is fetched and searched
Then the query executes against docs_v2 without the caller knowing the physical name
