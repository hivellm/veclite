# Core Specification

## ADDED Requirements

### Requirement: HNSW Index with Soft Deletes
The engine SHALL provide HNSW k-NN search via pinned hnsw_rs 0.3 with soft-delete tombstones excluded from results, over-fetching internally so that limit live results are returned whenever enough live vectors exist, and reindex() SHALL rebuild the graph purging tombstones (SPEC-001 CORE-030..035).

#### Scenario: Deleted vectors never surface
Given a collection with 1000 vectors of which 500 are deleted
When a search with limit 10 runs
Then 10 results are returned and none is a deleted id

### Requirement: SQ-8 Quantization Default
Collections MUST default to scalar quantization with 8 bits using vectorizer-core encodings, achieving top-10 recall of at least 0.99 versus unquantized search on the standard corpus (SPEC-001 CORE-040..043).

#### Scenario: Quantized recall gate
Given the standard benchmark corpus indexed with default options
When top-10 results are compared against an unquantized index for the standard query set
Then overlap is at least 0.99
