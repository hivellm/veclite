# C# quickstart

Add the package — a single NuGet with prebuilt native assets, no Rust toolchain
(.NET ≥ 8):

```bash
dotnet add package VecLite
```

The program below opens a durable single-file database, does a filtered k-NN
search over `float[]` vectors (or `ReadOnlySpan<float>` zero-copy), and a text
search over an offline BM25 auto-embed collection. `Database` and `Collection`
are `IDisposable` — `using` releases the native handles deterministically.

```csharp
{{#include ../../../bindings/csharp/Quickstart/Program.cs}}
```

Run it:

```bash
dotnet run --project bindings/csharp/Quickstart -c Release
# veclite 0.1.0: quickstart OK (a, c)
```

See [SPEC-011](../../specs/SPEC-011-bindings-go-csharp.md) for the full surface,
the P/Invoke design, and the exception hierarchy.
