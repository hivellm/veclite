# Search Specification

## ADDED Requirements

### Requirement: Server-Parity Filter Semantics
The engine SHALL evaluate must/should/must_not filters with Eq, In, Range, and Exists conditions using semantics identical to the Vectorizer server, and MUST reject filter documents containing unsupported geo or nested-path features with InvalidArgument rather than ignoring them (SPEC-006 FLT-010..012).

#### Scenario: Combined clause evaluation
Given points with payloads {lang: en, year: 2024} and {lang: pt, year: 2020}
When a search filters must [lang eq en, year range gte 2021]
Then only points satisfying every must condition are returned

#### Scenario: Unsupported filter rejected
Given a filter document containing a geo_radius condition
When the query is executed
Then the call fails with InvalidArgument naming the unsupported feature

### Requirement: Indexes Accelerate Without Gating
Payload indexes MUST act as accelerators only: filtering on an unindexed key SHALL work via payload scan and return results identical to the indexed path (SPEC-006 FLT-022).

#### Scenario: Index and scan agree
Given the same data indexed and unindexed on key lang
When the same filtered search runs against both collections
Then the result sets are identical
