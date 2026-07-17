# Proposal: phase5c_onnx-feature

## Why
DAG T5.4: users who want real dense neural embeddings without bringing their own vectors need the opt-in onnx tier — kept strictly out of the default build so the base install stays lean (FR-46, EMB-040..042).

## What Changes
- onnx cargo feature pulling fastembed 5.x (ONNX Runtime) — never in default (API-050)
- fastembed:<model> provider with model download to OpenOptions::model_cache_dir; fastembed:path:<dir> fully offline for air-gapped use (EMB-041)
- Graceful degradation: onnx collections open on non-onnx builds, vector ops work, text ops fail with UnsupportedProvider (EMB-023)
- Heavy distribution artifacts: veclite-onnx wheel, @veclite/onnx, VecLite.Onnx, Go build tag veclite_onnx — base packages never depend on them (EMB-040, REL-021)
- wasm32 exclusion unconditional (EMB-042)

## Impact
- Affected specs: SPEC-005 §6, SPEC-016 §3
- Affected code: crates/veclite/src/embedding/fastembed.rs (feature-gated), packaging pipelines
- Breaking change: NO
- User benefit: opt-in MiniLM-class dense embeddings; the only permitted network access in the product, explicit and redirectable offline
