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
	"unsafe"
)

// SparseVector is a sparse lane for hybrid search (SPEC-007).
type SparseVector struct {
	Indices []uint32  `json:"indices"`
	Values  []float32 `json:"values"`
}

// Point is a vector record. Payload is any JSON-serializable value (typically a
// map[string]any); Sparse is optional.
type Point struct {
	ID      string        `json:"id"`
	Vector  []float32     `json:"vector"`
	Payload any           `json:"payload,omitempty"`
	Sparse  *SparseVector `json:"sparse,omitempty"`
}

// Hit is one ranked search result.
type Hit struct {
	ID      string    `json:"id"`
	Score   float32   `json:"score"`
	Payload any       `json:"payload,omitempty"`
	Vector  []float32 `json:"vector,omitempty"`
}

// SearchOptions tunes a k-NN or text search.
type SearchOptions struct {
	Limit       int            `json:"-"`
	EfSearch    *int           `json:"ef_search,omitempty"`
	WithPayload *bool          `json:"with_payload,omitempty"`
	WithVector  *bool          `json:"with_vector,omitempty"`
	Filter      map[string]any `json:"filter,omitempty"`
}

// ScrollOptions paginates a full scan.
type ScrollOptions struct {
	Limit    int            `json:"limit,omitempty"`
	OffsetID string         `json:"cursor,omitempty"`
	Filter   map[string]any `json:"filter,omitempty"`
}

// HybridOptions configures a fused dense+sparse+text search.
type HybridOptions struct {
	Dense       []float32      `json:"dense,omitempty"`
	Text        string         `json:"text,omitempty"`
	Sparse      *SparseVector  `json:"sparse,omitempty"`
	Limit       int            `json:"limit,omitempty"`
	Alpha       *float32       `json:"alpha,omitempty"`
	RRFK        *float32       `json:"rrf_k,omitempty"`
	WithPayload *bool          `json:"with_payload,omitempty"`
	WithVector  *bool          `json:"with_vector,omitempty"`
	Filter      map[string]any `json:"filter,omitempty"`
}

// PayloadIndexKind selects a payload-index type.
type PayloadIndexKind uint8

const (
	IndexKeyword PayloadIndexKind = C.VL_PIDX_KEYWORD
	IndexInteger PayloadIndexKind = C.VL_PIDX_INTEGER
	IndexFloat   PayloadIndexKind = C.VL_PIDX_FLOAT
)

// Collection is a handle to a collection. A lightweight view over the database;
// its native handle is released by Close (also from a finalizer, GO-012).
type Collection struct {
	ptr *C.vl_collection
}

func newCollection(ptr *C.vl_collection) *Collection {
	c := &Collection{ptr: ptr}
	runtime.SetFinalizer(c, (*Collection).Close)
	return c
}

// Close frees the collection handle. Idempotent.
func (c *Collection) Close() error {
	if c.ptr == nil {
		return nil
	}
	code := C.vl_collection_free(c.ptr)
	c.ptr = nil
	runtime.SetFinalizer(c, nil)
	return statusErr(code)
}

// Upsert inserts or replaces one point. A point carrying a sparse lane routes
// through the batch path, since the single-point C entry point takes only a
// dense vector + payload (the frozen ABI has no sparse parameter there).
func (c *Collection) Upsert(p Point) error {
	if p.Sparse != nil {
		return c.UpsertBatch([]Point{p})
	}
	cid := C.CString(p.ID)
	defer C.free(unsafe.Pointer(cid))
	vptr, vlen := cFloats(p.Vector)

	var payloadPtr *C.uint8_t
	var payloadLen C.size_t
	var payloadBytes []byte
	if p.Payload != nil {
		b, err := json.Marshal(p.Payload)
		if err != nil {
			return marshalErr(err)
		}
		payloadBytes = b
		payloadPtr, payloadLen = cBytes(b)
	}
	code := C.vl_upsert(c.ptr, cid, vptr, vlen, payloadPtr, payloadLen, codecJSON)
	runtime.KeepAlive(p.Vector)
	runtime.KeepAlive(payloadBytes)
	runtime.KeepAlive(c)
	return statusErr(code)
}

// UpsertBatch inserts or replaces many points atomically.
func (c *Collection) UpsertBatch(points []Point) error {
	blob, err := json.Marshal(points)
	if err != nil {
		return marshalErr(err)
	}
	ptr, n := cBytes(blob)
	code := C.vl_upsert_batch(c.ptr, ptr, n, codecJSON)
	runtime.KeepAlive(blob)
	runtime.KeepAlive(c)
	return statusErr(code)
}

// UpsertText inserts or replaces one text document (auto-embed collections).
func (c *Collection) UpsertText(id, text string, payload any) error {
	cid := C.CString(id)
	defer C.free(unsafe.Pointer(cid))
	ctext := C.CString(text)
	defer C.free(unsafe.Pointer(ctext))
	var payloadPtr *C.uint8_t
	var payloadLen C.size_t
	var payloadBytes []byte
	if payload != nil {
		b, err := json.Marshal(payload)
		if err != nil {
			return marshalErr(err)
		}
		payloadBytes = b
		payloadPtr, payloadLen = cBytes(b)
	}
	code := C.vl_upsert_text(c.ptr, cid, ctext, payloadPtr, payloadLen, codecJSON)
	runtime.KeepAlive(payloadBytes)
	runtime.KeepAlive(c)
	return statusErr(code)
}

