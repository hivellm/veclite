## 1. Implementation

Order matters: 1.1 pins the current behaviour with failing tests so the fixes are
proven, and 1.6 is the only item that changes the core rather than a binding.

- [ ] 1.1 Context: read the proposal, SPEC-004 (search_text semantics), SPEC-005 (embedding providers), and `CollectionOptions::auto_embed` in `crates/veclite/src/options.rs`
- [ ] 1.2 Regression tests that fail today: (a) a collection created with `metric=euclidean` + an embedding provider reports `euclidean`, asserted per binding; (b) `search_text` with an out-of-vocabulary query returns an empty result set
- [ ] 1.3 `crates/veclite-py/src/lib.rs` — build `CollectionOptions` from the requested metric, then attach `embedding_provider`, instead of `CollectionOptions::auto_embed`
- [ ] 1.4 `crates/veclite-ffi/src/lib.rs` — same fix; this is the one the Go and C# bindings inherit, so verify through them too
- [ ] 1.5 `crates/veclite-node/src/lib.rs` — same fix
- [ ] 1.6 `crates/veclite/src/collection.rs` — `search_text` (and the hybrid text lane) returns an empty result set when the embedded query is the zero vector. Leave the guard in `search()` untouched: an explicitly all-zero query with cosine stays `InvalidArgument`
- [ ] 1.7 Confirm the two defects are gone against the *published* artefacts as well, not just the dev tree — a fresh venv/npm install reproduced both originally

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
