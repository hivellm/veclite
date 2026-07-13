# Core Specification

## ADDED Requirements

### Requirement: Collection Registry and Vector CRUD
The engine SHALL manage named collections with create/get/delete/rename and per-vector upsert/get/delete operations, rejecting wrong-dimension and non-finite vectors with typed errors and never silently coercing input (SPEC-001 CORE-010..022).

#### Scenario: Dimension mismatch rejected
Given a collection created with dimension 384
When a vector of length 100 is upserted
Then the call fails with DimensionMismatch expected=384 got=100 and the collection state is unchanged

### Requirement: In-Memory Mode
VecLite::memory() MUST provide the identical API surface as file-backed databases with no file and no WAL (FR-02).

#### Scenario: Memory database behaves like file database
Given an in-memory database and a file-backed database receiving the same operation sequence
When the same searches are executed on both
Then the results are identical
