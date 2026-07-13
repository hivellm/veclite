## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-016 §4/§6, docs/PRD.md §9; DAG T6.5 and gate G6
- [ ] 1.2 Publish format stability pledge + SemVer/lockstep policy on the docs site (REL-030/031)
- [ ] 1.3 .github/RELEASE_TEMPLATE.md with the PRD §9 checklist
- [ ] 1.4 Verify every PRD §9 criterion with linked evidence (gates, crash suite <= 7 days old, conformance, overlap, fuzz/soak, budgets, clean installs)
- [ ] 1.5 Release dry-run: TestPyPI, npm dist-tag next, NuGet test feed
- [ ] 1.6 Version stamp 1.0.0 across core + all bindings (lockstep); CHANGELOG finalized
- [ ] 1.7 Tag and run the atomic multi-channel publish (REL-012)
- [ ] 1.8 Post-publish smoke installs in every ecosystem

## 2. Testing
- [ ] 2.1 Dry-run publish verified end to end before the real one
- [ ] 2.2 Post-publish quickstart smoke from public indexes (pip/npm/cargo/go/dotnet)

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass (G6 checklist archived in the release PR)
