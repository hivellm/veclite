# Core Specification

## ADDED Requirements

### Requirement: Search and Query Builder
The engine SHALL expose search(vector, limit) and a query() builder with limit, per-query ef_search override, with_payload defaulting to true, and with_vector defaulting to false, returning hits ordered descending for Cosine/DotProduct similarity and ascending for Euclidean distance (SPEC-004 §5, SPEC-001 CORE-035).

#### Scenario: Per-query ef_search override
Given a collection with default ef_search 100
When a query runs with ef_search 200
Then the query uses 200 and the collection default remains 100 for subsequent queries

#### Scenario: Zero limit rejected
Given any collection
When a query runs with limit 0
Then the call fails with InvalidArgument
