# Storage Specification

## ADDED Requirements

### Requirement: Consistent Snapshots
snapshot(path) SHALL produce a valid standalone compacted .veclite file with a new file uuid via checkpoint plus copy of immutable live segments, without blocking writers beyond the TOC-swap window (SPEC-002 STG-070).

#### Scenario: Snapshot during writes
Given a database receiving continuous upserts
When snapshot(path) is invoked concurrently
Then the snapshot file opens standalone with a consistent point-in-time state and the writers experience no failure

### Requirement: In-Place Vacuum
vacuum() MUST rewrite live data dropping tombstoned slots and shrink the file in place via tail truncation, handling the Windows unmap-truncate-remap constraint, with auto-vacuum escalating at the configured tombstone threshold (SPEC-002 STG-071/072).

#### Scenario: Space reclaimed after bulk delete
Given a collection where half the vectors were deleted
When vacuum() runs
Then the file size decreases and all remaining vectors stay searchable with identical results
