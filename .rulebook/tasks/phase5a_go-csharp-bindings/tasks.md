## 1. Implementation
- [ ] 1.1 Context: read docs/specs/SPEC-011 in full, SPEC-008 for the ABI contract; DAG T5.1, T5.2
- [ ] 1.2 Go: cgo shim over veclite.h, static-lib bundling per platform build tags (GO-001)
- [ ] 1.3 Go: idiomatic surface (Open/Memory/CreateCollection/Upsert/Search/Scroll), sentinel errors (GO-010)
- [ ] 1.4 Go: pinned slice zero-copy with runtime.KeepAlive; finalizer safety nets; goroutine-safe (GO-011..013)
- [ ] 1.5 C#: P/Invoke declarations + SafeHandle wrappers; NuGet runtimes layout (CS-001/010)
- [ ] 1.6 C#: surface with Span pinned interop, exception hierarchy with ErrorCode enum (CS-011/012)
- [ ] 1.7 Conformance runners in Go and C# consuming the shared YAML corpus

## 2. Testing
- [ ] 2.1 Corpus green in Go CI and .NET CI across the platform matrix
- [ ] 2.2 Leak checks: Go finalizer stress + valgrind shim; C# SafeHandle stress under forced GC
- [ ] 2.3 Concurrency smoke: parallel goroutines / Parallel.For across all ops
- [ ] 2.4 Error mapping exhaustiveness incl. unknown-code fallback

## 3. Tail (mandatory — enforced by rulebook v5.3.0)
- [ ] 3.1 Update or create documentation covering the implementation
- [ ] 3.2 Write tests covering the new behavior
- [ ] 3.3 Run tests and confirm they pass
