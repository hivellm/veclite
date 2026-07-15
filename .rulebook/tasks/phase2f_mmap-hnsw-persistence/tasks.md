## 1. Index strategy (prerequisite)
- [x] 1.1 ADR-0004: single-file mmap of VECTORS + exact SIMD brute-force larger-than-RAM tier; HNSW rebuilt from mmap on open (no graph persistence). Supersedes ADR-0003 (corrects its "must fork hnsw_rs" premise: hnsw_rs does mmap/serialize, but only over its own directory format, which breaks single-file). SPEC-002 STG-063 reframed + STG-064 added — behaviour change, no byte change, freeze holds.

## 2. Implementation
- [ ] 2.1 mmap read path over VECTORS segments with stride addressing; auto threshold 64 MiB (OpenOptions::mmap, STG-004)
- [ ] 2.2 Larger-than-RAM tier (STG-063 reframed + STG-064, ADR-0004): above a memory budget, skip the HNSW build and serve exact SIMD brute-force k-NN over the mmap'd VECTORS; below it, rebuild HNSW in RAM from the mmap on open. No graph persistence; OpenOptions warning callback retained but unused in v1.
- [ ] 2.3 vacuum with an active mmap: unmap→truncate→remap so a mapped region can be shrunk without invalidating readers (STG-071; the no-mmap swap shipped in phase2d)

## 3. Testing
- [ ] 3.1 Larger-than-RAM smoke: dataset several times available RAM opens and serves searches via mmap (was phase2c 2.1)
- [ ] 3.2 Corrupt-HNSW fixture: open rebuilds graph, warning fired, search results correct (was phase2c 2.3)
- [ ] 3.3 Windows vacuum with an active mmap passes on CI (was phase2d 2.3)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation
- [ ] 4.2 Write tests covering the new behavior
- [ ] 4.3 Run tests and confirm they pass
