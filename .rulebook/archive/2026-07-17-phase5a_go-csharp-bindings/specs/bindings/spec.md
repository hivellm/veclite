# Bindings Specification

## ADDED Requirements

### Requirement: Go Binding over the C ABI
The Go module SHALL wrap veclite-ffi via cgo with bundled static libraries selected by build tags, expose sentinel errors compatible with errors.Is mapped 1:1 from FFI codes, and pass []float32 slices pinned without copying (SPEC-011 GO-001..013).

#### Scenario: Locked error detectable
Given another process holds the write lock on a database
When veclite.Open is called from Go
Then the returned error satisfies errors.Is(err, veclite.ErrLocked)

### Requirement: C# Binding with SafeHandles
The NuGet package MUST wrap native handles in SafeHandle subclasses with idempotent Dispose, pin ReadOnlySpan<float> vectors for call duration, and surface FFI errors as VecLiteException with an ErrorCode enum (SPEC-011 CS-010..012).

#### Scenario: Handle cleanup under GC
Given database handles that were not explicitly disposed
When the garbage collector finalizes them under stress
Then all native resources are released with zero native memory leaks
