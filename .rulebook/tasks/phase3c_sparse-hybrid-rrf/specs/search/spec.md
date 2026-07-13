# Search Specification

## ADDED Requirements

### Requirement: Deterministic RRF Hybrid Fusion
Hybrid search SHALL fuse dense and sparse lanes with reciprocal rank fusion using alpha default 0.5 and rrf_k default 60, breaking ties by dense rank then bytewise id, producing rankings identical to the Vectorizer server on the shared corpus (SPEC-007 HYB-020..022).

#### Scenario: Fused ranking matches server
Given the shared conformance corpus indexed identically in VecLite and the server
When the same hybrid queries run on both
Then the fused result orderings are identical

### Requirement: Single-Lane Degeneration
A hybrid query with only one lane provided MUST return exactly the same scores as the dedicated dense or sparse search API, and a query with no lanes MUST fail with InvalidArgument (SPEC-007 HYB-010).

#### Scenario: Dense-only hybrid equals plain search
Given a collection with dense vectors
When a hybrid query supplies only the dense lane
Then results equal collection.search for the same vector and limit
