## 1. Implementation

Order matters: 1.1 pins the current behaviour with failing tests so the fixes are
proven, and 1.6 is the only item that changes the core rather than a binding.

- [x] 1.1 Context: read the proposal, SPEC-004 (search_text semantics), SPEC-005 (embedding providers), and `CollectionOptions::auto_embed` in `crates/veclite/src/options.rs` — confirmed both behaviours are *unspecified*: `auto_embed` merely picks `Metric::default()` (metric is a plain public field, no correctness constraint), and no spec states the zero-vector outcome for the text path. That is why both fell through.
- [x] 1.2 Regression tests that fail today: (a) a collection created with `metric=euclidean` + an embedding provider reports `euclidean`, asserted per binding; (b) `search_text` with an out-of-vocabulary query returns an empty result set — both written and confirmed failing before the fixes
- [x] 1.3 `crates/veclite-py/src/lib.rs` — fixed via the new `CollectionOptions::metric()` builder; `stats()` now reports `metric`
- [x] 1.4 `crates/veclite-ffi/src/lib.rs` — same fix; `metric` added to both stats payloads (collection stats and db_info)
- [x] 1.5 `crates/veclite-node/src/lib.rs` — same fix; `stats().metric` exposed, index.d.ts regenerated
- [x] 1.6 `crates/veclite/src/collection.rs` + `hybrid.rs` — `search_text` and the hybrid text lane return an empty result set on a zero-vector embedding. The hybrid lane had the same defect (found by testing, not assumed). Short-circuit guarded on `limit > 0` so an invalid limit still reaches validation — the first attempt masked it and broke `surface_edges`
- [x] 1.7 Confirmed against built artefacts: rebuilt wheel + rebuilt linux-x64 node addon, both fixes verified through each binding; node conformance 34/34, python conformance 34/34

## 2. Documentation

- [ ] 2.1 README: note that `bm25` is lexical and that natural-language questions want the dense `onnx` tier — the flagship example pairs a natural-language question with the lexical default. Measured on 48 files of `docs/`: 3/10 at rank 1 for questions, 4/10 (8/10 within the top three files) for keyword phrasing
- [ ] 2.2 CHANGELOG entries for both fixes, flagging that `search_text` stops raising on an out-of-vocabulary query
- [ ] 2.3 SPEC-004/SPEC-005: state the zero-vector outcome for the text path explicitly, so the next reader does not have to infer it from an error message

## 3. Open question (decide before 1.6)

- [ ] 3.1 The dogfooding run produced a `.veclite` of 4.3 MB for 279 KB of text. The vectors segment is 993,333 bytes and 485 chunks x 512 dims x 4 bytes = 993,280 — i.e. f32 on disk, while `veclite inspect` reports `quantization sq-8`. Determine whether sq-8 applies only to the index or whether the report is misleading; open a separate task if it is a real inconsistency

## 4. Tail (docs + tests — check or waive with tailWaiver)

- [ ] 4.1 Update or create documentation covering the implementation
- [ ] 4.2 Write tests covering the new behavior
- [ ] 4.3 Run tests and confirm they pass
