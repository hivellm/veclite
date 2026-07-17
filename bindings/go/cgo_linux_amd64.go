//go:build linux && amd64

package veclite

// Linux/amd64 links the bundled static library (GO-001) — a self-contained
// binary with no runtime shared-library dependency. The .a is placed here by the
// release; system libs satisfy the Rust std runtime.

// #cgo LDFLAGS: ${SRCDIR}/lib/linux_amd64/libveclite_ffi.a -lpthread -ldl -lm
import "C"
