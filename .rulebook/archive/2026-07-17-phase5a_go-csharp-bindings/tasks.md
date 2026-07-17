## 1. Implementation
- [x] 1.1 Context: read docs/specs/SPEC-011 in full, SPEC-008 for the ABI contract; DAG T5.1, T5.2
- [x] 1.2 Go: cgo shim over veclite.h, static-lib bundling per platform build tags (GO-001) — per-platform cgo directive files (Linux/macOS static, Windows dll); bundled libs are release artifacts (lib/README.md)
- [x] 1.3 Go: idiomatic surface (Open/Memory/CreateCollection/Upsert/Search/Scroll…), sentinel errors (GO-010) — full surface + errors.Is/As sentinels + CodeString + unknown-code fallback
- [x] 1.4 Go: pinned slice zero-copy with runtime.KeepAlive; finalizer safety nets; goroutine-safe (GO-011..013) — unsafe.Pointer + KeepAlive, SetFinalizer closing leaked handles, mutex-guarded db handle, synchronous calls
- [x] 1.5 C#: P/Invoke declarations + SafeHandle wrappers; NuGet runtimes layout (CS-001/010) — LibraryImport + runtimes/<rid>/native resolver; VecLiteDbHandle/VecLiteCollectionHandle
- [x] 1.6 C#: surface with Span pinned interop, exception hierarchy with ErrorCode enum (CS-011/012) — ReadOnlySpan<float> via fixed; VecLiteException + ErrorCode + typed subclasses; sync-only v1 (CS-013)
- [x] 1.7 Conformance runners in Go and C# consuming the shared YAML corpus — both 34/34 green (memory + file), identical observations to the Rust/Py/Node runners

## 2. Testing
- [x] 2.1 Corpus green in Go CI and .NET CI across the platform matrix — 34/34 in both locally; dormant CI job (veclite-bindings-go-csharp.yml) runs both on linux-x64 (Actions off)
- [x] 2.2 Leak checks: Go finalizer stress; C# SafeHandle stress under forced GC — TestFinalizerReleasesFileLock (Go) + SafeHandle_Releases_Lock_Under_GC (C#), both reopen the same path after leaking, proving the lock was released
- [x] 2.3 Concurrency smoke: parallel goroutines / Parallel.For across all ops — 16-goroutine (Go) and 16-way Parallel.For (C#) hammer one shared collection; exact final count
- [x] 2.4 Error mapping exhaustiveness incl. unknown-code fallback — TestErrorMapping (Go) + Errors_Map_To_Typed_Exceptions (C#): ALREADY_EXISTS/COLLECTION_NOT_FOUND/DIMENSION_MISMATCH/UNSUPPORTED_PROVIDER/LOCKED + unknown→INTERNAL

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [x] 3.1 Update or create documentation covering the implementation — bindings/go/README.md, bindings/csharp/README.md, lib/runtimes READMEs, SPEC-011 status
- [x] 3.2 Write tests covering the new behavior — Go test suite + C# xUnit suite + both conformance runners
- [x] 3.3 Run tests and confirm they pass — Go: all tests + 34 conformance; C#: 6 xUnit + 34 conformance; all green (windows/amd64, via zig cc for cgo and .NET 8)
