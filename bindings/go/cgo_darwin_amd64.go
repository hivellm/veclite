//go:build darwin && amd64

package veclite

// macOS links the bundled static library plus the frameworks Rust std needs.

// #cgo LDFLAGS: ${SRCDIR}/lib/darwin_amd64/libveclite_ffi.a -framework Security -framework CoreFoundation
import "C"
