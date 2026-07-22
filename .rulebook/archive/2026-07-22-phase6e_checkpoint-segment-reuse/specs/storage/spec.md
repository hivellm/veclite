# Checkpoint Segment Reuse Specification

## ADDED Requirements

### Requirement: A Checkpoint Without Mutations Does Not Grow the File
A checkpoint SHALL reference a collection's already-committed segments in place when the collection has not been mutated since it was last sealed, for every storage tier — not only when an mmap base is present. Closing a database, which checkpoints, MUST therefore leave the file byte-length unchanged when nothing was written (SPEC-002 STG-070).

#### Scenario: Repeated no-op checkpoints
Given a database with one ordinary in-memory-tier collection holding 500 vectors
When `checkpoint()` is called ten times with no writes in between
Then the file size after the tenth checkpoint equals the size after the first

#### Scenario: Open and close without writing
Given an existing database file of a known size
When it is opened and closed five times with no writes
Then the file size is unchanged from the starting size

#### Scenario: The mmap tier does not regress
Given a collection large enough to load on the mmap tier
When it is checkpointed without mutations
Then it reuses its committed segments exactly as it does today

### Requirement: Loading Does Not Dirty a Collection
Reading a collection's points from disk into memory MUST NOT mark it dirty: at that instant the in-memory state and the committed state are identical, so a checkpoint that follows has nothing to reseal (SPEC-002 STG-070).

#### Scenario: A freshly loaded collection is clean
Given a database reopened from an existing file
When it is checkpointed before any write
Then no collection is resealed and the file does not grow

### Requirement: Recovery Is Unaffected by Reuse
Reusing committed segments across a checkpoint MUST NOT weaken crash recovery: the committed generation MUST remain self-consistent, and a kill-9 at any point MUST still recover to a prefix of acked writes (SPEC-003).

#### Scenario: Crash suite after reuse
Given the crash suite run against a build that reuses segments
When the randomized workloads and kill-9 harness complete
Then recovery matches the oracle exactly, as it does today
