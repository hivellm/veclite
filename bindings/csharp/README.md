# VecLite (.NET)

.NET binding for [VecLite](https://github.com/hivellm/veclite) — an embedded,
single-file, in-process vector database — over its stable C ABI (SPEC-008/011).
Wraps `veclite-ffi` via P/Invoke; it never links the Rust crate directly.

```csharp
using VecLite;

using var db = Database.Open("app.veclite");     // or Database.Memory()
using var docs = db.CreateCollection("docs", new CollectionOptions {
    Dimension = 3, Metric = Metric.Euclidean, QuantizationBits = 0,
});

docs.Upsert("a", new float[] { 1, 0, 0 }, new { lang = "en" });   // payload: object | JsonNode
var hits = docs.Search(new float[] { 0.9f, 0.1f, 0f }, new SearchOptions {
    Limit = 10,
    Filter = System.Text.Json.Nodes.JsonNode.Parse("""{"must":[{"key":"lang","match":{"value":"en"}}]}"""),
});
var page = docs.Scroll(new ScrollOptions { Limit = 100 });
```

## Design

- **Install** (CS-001): `dotnet add package VecLite` pulls a single package with
  prebuilt native assets under `runtimes/<rid>/native/` for the FR-66 RIDs; a
  P/Invoke resolver loads the right one. No Rust toolchain. .NET ≥ 8.
- **SafeHandles** (CS-010): `Database`/`Collection` wrap native handles in
  `SafeHandle` subclasses — released exactly once on `Dispose` or, as a safety
  net, under finalization/thread-abort, with no native leak.
- **Zero-copy vectors** (CS-011): `ReadOnlySpan<float>` pins via `fixed` for the
  call duration. Hit vectors return as `float[]` (a copy — .NET arrays can't
  safely borrow native memory across GC).
- **Exceptions** (CS-012): `VecLiteException` with an `ErrorCode` enum and
  specific subclasses (`DimensionMismatchException`, `CollectionNotFoundException`,
  `AlreadyExistsException`, `LockedException`); messages from the FFI. Unknown
  codes fall back to `Internal`.
- **Sync only** in v1 (CS-013): the core is synchronous; `Task` wrappers add
  nothing over the caller's own `Task.Run`.

## Building

```bash
cargo build -p veclite-ffi --release
mkdir -p bindings/csharp/VecLite/runtimes/win-x64/native
cp target/release/veclite_ffi.dll bindings/csharp/VecLite/runtimes/win-x64/native/   # or .so/.dylib
cd bindings/csharp && dotnet test
```

The `VecLite.Onnx` package (ONNX/`fastembed:*` providers, EMB-040) awaits the
`onnx` core feature (phase5c).

## License

Apache-2.0.