// Delete removes one id; returns whether it existed.
func (c *Collection) Delete(id string) (bool, error) {
	cid := C.CString(id)
	defer C.free(unsafe.Pointer(cid))
	var existed C.bool
	code := C.vl_delete(c.ptr, cid, &existed)
	runtime.KeepAlive(c)
	if err := statusErr(code); err != nil {
		return false, err
	}
	return bool(existed), nil
}

// DeleteBatch removes many ids; returns how many existed.
func (c *Collection) DeleteBatch(ids []string) (int, error) {
	blob, err := json.Marshal(ids)
	if err != nil {
		return 0, marshalErr(err)
	}
	ptr, n := cBytes(blob)
	var deleted C.uint64_t
	code := C.vl_delete_batch(c.ptr, ptr, n, codecJSON, &deleted)
	runtime.KeepAlive(blob)
	runtime.KeepAlive(c)
	if err := statusErr(code); err != nil {
		return 0, err
	}
	return int(deleted), nil
}

// Count returns the number of live vectors.
func (c *Collection) Count() (int, error) {
	var n C.uint64_t
	code := C.vl_count(c.ptr, &n)
	runtime.KeepAlive(c)
	if err := statusErr(code); err != nil {
		return 0, err
	}
	return int(n), nil
}

// Get fetches one point by id, or (nil, nil) if absent.
func (c *Collection) Get(id string) (*Point, error) {
	cid := C.CString(id)
	defer C.free(unsafe.Pointer(cid))
	var buf C.vl_buf
	code := C.vl_get(c.ptr, cid, codecJSON, &buf)
	runtime.KeepAlive(c)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	raw := takeBuf(&buf)
	if len(raw) == 0 {
		return nil, nil // absent
	}
	var p Point
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	return &p, nil
}

// Search runs a k-NN search over a dense query vector.
func (c *Collection) Search(query []float32, opts SearchOptions) ([]Hit, error) {
	optBytes, err := marshalQueryOpts(opts)
	if err != nil {
		return nil, err
	}
	vptr, vlen := cFloats(query)
	optPtr, optLen := cBytes(optBytes)
	var hits *C.vl_hits
	code := C.vl_search(c.ptr, vptr, vlen, C.uint32_t(defaultLimit(opts.Limit)), optPtr, optLen, codecJSON, &hits)
	runtime.KeepAlive(query)
	runtime.KeepAlive(optBytes)
	runtime.KeepAlive(c)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	return collectHits(hits), nil
}

// SearchText runs a text search (auto-embed collections). Only WithPayload /
// WithVector from opts apply; Filter/EfSearch are rejected by the core text path
// (use HybridSearch with a text query and a filter).
func (c *Collection) SearchText(query string, opts SearchOptions) ([]Hit, error) {
	optBytes, err := marshalQueryOpts(opts)
	if err != nil {
		return nil, err
	}
	cq := C.CString(query)
	defer C.free(unsafe.Pointer(cq))
	optPtr, optLen := cBytes(optBytes)
	var hits *C.vl_hits
	code := C.vl_search_text(c.ptr, cq, C.uint32_t(defaultLimit(opts.Limit)), optPtr, optLen, codecJSON, &hits)
	runtime.KeepAlive(optBytes)
	runtime.KeepAlive(c)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	return collectHits(hits), nil
}

// HybridSearch runs a fused dense+sparse+text search (at least one channel).
func (c *Collection) HybridSearch(opts HybridOptions) ([]Hit, error) {
	blob, err := json.Marshal(opts)
	if err != nil {
		return nil, marshalErr(err)
	}
	ptr, n := cBytes(blob)
	var hits *C.vl_hits
	code := C.vl_hybrid_search(c.ptr, ptr, n, codecJSON, &hits)
	runtime.KeepAlive(blob)
	runtime.KeepAlive(c)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	return collectHits(hits), nil
}

// Page is one scroll page.
type Page struct {
	Points     []Point
	NextCursor string
}

// Scroll paginates the collection in id order.
func (c *Collection) Scroll(opts ScrollOptions) (*Page, error) {
	blob, err := json.Marshal(opts)
	if err != nil {
		return nil, marshalErr(err)
	}
	ptr, n := cBytes(blob)
	var page *C.vl_page
	code := C.vl_scroll(c.ptr, ptr, n, codecJSON, &page)
	runtime.KeepAlive(blob)
	runtime.KeepAlive(c)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	defer C.vl_page_free(page)

	out := &Page{}
	count := int(C.vl_page_len(page))
	out.Points = make([]Point, 0, count)
	for i := 0; i < count; i++ {
		var buf C.vl_buf
		if err := statusErr(C.vl_page_point(page, C.uint32_t(i), &buf)); err != nil {
			return nil, err
		}
		raw := takeBuf(&buf)
		var p Point
		if err := json.Unmarshal(raw, &p); err != nil {
			return nil, err
		}
		out.Points = append(out.Points, p)
	}
	if cur := C.vl_page_cursor(page); cur != nil {
		out.NextCursor = C.GoString(cur)
	}
	return out, nil
}

