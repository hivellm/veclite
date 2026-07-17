# SPEC-011 — Go & C# Bindings (over the C ABI)

| | |
|---|---|
| **Status** | Implemented (phase5a): `bindings/go` (cgo over veclite.h — sentinel errors + errors.Is, pinned []float32 zero-copy, SetFinalizer safety nets, goroutine-safe) and `bindings/csharp` (P/Invoke + SafeHandle wrappers, ReadOnlySpan<float> pinned interop, VecLiteException + ErrorCode enum, sync v1). Both wrap veclite-ffi only. Shared conformance corpus green in both (34 cases, memory + file); Go + .NET unit tests cover quickstart, concurrency, error mapping, and finalizer/SafeHandle leak-release. Per-platform static/dynamic library bundling is a release-CI artifact (dormant while Actions is off). |
| **Phase / tasks** | Phase 5 · T5.1, T5.2 ([DAG](../DAG.md)) |
| **PRD requirements** | FR-63, FR-65, FR-66 |
| **Planning source** | [06-sdk-bindings.md](../vectorizer-lite/06-sdk-bindings.md) |

Requirement IDs `GO-xxx` / `CS-xxx`. Both bindings wrap `veclite-ffi` (SPEC-008); they MUST NOT link the Rust crate directly. Conformance corpus (SPEC-015 §3) is the behavioral arbiter for both.

## 1. Go (`github.com/hivellm/veclite-go`)

### Packaging

- **GO-001** cgo wrapper with **bundled static libraries** per platform (FR-66 matrix) selected by build tags — `go get` + build works with a C toolchain only, no Rust. ONNX variant behind build tag `veclite_onnx` linking the heavy artifact.
- **GO-002** Module version tracks core semver (lockstep); `veclite.Version()`, `veclite.FormatVersion()`.

### API shape

```go
db, err := veclite.Open("app.veclite", &veclite.OpenOptions{ReadOnly: false})
defer db.Close()
mem := veclite.Memory()

docs, err := db.CreateCollection("docs", veclite.CollectionOptions{
    Dimension: 384, Metric: veclite.Cosine,
    HNSW: &veclite.HNSWOptions{M: 16, EfConstruction: 200, EfSearch: 100},
})
err = docs.Upsert(veclite.Point{ID: "id-1", Vector: vec, Payload: map[string]any{"lang": "en"}})
err = docs.UpsertBatch(points)
hits, err := docs.Search(query, veclite.SearchOptions{Limit: 10, Filter: f})
hits, err = docs.SearchText("query", veclite.SearchOptions{Limit: 5})
page, err := docs.Scroll(veclite.ScrollOptions{Limit: 100, OffsetID: "id-500"})
```

- **GO-010** Idioms: `error` returns everywhere; `errors.Is/As` support via sentinel errors (`veclite.ErrCollectionNotFound`, `veclite.ErrLocked`, …) mapped 1:1 from FFI codes; the FFI thread-local message becomes the error string.
- **GO-011** `[]float32` slices pass to cgo pinned, no copy (`unsafe.Pointer` on the slice data, kept alive with `runtime.KeepAlive`). Payloads/filters/options cross as MessagePack (`VL_CODEC_MSGPACK`).
- **GO-012** Handles carry `runtime.SetFinalizer` safety nets that close leaked handles; `Close` is idempotent and finalizer-safe. All types are safe for concurrent use by multiple goroutines (FFI-001).
- **GO-013** No goroutine is spawned by the binding; calls block the calling goroutine (cgo call), which is the idiomatic Go equivalent of the sync core.

## 2. C# (`VecLite` on NuGet)

### Packaging

- **CS-001** Single NuGet package with native assets under `runtimes/<rid>/native/` for the FR-66 matrix (RIDs: `linux-x64`, `linux-arm64`, `linux-musl-x64`, `osx-x64`, `osx-arm64`, `win-x64`, `win-arm64`). .NET ≥ 8. `VecLite.Onnx` is a separate package (EMB-040).
- **CS-002** Assembly version tracks core semver (lockstep).

### API shape

```csharp
using var db = VecLite.Open("app.veclite", new OpenOptions { ReadOnly = false });
using var mem = VecLite.Memory();

var docs = db.CreateCollection("docs", new CollectionOptions {
    Dimension = 384, Metric = Metric.Cosine,
    Hnsw = new HnswOptions { M = 16, EfConstruction = 200, EfSearch = 100 },
});
docs.Upsert("id-1", vector, new { lang = "en" });          // payload: object | JsonNode
docs.UpsertBatch(points);                                   // ReadOnlySpan<float> flat + dim
var hits = docs.Search(query, new SearchOptions { Limit = 10, Filter = filter });
var hits2 = docs.SearchText("query", new SearchOptions { Limit = 5 });
var page = docs.Scroll(new ScrollOptions { Limit = 100, OffsetId = "id-500" });
db.Snapshot("backup.veclite"); db.Vacuum();
```

- **CS-010** Native handles wrapped in `SafeHandle` subclasses (`VecLiteDbHandle`, `VecLiteCollectionHandle`) — correct cleanup under finalization and thread aborts; `Dispose` idempotent (`IDisposable` on `Database`; collections are lightweight views).
- **CS-011** Zero-copy vector interop: `ReadOnlySpan<float>` / `ReadOnlyMemory<float>` pinned via `fixed`/`MemoryHandle` for the call duration. Hit vectors return as `float[]` (copy — .NET arrays can't borrow native memory safely across GC) unless the caller uses the `HitView` low-level API.
- **CS-012** Exceptions: `VecLiteException : Exception` with `ErrorCode` enum mirroring FFI codes; specific subclasses for the programming-error-adjacent ones (`DimensionMismatchException`, `CollectionNotFoundException`). Messages from `vl_last_error_message`.
- **CS-013** Sync API only in v1 (the core is sync; `Task` wrappers add nothing over `Task.Run` in the caller). Documented explicitly.

## 3. Shared acceptance criteria

1. Conformance corpus green in both CIs across the platform matrix (gate G5).
2. Clean-machine installs: `go build` (with only Go + C toolchain) and `dotnet add package VecLite` + quickstart run.
3. Leak checks: Go — `runtime` finalizer test + valgrind on the cgo shim; C# — `SafeHandle` stress with forced GC, zero native leaks.
4. Concurrency smoke: parallel goroutines / `Parallel.For` across all ops (backed by FFI-001 thread safety).
5. Error mapping exhaustiveness tests: every FFI code reachable and mapped (incl. unknown-code → internal-error fallback for forward compat).
