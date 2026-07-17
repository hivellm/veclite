//go:build linux && arm64

package veclite

// #cgo LDFLAGS: ${SRCDIR}/lib/linux_arm64/libveclite_ffi.a -lpthread -ldl -lm
import "C"
