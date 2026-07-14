# Storage Specification

## ADDED Requirements

### Requirement: WAL Atomicity and Recovery
Every mutating call SHALL append exactly one WAL entry as the atomic unit; recovery MUST replay valid entries in sequence order and discard the torn tail from the first invalid entry onward, so a partially applied batch is never observable (SPEC-003 WAL-010..012, WAL-040..043).

#### Scenario: Torn tail discarded
Given a WAL whose last entry was half-written when the process died
When the database is reopened
Then all entries before the torn one are applied and the torn entry is discarded entirely

### Requirement: Durability Modes Preserve Integrity
The engine MUST support Full, Normal, and Off durability where the fsync policy trades freshness only — the main file SHALL never be corrupt in any mode, and in Full mode every acknowledged write survives an OS crash (SPEC-003 WAL-020/021).

#### Scenario: Full mode survives kill
Given a database opened with Durability::Full
When 100 upserts are acknowledged and the process is killed with SIGKILL
Then reopening recovers all 100 upserts

### Requirement: Checkpoint Truncation Ordering
Checkpoint MUST truncate the WAL only after the header-swap fsync completes, so a crash during checkpoint recovers to exactly the pre-checkpoint or post-checkpoint state (SPEC-003 WAL-032).

#### Scenario: Crash during checkpoint
Given a checkpoint in progress
When the process crashes at any point during the checkpoint sequence
Then reopening yields either the full pre-checkpoint state or the full post-checkpoint state
