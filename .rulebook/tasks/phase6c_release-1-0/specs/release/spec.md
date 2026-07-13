# Release Specification

## ADDED Requirements

### Requirement: Atomic Lockstep Release
Version 1.0.0 SHALL publish the core and every binding with one version number through a single all-or-nothing workflow — a failed publish leg aborts the entire release — followed by post-publish smoke installs in every ecosystem (SPEC-016 REL-012/030, PRD NFR-12).

#### Scenario: Failed leg aborts release
Given the release workflow publishing to all channels
When the NuGet publish step fails
Then no channel ends up with a partially released 1.0.0 and the tag is rolled back or re-run cleanly

### Requirement: Go/No-Go Checklist Enforcement
The 1.0.0 release MUST verify every PRD section 9 criterion with linked evidence in the release PR using the committed release template, including a crash-suite run within the previous 7 days (SPEC-016 §6).

#### Scenario: Stale crash evidence blocks release
Given a release PR whose latest crash-suite evidence is 10 days old
When the go/no-go checklist is reviewed
Then the release is blocked until a fresh 10 000-iteration run passes
