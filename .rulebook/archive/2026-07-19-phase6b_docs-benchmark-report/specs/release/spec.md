# Release Specification

## ADDED Requirements

### Requirement: CI-Executed Documentation
Every code sample on the docs site SHALL be extracted and executed in CI, and the six language quickstarts MUST run green on the supported platform matrix before 1.0 (SPEC-016 REL-040/041, PRD §9.8).

#### Scenario: Stale sample fails the build
Given a docs code sample referencing an API that a PR changes incompatibly
When CI runs the sample-runner
Then the build fails pointing at the stale sample

### Requirement: Reproducible Benchmark Report
The 1.0 benchmark report MUST compare VecLite against sqlite-vec, LanceDB embedded, Chroma embedded, and the Vectorizer server using a published harness with pinned datasets and disclosed hardware, publishing unfavorable results alongside favorable ones (SPEC-015 TST-042).

#### Scenario: Third party reproduces results
Given the published harness, datasets, and hardware profile
When an external party reruns the benchmarks on equivalent hardware
Then results fall within the report's stated variance bounds
