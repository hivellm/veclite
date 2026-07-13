# Interop Specification

## ADDED Requirements

### Requirement: Graduation Round-Trip Fidelity
Export SHALL produce a .vecdb archive the Vectorizer server imports with top-10 result overlap of at least 0.99 and bm25 text scores identical within 1e-5, translating quantized vector blocks losslessly (SPEC-013 IOP-001, IOP-010, PRD NFR-04).

#### Scenario: Server serves exported data identically
Given a VecLite database with the standard benchmark corpus
When it is exported and imported into the pinned Vectorizer server and the standard queries run on both
Then top-10 overlap is at least 0.99

### Requirement: Degrading Import, Never Silent
Import of server data MUST read both Compact and Legacy layouts, drop server-only aspects with explicit warnings, refuse encrypted payloads with a clear error, and convert server-only-provider collections to BYO-vector recording origin_provider (SPEC-013 IOP-020..022).

#### Scenario: Encrypted payloads refused
Given a server .vecdb containing payload-encrypted collections
When veclite import runs
Then the import fails with a clear error naming encryption as the cause

### Requirement: CLI Contract
The veclite binary SHALL provide inspect, export, import, vacuum, snapshot, and verify with stable exit codes (0 success, 1 integrity, 2 usage, 3 environment), honoring file locks and performing no network access (SPEC-014 CLI-001..004).

#### Scenario: Verify finds corruption
Given a database file with a bit-flipped segment
When veclite verify runs
Then it exits with code 1 and prints the damaged segment offset and type
