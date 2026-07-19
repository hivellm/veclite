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

- [x] 2.1 README: the auto-embed bullet now says the sparse providers are lexical and points question-style retrieval at the dense `onnx` tier; the headline example's query changed to a keyword phrasing that actually suits `bm25`, with a note explaining the choice. Example re-run to confirm it works as printed
- [x] 2.2 CHANGELOG: Fixed entries for both defects (flagging that `search_text` no longer raises) plus Added entries for `CollectionOptions::metric()` and `CollectionStats::metric`
- [x] 2.3 SPEC-004 API-023/API-024 and SPEC-005 EMB-020/EMB-024 state the zero-vector outcome and the metric-with-provider rule outright

## 3. Open question (decide before 1.6)

- [x] 3.1 Resolved — not a defect. Verified empirically: f32 vectors are the source of truth (`reindex`/`refit` need them) and sq-8 is a separate code block built only by `reindex`; `inspect` reports the *configured* quantization, which is accurate. The 57KB->111KB growth after reindex is the append-only segment design, reclaimed by `vacuum`. Possible follow-up, not opened: `inspect` could say whether the code block is actually present

## 4. Tail (docs + tests — check or waive with tailWaiver)

- [x] 4.1 Update or create documentation covering the implementation — README, CHANGELOG, SPEC-004, SPEC-005
- [x] 4.2 Write tests covering the new behavior — Rust (auto_embed: OOV text, OOV hybrid, explicit-zero guard retained), FFI (metric across three create shapes), Python (metric + OOV + limit=0), Node (metric + OOV)
- [x] 4.3 Run tests and confirm they pass — fmt/clippy -D warnings clean, 40 suites green, api-freeze PASS (additive only), node + python conformance 34/34 against rebuilt artefacts
