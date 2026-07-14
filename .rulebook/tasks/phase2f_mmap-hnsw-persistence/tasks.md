## 1. Index strategy (prerequisite)
- [ ] 1.1 ADR: choose the index that unblocks mmap + graph persistence — vendored/forked HNSW reading vectors from mmap, flat/IVF over mapped pages, or a maintained HNSW crate with stable serialization (supersedes the hnsw_rs constraint in ADR-0003)

## 2. Implementation
- [ ] 2.1 mmap read path over VECTORS segments with stride addressing; auto threshold 64 MiB (OpenOptions::mmap, STG-004)
- [ ] 2.2 HNSW segment load; rebuild-from-vectors fallback emitting the OpenOptions warning (STG-063)
- [ ] 2.3 vacuum with an active mmap: unmap→truncate→remap so a mapped region can be shrunk without invalidating readers (STG-071; the no-mmap swap shipped in phase2d)

## 3. Testing
- [ ] 3.1 Larger-than-RAM smoke: dataset several times available RAM opens and serves searches via mmap (was phase2c 2.1)
- [ ] 3.2 Corrupt-HNSW fixture: open rebuilds graph, warning fired, search results correct (was phase2c 2.3)
- [ ] 3.3 Windows vacuum with an active mmap passes on CI (was phase2d 2.3)

## 4. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 4.1 Update or create documentation covering the implementation
- [ ] 4.2 Write tests covering the new behavior
- [ ] 4.3 Run tests and confirm they pass
