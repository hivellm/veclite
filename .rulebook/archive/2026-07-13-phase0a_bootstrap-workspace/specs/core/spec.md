# Core Specification

## ADDED Requirements

### Requirement: Workspace Bootstrap
The project SHALL provide a cargo workspace whose default build depends only on vectorizer-core (pinned minor "3.5") plus pure-Rust crates, compiles on Linux/macOS/Windows and wasm32-unknown-unknown, and enforces clippy -D warnings with unwrap_used/expect_used denied (SPEC-001 CORE-001..004, SPEC-016 REL-010).

#### Scenario: Clean checkout builds everywhere
Given a clean checkout with no prior build cache
When CI runs fmt, clippy, and cargo test on Linux, macOS, and Windows plus a wasm32-unknown-unknown build check
Then all jobs pass and the default-build dependency tree contains no network crates

### Requirement: Server-Parity Defaults
CollectionOptions and OpenOptions MUST default to Metric::Cosine, hnsw m=16 / ef_construction=200 / ef_search=100, Quantization::Scalar bits=8, LZ4 compression threshold 1024 B, and Durability::Normal (SPEC-004 §3).

#### Scenario: Defaults pinned by tests
Given a CollectionOptions::new(384, Metric::Cosine) with no further tuning
When the resolved config is inspected in a unit test
Then every field equals the SPEC-004 §3 defaults table value
