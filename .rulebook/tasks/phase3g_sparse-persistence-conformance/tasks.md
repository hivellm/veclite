## 1. Implementation
- [ ] 1.1 SPARSE segment persistence: seal::seal/seal::load carry the sparse lane; survives checkpoint+reopen (HYB-030)
- [ ] 1.2 Tombstone-aware SPARSE rewrite in vacuum/compact (HYB-031)
- [ ] 1.3 HybridQuery::text(&str) on auto-embed collections — dense embed + provider-derived sparse weights (HYB-011)

## 2. Testing
- [ ] 2.1 Reopen: BYO sparse + hybrid results identical after checkpoint+reopen
- [ ] 2.2 Crash-recovery: sparse index after kill-9 + replay equals rebuilt-from-scratch (acceptance 4)
- [ ] 2.3 Server conformance corpus: fused RRF rankings identical to the server (HYB-022, acceptance 1)

## 3. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
