# Proposal: phase5a_go-csharp-bindings

## Why
DAG T5.1 + T5.2: Go and C# extend reach to backend and enterprise ecosystems over the frozen C ABI — both consume veclite-ffi, never the Rust crate directly (FR-63).

## What Changes
- bindings/go: cgo wrapper, bundled static libs per platform via build tags, sentinel errors with errors.Is/As, pinned []float32 zero-copy, finalizer safety nets, MessagePack codec (GO-001..013)
- bindings/csharp: NuGet package with runtimes/<rid>/native/ assets, SafeHandle wrappers, ReadOnlySpan<float> pinned interop, VecLiteException hierarchy, sync-only v1 (CS-001..013)
- Conformance corpus runners in both languages, wired into CI

## Impact
- Affected specs: SPEC-011 (all)
- Affected code: bindings/go/, bindings/csharp/, CI conformance jobs
- Breaking change: NO
- User benefit: go get / dotnet add package with no Rust toolchain; same behavior as every other binding
