## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-016 §5, SPEC-015 §5 (TST-042); DAG T6.3, T6.4
- [ ] 1.2 Docs site scaffolding + navigation (quickstarts, API refs, format doc, limits, migration)
- [ ] 1.3 Six quickstarts authored; sample-extraction runner executes each in CI (REL-041)
- [ ] 1.4 Migration guides: graduation and reverse paths with CLI walkthroughs
- [ ] 1.5 Benchmark harness vs sqlite-vec / LanceDB embedded / Chroma embedded / Vectorizer server (reproducible, pinned datasets)
- [ ] 1.6 Publish benchmark report with hardware disclosure; include losses (honesty rule)

## 2. Testing
- [ ] 2.1 All six quickstarts green in CI on the platform matrix
- [ ] 2.2 Benchmark harness reruns reproduce published numbers within stated variance
- [ ] 2.3 Docs link checker green

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
