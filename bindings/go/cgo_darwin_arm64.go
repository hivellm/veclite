//go:build darwin && arm64

package veclite

// #cgo LDFLAGS: ${SRCDIR}/lib/darwin_arm64/libveclite_ffi.a -framework Security -framework CoreFoundation
import "C"
