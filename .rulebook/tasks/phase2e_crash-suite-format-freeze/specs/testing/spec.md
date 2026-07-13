# Testing Specification

## ADDED Requirements

### Requirement: Crash-Safety Suite
The project SHALL maintain a crash suite combining kill-9 harness, torn-write fault injection, and bit-flip drills that passes 10 000 iterations with zero main-file corruption on Linux, macOS, and Windows before gate G2 (SPEC-015 TST-010..013, PRD NFR-05).

#### Scenario: Ten thousand iterations clean
Given the crash suite configured with randomized workloads across all durability modes
When 10 000 kill-and-reopen iterations execute
Then every reopen succeeds, verify reports clean, and every acknowledged Full-durability commit is present

### Requirement: Format v1 Freeze
Passing the crash suite MUST freeze storage format v1: SPEC-002 becomes frozen-normative, v1 golden files are committed, and every subsequent CI run MUST read them successfully (PRD NFR-11).

#### Scenario: Golden files guard the freeze
Given committed v1 golden database files
When any later commit changes storage code and CI runs
Then the golden files still open and return their recorded query results
