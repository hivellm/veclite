package veclite

/*
#cgo CFLAGS: -I${SRCDIR}/internal/csrc
#include "veclite.h"
#include <stdlib.h>
*/
import "C"

import (
	"encoding/json"
	"runtime"
	"sync"
	"unsafe"
)

// Metric names a distance function (SPEC-004).
type Metric string

const (
	Cosine     Metric = "cosine"
	Euclidean  Metric = "euclidean"
	DotProduct Metric = "dot"
)

// OpenOptions configures Open. ReadOnly is accepted for forward compatibility;
// the current C ABI opens with defaults (vl_open ignores tuned options).
type OpenOptions struct {
	ReadOnly bool
}

// CollectionOptions configures CreateCollection.
type CollectionOptions struct {
	Dimension         int    `json:"dimension"`
	Metric            Metric `json:"metric,omitempty"`
	QuantizationBits  *uint8 `json:"quantization_bits,omitempty"`
	EmbeddingProvider string `json:"embedding_provider,omitempty"`
}

// Database is a handle to a VecLite database. Safe for concurrent use by
// multiple goroutines (FFI-001). Close is idempotent and also runs from a
// finalizer if a handle is leaked (GO-012).
type Database struct {
	mu  sync.Mutex
	ptr *C.vl_db
}

// Open opens (or creates) a durable single-file database.
func Open(path string, _ *OpenOptions) (*Database, error) {
	cpath := C.CString(path)
	defer C.free(unsafe.Pointer(cpath))
	var out *C.vl_db
	// Tuned options are not yet plumbed through the C ABI; open with defaults.
	if err := statusErr(C.vl_open(cpath, nil, 0, &out)); err != nil {
		return nil, err
	}
	return newDatabase(out), nil
}

// Memory opens an ephemeral in-memory database.
func Memory() *Database {
	var out *C.vl_db
	// vl_open_memory only fails on a null out pointer, which cannot happen here.
	C.vl_open_memory(&out)
	return newDatabase(out)
}

func newDatabase(ptr *C.vl_db) *Database {
	db := &Database{ptr: ptr}
	runtime.SetFinalizer(db, (*Database).finalize)
	return db
}

func (db *Database) finalize() {
	// A leaked handle: release native resources. Close is idempotent.
	_ = db.Close()
}

// Close flushes and releases the database handle and its file lock. Idempotent.
func (db *Database) Close() error {
	db.mu.Lock()
	defer db.mu.Unlock()
	if db.ptr == nil {
		return nil
	}
	code := C.vl_db_close(db.ptr)
	db.ptr = nil
	runtime.SetFinalizer(db, nil)
	return statusErr(code)
}

// Checkpoint forces a WAL checkpoint.
func (db *Database) Checkpoint() error {
	defer runtime.KeepAlive(db)
	return statusErr(C.vl_db_checkpoint(db.handle()))
}

// Snapshot writes a consistent copy of the whole database to path.
func (db *Database) Snapshot(path string) error {
	cpath := C.CString(path)
	defer C.free(unsafe.Pointer(cpath))
	defer runtime.KeepAlive(db)
	return statusErr(C.vl_db_snapshot(db.handle(), cpath))
}

// Vacuum reclaims space from tombstoned/rewritten data.
func (db *Database) Vacuum() error {
	defer runtime.KeepAlive(db)
	return statusErr(C.vl_db_vacuum(db.handle()))
}

func (db *Database) handle() *C.vl_db {
	db.mu.Lock()
	defer db.mu.Unlock()
	return db.ptr
}

// CreateCollection creates a collection and returns a handle to it.
func (db *Database) CreateCollection(name string, opts CollectionOptions) (*Collection, error) {
	blob, err := json.Marshal(opts)
	if err != nil {
		return nil, &Error{Code: int32(C.VL_ERR_INVALID_ARGUMENT), Message: err.Error(), sentinel: ErrInvalidArgument}
	}
	cname := C.CString(name)
	defer C.free(unsafe.Pointer(cname))
	optPtr, optLen := cBytes(blob)
	var out *C.vl_collection
	code := C.vl_collection_create(db.handle(), cname, optPtr, optLen, codecJSON, &out)
	runtime.KeepAlive(db)
	runtime.KeepAlive(blob)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	return newCollection(out), nil
}

// Collection returns a handle to an existing collection (or alias).
func (db *Database) Collection(name string) (*Collection, error) {
	cname := C.CString(name)
	defer C.free(unsafe.Pointer(cname))
	var out *C.vl_collection
	code := C.vl_collection_get(db.handle(), cname, &out)
	runtime.KeepAlive(db)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	return newCollection(out), nil
}

// DropCollection deletes a collection.
func (db *Database) DropCollection(name string) error {
	cname := C.CString(name)
	defer C.free(unsafe.Pointer(cname))
	defer runtime.KeepAlive(db)
	return statusErr(C.vl_collection_drop(db.handle(), cname))
}

// RenameCollection renames a collection.
func (db *Database) RenameCollection(from, to string) error {
	cfrom := C.CString(from)
	defer C.free(unsafe.Pointer(cfrom))
	cto := C.CString(to)
	defer C.free(unsafe.Pointer(cto))
	defer runtime.KeepAlive(db)
	return statusErr(C.vl_collection_rename(db.handle(), cfrom, cto))
}

// CreateAlias creates an alias resolving to target.
func (db *Database) CreateAlias(alias, target string) error {
	ca := C.CString(alias)
	defer C.free(unsafe.Pointer(ca))
	ct := C.CString(target)
	defer C.free(unsafe.Pointer(ct))
	defer runtime.KeepAlive(db)
	return statusErr(C.vl_alias_create(db.handle(), ca, ct))
}

// DeleteAlias removes an alias.
func (db *Database) DeleteAlias(alias string) error {
	ca := C.CString(alias)
	defer C.free(unsafe.Pointer(ca))
	defer runtime.KeepAlive(db)
	return statusErr(C.vl_alias_delete(db.handle(), ca))
}

// ListCollections returns the sorted collection names.
func (db *Database) ListCollections() ([]string, error) {
	var buf C.vl_buf
	code := C.vl_collections_list(db.handle(), codecJSON, &buf)
	runtime.KeepAlive(db)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	raw := takeBuf(&buf)
	var names []string
	if err := json.Unmarshal(raw, &names); err != nil {
		return nil, err
	}
	return names, nil
}

// Info returns a database overview (format version, collections, aliases).
func (db *Database) Info() (map[string]any, error) {
	var buf C.vl_buf
	code := C.vl_db_info(db.handle(), codecJSON, &buf)
	runtime.KeepAlive(db)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	raw := takeBuf(&buf)
	var info map[string]any
	if err := json.Unmarshal(raw, &info); err != nil {
		return nil, err
	}
	return info, nil
}

// cBytes returns a pinned pointer + length for a byte slice; keep it alive
// across the call. Empty slices yield (nil, 0).
func cBytes(b []byte) (*C.uint8_t, C.size_t) {
	if len(b) == 0 {
		return nil, 0
	}
	return (*C.uint8_t)(unsafe.Pointer(&b[0])), C.size_t(len(b))
}

// takeBuf copies a library-filled vl_buf into a Go slice and frees the native
// buffer. An empty buffer yields nil.
func takeBuf(buf *C.vl_buf) []byte {
	defer C.vl_buf_free(buf)
	if buf.data == nil || buf.len == 0 {
		return nil
	}
	return C.GoBytes(unsafe.Pointer(buf.data), C.int(buf.len))
}
