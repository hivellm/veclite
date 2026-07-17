// Package veclite is a Go binding for VecLite — an embedded, single-file,
// in-process vector database — over its stable C ABI (SPEC-008/011). It links
// the bundled prebuilt VecLite shared library per platform (GO-001); no Rust
// toolchain is required to build, only cgo's C compiler.
//
// Structured values (payloads, filters, options, results) cross the ABI as JSON
// (VL_CODEC_JSON); vectors cross as pinned float32 slices without a copy
// (GO-011). Every call blocks the calling goroutine (GO-013) and all handles are
// safe for concurrent use by multiple goroutines (FFI-001, GO-012).
package veclite

/*
#cgo CFLAGS: -I${SRCDIR}/internal/csrc
#include "veclite.h"
#include <stdlib.h>
*/
import "C"

import (
	"errors"
	"unsafe"
)

// Codec flags (FFI-005). This binding uses JSON everywhere for a zero-dependency
// footprint; the ABI accepts either JSON or MessagePack.
const codecJSON = C.uint8_t(C.VL_CODEC_JSON)

// Version returns the VecLite core semver string (GO-002).
func Version() string {
	return C.GoString(C.vl_version())
}

// AbiVersion returns the C ABI version; loaders gate on it (FFI-007).
func AbiVersion() uint32 {
	return uint32(C.vl_abi_version())
}

// FormatVersion returns the on-disk storage format version (GO-002).
func FormatVersion() uint32 {
	return uint32(C.vl_format_version())
}

// ── error mapping (GO-010) ───────────────────────────────────────────────────

// Sentinel errors mapped 1:1 from FFI codes (SPEC-008 §3). Compare with
// errors.Is; the concrete error also carries the FFI thread-local message.
var (
	ErrCollectionNotFound  = errors.New("veclite: collection not found")
	ErrVectorNotFound      = errors.New("veclite: vector not found")
	ErrAlreadyExists       = errors.New("veclite: already exists")
	ErrDimensionMismatch   = errors.New("veclite: dimension mismatch")
	ErrLocked              = errors.New("veclite: database is locked by another process")
	ErrCorrupt             = errors.New("veclite: corrupt data")
	ErrUnsupportedFormat   = errors.New("veclite: unsupported format version")
	ErrUnsupportedProvider = errors.New("veclite: unsupported embedding provider")
	ErrReadOnly            = errors.New("veclite: database is read-only")
	ErrInvalidArgument     = errors.New("veclite: invalid argument")
	ErrIO                  = errors.New("veclite: I/O error")
	ErrWALPending          = errors.New("veclite: WAL replay pending")
	ErrClosed              = errors.New("veclite: handle is closed")
	ErrInternal            = errors.New("veclite: internal error")
)

// Error is a VecLite failure: a sentinel (for errors.Is) wrapping the exact FFI
// message. Code is the raw FFI status.
type Error struct {
	Code     int32
	Message  string
	sentinel error
}

func (e *Error) Error() string {
	if e.Message != "" {
		return e.Message
	}
	return e.sentinel.Error()
}

// Unwrap exposes the sentinel so errors.Is(err, ErrLocked) works.
func (e *Error) Unwrap() error { return e.sentinel }

// CodeString is the stable, language-agnostic error code (e.g.
// "DIMENSION_MISMATCH"), shared with every other VecLite binding and the
// conformance corpus. Unknown/future codes report "INTERNAL".
func (e *Error) CodeString() string {
	switch e.Code {
	case int32(C.VL_ERR_COLLECTION_NOT_FOUND):
		return "COLLECTION_NOT_FOUND"
	case int32(C.VL_ERR_VECTOR_NOT_FOUND):
		return "VECTOR_NOT_FOUND"
	case int32(C.VL_ERR_ALREADY_EXISTS):
		return "ALREADY_EXISTS"
	case int32(C.VL_ERR_DIMENSION_MISMATCH):
		return "DIMENSION_MISMATCH"
	case int32(C.VL_ERR_LOCKED):
		return "LOCKED"
	case int32(C.VL_ERR_CORRUPT):
		return "CORRUPT"
	case int32(C.VL_ERR_UNSUPPORTED_FORMAT):
		return "UNSUPPORTED_FORMAT"
	case int32(C.VL_ERR_UNSUPPORTED_PROVIDER):
		return "UNSUPPORTED_PROVIDER"
	case int32(C.VL_ERR_READ_ONLY):
		return "READ_ONLY"
	case int32(C.VL_ERR_INVALID_ARGUMENT):
		return "INVALID_ARGUMENT"
	case int32(C.VL_ERR_IO):
		return "IO"
	case int32(C.VL_ERR_WAL_PENDING):
		return "WAL_PENDING"
	case int32(C.VL_ERR_CLOSED):
		return "CLOSED"
	default:
		return "INTERNAL"
	}
}

func sentinelFor(code int32) error {
	switch code {
	case C.VL_ERR_COLLECTION_NOT_FOUND:
		return ErrCollectionNotFound
	case C.VL_ERR_VECTOR_NOT_FOUND:
		return ErrVectorNotFound
	case C.VL_ERR_ALREADY_EXISTS:
		return ErrAlreadyExists
	case C.VL_ERR_DIMENSION_MISMATCH:
		return ErrDimensionMismatch
	case C.VL_ERR_LOCKED:
		return ErrLocked
	case C.VL_ERR_CORRUPT:
		return ErrCorrupt
	case C.VL_ERR_UNSUPPORTED_FORMAT:
		return ErrUnsupportedFormat
	case C.VL_ERR_UNSUPPORTED_PROVIDER:
		return ErrUnsupportedProvider
	case C.VL_ERR_READ_ONLY:
		return ErrReadOnly
	case C.VL_ERR_INVALID_ARGUMENT:
		return ErrInvalidArgument
	case C.VL_ERR_IO:
		return ErrIO
	case C.VL_ERR_WAL_PENDING:
		return ErrWALPending
	case C.VL_ERR_CLOSED:
		return ErrClosed
	default:
		// Unknown/future codes (incl. VL_ERR_INTERNAL) map to internal — forward
		// compatible with a #[non_exhaustive] core (GO-010, acceptance 5).
		return ErrInternal
	}
}

// lastMessage reads the thread-local FFI error message set on the failing call.
func lastMessage() string {
	return C.GoString(C.vl_last_error_message())
}

// statusErr turns a non-OK FFI status into an *Error, or nil for VL_OK.
func statusErr(code C.int32_t) error {
	if code == C.VL_OK {
		return nil
	}
	c := int32(code)
	return &Error{Code: c, Message: lastMessage(), sentinel: sentinelFor(c)}
}

// cFloats returns a pinned pointer to the slice data for the duration of a cgo
// call. The caller MUST runtime.KeepAlive(the slice) until after the C call
// returns (GO-011). Empty slices yield a nil pointer with length 0.
func cFloats(v []float32) (*C.float, C.size_t) {
	if len(v) == 0 {
		return nil, 0
	}
	return (*C.float)(unsafe.Pointer(&v[0])), C.size_t(len(v))
}
