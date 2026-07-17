//go:build windows && amd64

package veclite

// Windows links the shared library dynamically: static linking would have to
// bundle the `windows` crate's import libraries, so the release ships
// veclite_ffi.dll + its import lib instead (the dll must be on PATH or beside
// the executable). One cgo directive file per platform keeps the linker flags
// build-tagged (GO-001).

// #cgo LDFLAGS: -L${SRCDIR}/lib/windows_amd64 -lveclite_ffi
import "C"
