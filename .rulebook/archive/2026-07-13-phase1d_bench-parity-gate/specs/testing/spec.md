# Testing Specification

## ADDED Requirements

### Requirement: Performance Benchmarks
The project SHALL maintain criterion benchmarks with pinned reference hardware profiles proving p50 search under 3 ms for 1M x 512-dim SQ-8 warm and index build within 2x of the Vectorizer server single-node time (SPEC-015 TST-040/041, PRD NFR-01..03).

#### Scenario: Bench regression fence
Given the per-PR smoke benchmark baseline
When a PR degrades search p50 by more than 20 percent
Then CI fails the benchmark job

### Requirement: Server Parity Harness
The project MUST prove top-10 result overlap of at least 0.99 against a pinned Vectorizer server version on the standard benchmark corpus with identical collection configs (SPEC-015 TST-030, PRD NFR-04).

#### Scenario: Gate G1 parity check
Given the standard corpus loaded into VecLite and the pinned Vectorizer server with identical configs
When the standard query set runs on both
Then top-10 overlap is at least 0.99 for every query set aggregate
