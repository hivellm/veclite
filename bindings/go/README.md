# veclite-go

Go binding for [VecLite](https://github.com/hivellm/veclite) — an embedded,
single-file, in-process vector database — over its stable C ABI (SPEC-008/011).
Wraps `veclite-ffi` via cgo; it never links the Rust crate directly.

```go
import "github.com/hivellm/veclite-go"

db, err := veclite.Open("app.veclite", nil)   // or veclite.Memory()
defer db.Close()

bits := uint8(0)
docs, _ := db.CreateCollection("docs", veclite.CollectionOptions{
    Dimension: 3, Metric: veclite.Euclidean, QuantizationBits: &bits,
})
_ = docs.Upsert(veclite.Point{ID: "a", Vector: []float32{1, 0, 0}, Payload: map[string]any{"lang": "en"}})

hits, _ := docs.Search([]float32{0.9, 0.1, 0}, veclite.SearchOptions{
    Limit:  10,
    Filter: map[string]any{"must": []any{map[string]any{"key": "lang", "match": map[string]any{"value": "en"}}}},
})
```

## Design

- **Errors** (GO-010): every call returns `error`. Sentinels
  (`veclite.ErrLocked`, `veclite.ErrCollectionNotFound`, …) work with
  `errors.Is`; the concrete `*veclite.Error` carries the exact FFI message and a
  `Code`/`CodeString()` shared with every other binding. Unknown/future codes
  fall back to `ErrInternal`.
- **Zero-copy vectors** (GO-011): `[]float32` slices pass to cgo pinned via
  `unsafe.Pointer` + `runtime.KeepAlive`, no copy. Structured values cross as
  JSON.
- **Handles** (GO-012): `Close` is idempotent; a `runtime.SetFinalizer` safety
  net closes leaked handles and releases the file lock. All types are safe for
  concurrent use by multiple goroutines (FFI-001).
- **Synchronous** (GO-013): calls block the calling goroutine — the idiomatic Go
  equivalent of the sync core. No goroutine is spawned by the binding.

## Building

`go get` + build needs only Go and a C toolchain — no Rust. The prebuilt VecLite
library is bundled per platform under `lib/<goos>_<goarch>/` by the release
(see `lib/README.md`); Linux/macOS link it statically, Windows ships the dll.

Locally, build the library and point cgo at it:

```bash
cargo build -p veclite-ffi --release
cp target/release/veclite_ffi.dll bindings/go/lib/windows_amd64/   # or .so/.a
cd bindings/go
CC="zig cc" go test ./...    # any cgo C compiler works
```

## License

Apache-2.0.
