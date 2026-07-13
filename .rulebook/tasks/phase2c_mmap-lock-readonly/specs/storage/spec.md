# Storage Specification

## ADDED Requirements

### Requirement: Single-Writer File Locking
Read-write open SHALL take an exclusive advisory lock and read-only open a shared lock, with a conflicting second writer failing immediately with Locked and never blocking or corrupting (SPEC-002 STG-060).

#### Scenario: Second writer fails fast
Given a process holding a read-write handle on app.veclite
When a second process attempts a read-write open of the same file
Then the second open fails immediately with Locked

### Requirement: HNSW Rebuild Fallback
When the HNSW segment is missing or fails its checksum, open MUST fall back to rebuilding the graph from stored vectors and emit a warning callback, while any other segment corruption remains fatal (SPEC-002 STG-063).

#### Scenario: Corrupt graph segment recovered
Given a database file whose HNSW segment is corrupted
When the database is opened with a warning callback registered
Then open succeeds, the callback reports the rebuild, and subsequent searches return correct results

### Requirement: Memory-Mapped Reads
The engine SHALL serve vector reads for datasets larger than RAM through memory-mapped fixed-stride segments without decoding (SPEC-002 STG-004, PRD FR-53).

#### Scenario: Four-times-RAM dataset
Given a database file four times larger than available RAM
When the database is opened with mmap enabled and searched
Then searches complete correctly with pages faulting in on demand
