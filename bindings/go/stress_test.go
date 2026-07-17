package veclite

import (
	"errors"
	"fmt"
	"path/filepath"
	"runtime"
	"sync"
	"testing"
	"time"
)

// Concurrency smoke (GO-012, FFI-001): many goroutines hammer one shared
// collection with interleaved writes and reads; the final count is exact.
func TestConcurrentGoroutines(t *testing.T) {
	db := Memory()
	defer db.Close()
	bits := uint8(0)
	c, err := db.CreateCollection("v", CollectionOptions{Dimension: 4, Metric: Cosine, QuantizationBits: &bits})
	if err != nil {
		t.Fatal(err)
	}
	defer c.Close()

	const workers, per = 16, 64
	var wg sync.WaitGroup
	errCh := make(chan error, workers)
	for w := 0; w < workers; w++ {
		wg.Add(1)
		go func(w int) {
			defer wg.Done()
			for i := 0; i < per; i++ {
				id := fmt.Sprintf("w%d_%d", w, i)
				vec := []float32{float32(w), float32(i), float32(w ^ i), 1}
				if err := c.Upsert(Point{ID: id, Vector: vec}); err != nil {
					errCh <- err
					return
				}
				if _, err := c.Search(vec, SearchOptions{Limit: 5}); err != nil {
					errCh <- err
					return
				}
			}
		}(w)
	}
	wg.Wait()
	close(errCh)
	for err := range errCh {
		t.Fatal(err)
	}

	n, err := c.Count()
	if err != nil {
		t.Fatal(err)
	}
	if n != workers*per {
		t.Fatalf("count=%d, want %d", n, workers*per)
	}
}

// Error mapping is exhaustive and errors.Is-friendly (GO-010, acceptance 5).
func TestErrorMapping(t *testing.T) {
	db := Memory()
	defer db.Close()
	bits := uint8(0)
	if _, err := db.CreateCollection("v", CollectionOptions{Dimension: 3, Metric: Euclidean, QuantizationBits: &bits}); err != nil {
		t.Fatal(err)
	}

	// ALREADY_EXISTS: create a duplicate collection.
	_, err := db.CreateCollection("v", CollectionOptions{Dimension: 3})
	if !errors.Is(err, ErrAlreadyExists) {
		t.Fatalf("expected ErrAlreadyExists, got %v", err)
	}

	// COLLECTION_NOT_FOUND: get a missing collection.
	_, err = db.Collection("nope")
	if !errors.Is(err, ErrCollectionNotFound) {
		t.Fatalf("expected ErrCollectionNotFound, got %v", err)
	}

	// DIMENSION_MISMATCH: upsert a wrong-width vector.
	c, _ := db.Collection("v")
	defer c.Close()
	err = c.Upsert(Point{ID: "x", Vector: []float32{1, 2}})
	if !errors.Is(err, ErrDimensionMismatch) {
		t.Fatalf("expected ErrDimensionMismatch, got %v", err)
	}
	var ve *Error
	if !errors.As(err, &ve) || ve.CodeString() != "DIMENSION_MISMATCH" {
		t.Fatalf("errors.As / CodeString failed: %+v", ve)
	}

	// UNSUPPORTED_PROVIDER: unknown auto-embed provider.
	_, err = db.CreateCollection("bad", CollectionOptions{Dimension: 8, EmbeddingProvider: "no-such-provider"})
	if !errors.Is(err, ErrUnsupportedProvider) {
		t.Fatalf("expected ErrUnsupportedProvider, got %v", err)
	}

	// Unknown/forward-compatible code falls back to INTERNAL.
	unknown := &Error{Code: -12345, sentinel: sentinelFor(-12345)}
	if !errors.Is(unknown, ErrInternal) || unknown.CodeString() != "INTERNAL" {
		t.Fatalf("unknown-code fallback failed: %+v", unknown)
	}
}

// The finalizer safety net closes a leaked file-backed handle, releasing its
// advisory lock (GO-012). We leak a file db without Close, force GC, and confirm
// the same path can be reopened once the finalizer has run.
func TestFinalizerReleasesFileLock(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "fin.veclite")

	// Open and leak — no Close, no lingering reference after this returns.
	func() {
		db, err := Open(path, nil)
		if err != nil {
			t.Fatal(err)
		}
		_ = db
	}()

	var reopened *Database
	var err error
	for i := 0; i < 20; i++ {
		runtime.GC()
		time.Sleep(5 * time.Millisecond) // let the finalizer goroutine run
		reopened, err = Open(path, nil)
		if err == nil {
			break
		}
	}
	if err != nil {
		t.Fatalf("finalizer did not release the file lock: %v", err)
	}
	_ = reopened.Close()
}