// Reindex rebuilds the ANN index from the live vectors.
func (c *Collection) Reindex() error {
	defer runtime.KeepAlive(c)
	return statusErr(C.vl_collection_reindex(c.ptr))
}

// Refit recomputes the text embedder's vocabulary and re-embeds text documents.
func (c *Collection) Refit() error {
	defer runtime.KeepAlive(c)
	return statusErr(C.vl_collection_refit(c.ptr))
}

// CreatePayloadIndex declares a payload index on a field.
func (c *Collection) CreatePayloadIndex(field string, kind PayloadIndexKind) error {
	cf := C.CString(field)
	defer C.free(unsafe.Pointer(cf))
	defer runtime.KeepAlive(c)
	return statusErr(C.vl_payload_index_create(c.ptr, cf, C.uint8_t(kind)))
}

// Stats returns collection statistics.
func (c *Collection) Stats() (map[string]any, error) {
	var buf C.vl_buf
	code := C.vl_collection_stats(c.ptr, codecJSON, &buf)
	runtime.KeepAlive(c)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	raw := takeBuf(&buf)
	var s map[string]any
	if err := json.Unmarshal(raw, &s); err != nil {
		return nil, err
	}
	return s, nil
}

// TextChunk is one text chunk with its byte range in the source.
type TextChunk struct {
	Text  string `json:"text"`
	Start int    `json:"start"`
	End   int    `json:"end"`
}

// Chunk splits text into overlapping, UTF-8-safe chunks (SPEC-005 §7). A pure
// function; maxChars/overlap default to 2048/128 when zero.
func Chunk(text string, maxChars, overlap int) ([]TextChunk, error) {
	opts := map[string]int{}
	if maxChars > 0 {
		opts["max_chars"] = maxChars
	}
	if overlap > 0 {
		opts["overlap"] = overlap
	}
	blob, err := json.Marshal(opts)
	if err != nil {
		return nil, marshalErr(err)
	}
	ctext := C.CString(text)
	defer C.free(unsafe.Pointer(ctext))
	ptr, n := cBytes(blob)
	var buf C.vl_buf
	code := C.vl_chunk(ctext, ptr, n, codecJSON, &buf)
	runtime.KeepAlive(blob)
	if err := statusErr(code); err != nil {
		return nil, err
	}
	raw := takeBuf(&buf)
	var chunks []TextChunk
	if err := json.Unmarshal(raw, &chunks); err != nil {
		return nil, err
	}
	return chunks, nil
}

// ── helpers ──────────────────────────────────────────────────────────────────

func marshalErr(err error) error {
	return &Error{Code: int32(C.VL_ERR_INVALID_ARGUMENT), Message: err.Error(), sentinel: ErrInvalidArgument}
}

// defaultLimit applies the SPEC-004 default search limit of 10 when the caller
// leaves Limit unset (0).
func defaultLimit(limit int) int {
	if limit <= 0 {
		return 10
	}
	return limit
}

// marshalQueryOpts encodes SearchOptions to the query_opts JSON, or nil when no
// tunable field is set (an empty blob = defaults).
func marshalQueryOpts(opts SearchOptions) ([]byte, error) {
	if opts.EfSearch == nil && opts.WithPayload == nil && opts.WithVector == nil && opts.Filter == nil {
		return nil, nil
	}
	b, err := json.Marshal(opts)
	if err != nil {
		return nil, marshalErr(err)
	}
	return b, nil
}

// collectHits copies every hit out of a vl_hits set (borrowed views valid until
// free) into Go values, then frees the native set.
func collectHits(hits *C.vl_hits) []Hit {
	defer C.vl_hits_free(hits)
	n := int(C.vl_hits_len(hits))
	out := make([]Hit, 0, n)
	for i := 0; i < n; i++ {
		var view C.vl_hit_view
		if C.vl_hits_get(hits, C.uint32_t(i), &view) != C.VL_OK {
			continue
		}
		h := Hit{
			ID:    C.GoString(view.id),
			Score: float32(view.score),
		}
		if view.payload != nil && view.payload_len > 0 {
			raw := C.GoBytes(unsafe.Pointer(view.payload), C.int(view.payload_len))
			var payload any
			if json.Unmarshal(raw, &payload) == nil {
				h.Payload = payload
			}
		}
		if bool(view.has_vector) && view.vector != nil && view.vector_len > 0 {
			src := unsafe.Slice((*float32)(unsafe.Pointer(view.vector)), int(view.vector_len))
			h.Vector = append([]float32(nil), src...)
		}
		out = append(out, h)
	}
	return out
}
