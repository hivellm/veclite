# Testing Specification

## ADDED Requirements

### Requirement: Fuzz-Clean Parsers
All untrusted-input parsers (file header, TOC, segments, WAL replay, filter documents, option decoding) SHALL accumulate 72 hours of cargo-fuzz time with zero crashes before 1.0, with the corpus committed for regression (SPEC-015 TST-050).

#### Scenario: Malformed file never crashes
Given arbitrary bytes presented as a .veclite file
When open is attempted
Then the library returns a typed error and never panics, leaks, or exhibits undefined behavior

### Requirement: Sustained-Operation Stability
A 24 hour soak of continuous write, search, vacuum, and snapshot operations MUST complete with zero errors and a plateaued memory footprint, including runs against mmap datasets four times larger than RAM (SPEC-015 TST-051).

#### Scenario: No leak over 24 hours
Given the soak harness running the full operation mix
When 24 hours elapse
Then invariant checks all pass and process RSS shows no monotonic growth trend
