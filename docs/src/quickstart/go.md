# Go quickstart

Add the module (cgo over the C ABI, with bundled static libs):

```bash
go get github.com/hivellm/veclite-go
```

The program below opens an in-memory database, upserts BYO vectors with
payloads, and runs a k-NN search. Building needs a C toolchain for cgo — any of
`cc`, `gcc`, or `zig cc`:

```go
{{#include ../../../bindings/go/examples/quickstart/main.go}}
```

Run it (using `zig cc` as the compiler here):

```bash
cd bindings/go/examples/quickstart
CGO_ENABLED=1 CC="zig cc" go run .
```

See [SPEC-011](../../specs/SPEC-011-bindings-go-csharp.md) for the full package
surface and the per-platform build matrix.
