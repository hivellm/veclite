## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-015 §2, SPEC-002 §9; DAG T2.10 and gate G2
- [ ] 1.2 Fault-injection VFS shim: kill/truncate/reorder after N bytes written (sweep N)
- [ ] 1.3 kill-9 harness: randomized workload driver + supervisor, all three durability modes (TST-010)
- [ ] 1.4 Bit-flip drills per segment type and WAL; assert Corrupt naming the segment (TST-012)
- [ ] 1.5 Model-state equivalence checker for post-recovery assertions
- [ ] 1.6 Nightly CI job on Linux/macOS/Windows (TST-013); local one-command runner (cargo xtask crash)
- [ ] 1.7 Run the 10 000-iteration suite; attach evidence to the PR
- [ ] 1.8 Freeze: mark SPEC-002 frozen-normative, commit v1 golden files to tests/compat/golden/, wire golden check into CI

## 2. Testing
- [ ] 2.1 Suite passes 10 000 iterations with zero main-file corruption (NFR-05)
- [ ] 2.2 All acked-Full commits present after every kill point
- [ ] 2.3 Golden-file compatibility check green

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation (freeze note in docs/specs/README.md)
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass (gate G2 evidence attached)
