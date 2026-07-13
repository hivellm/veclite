# Storage Specification

## ADDED Requirements

### Requirement: Root-Pointer Commit Protocol
The storage layer SHALL commit checkpoints by appending segments, fsyncing, appending a new TOC, fsyncing, then atomically rewriting the 4 KiB header to point at the new TOC and fsyncing, so a crash between any two steps leaves the previous header-to-TOC chain valid (SPEC-002 STG-050).

#### Scenario: Crash mid-commit preserves committed state
Given a database with a committed TOC generation N
When the process crashes after writing segments for generation N+1 but before the header swap completes
Then reopening loads generation N with all its data intact

### Requirement: Checksummed Immutable Segments
Every segment MUST carry a crc32 verified before use, VECTORS segments MUST be uncompressed fixed-stride blocks addressable without decoding, and readers MUST fail with Corrupt naming the segment offset and type on checksum mismatch (SPEC-002 STG-020/021/030/031).

#### Scenario: Bit flip detected
Given a valid database file
When a random bit inside a PAYLOAD segment body is flipped and the file is opened
Then open fails with Corrupt identifying the damaged segment and no undefined behavior occurs
