# Bindings Specification

## ADDED Requirements

### Requirement: WASM File Interchange
The WASM package SHALL serialize databases as valid .veclite v1 file images readable by native VecLite and deserialize native-written files, with no WASM-specific dialect (SPEC-012 WASM-010).

#### Scenario: Round-trip with native
Given a database written by native VecLite on disk
When its bytes are loaded with deserialize in a browser and the same queries run
Then results are identical to the native results, and a serialize output from the browser opens natively

### Requirement: OPFS Persistence with Bounded Loss
The OPFS backend MUST operate on an in-memory image with atomic save via temp-write-then-move, so a crash between autosaves loses at most the writes since the last save and never corrupts the stored image (SPEC-012 WASM-011).

#### Scenario: Crash between autosaves
Given an OPFS-backed database with autosave every 100 writes
When the page crashes after 150 writes since the last save completed at write 100
Then reopening loads the state as of write 100 with the stored image fully valid

### Requirement: Bundle Size Budget
The gzipped wasm bundle including default providers MUST stay at or under 3 MB, enforced by CI (SPEC-012 WASM-030).

#### Scenario: Budget regression blocked
Given the CI bundle-size check
When a change pushes the gzipped bundle above 3 MB
Then the CI job fails
