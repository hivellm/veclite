package veclite

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func TestQuickstartMemory(t *testing.T) {
	db := Memory()
	defer db.Close()

	bits := uint8(0)
	docs, err := db.CreateCollection("docs", CollectionOptions{
		Dimension: 3, Metric: Euclidean, QuantizationBits: &bits,
	})
	if err != nil {
		t.Fatal(err)
	}
	defer docs.Close()

	if err := docs.Upsert(Point{ID: "a", Vector: []float32{1, 0, 0}, Payload: map[string]any{"lang": "en"}}); err != nil {
		t.Fatal(err)
	}
	if err := docs.Upsert(Point{ID: "b", Vector: []float32{0, 1, 0}}); err != nil {
		t.Fatal(err)
	}
	n, err := docs.Count()
	if err != nil || n != 2 {
		t.Fatalf("count=%d err=%v", n, err)
	}

	hits, err := docs.Search([]float32{0.9, 0.1, 0}, SearchOptions{Limit: 1})
	if err != nil {
		t.Fatal(err)
	}
	if len(hits) != 1 || hits[0].ID != "a" {
		t.Fatalf("unexpected hits: %+v", hits)
	}
	payload, _ := hits[0].Payload.(map[string]any)
	if payload["lang"] != "en" {
		t.Fatalf("payload: %+v", hits[0].Payload)
	}

	got, err := docs.Get("a")
	if err != nil || got == nil || got.ID != "a" {
		t.Fatalf("get: %+v err=%v", got, err)
	}
	missing, err := docs.Get("nope")
	if err != nil || missing != nil {
		t.Fatalf("expected nil for missing: %+v err=%v", missing, err)
	}
	existed, err := docs.Delete("a")
	if err != nil || !existed {
		t.Fatalf("delete: existed=%v err=%v", existed, err)
	}
}

func TestBatchScrollAndFilter(t *testing.T) {
	db := Memory()
	defer db.Close()
	bits := uint8(0)
	c, err := db.CreateCollection("v", CollectionOptions{Dimension: 2, Metric: Euclidean, QuantizationBits: &bits})
	if err != nil {
		t.Fatal(err)
	}
	points := []Point{
		{ID: "a", Vector: []float32{0, 0}, Payload: map[string]any{"lang": "en"}},
		{ID: "b", Vector: []float32{1, 0}, Payload: map[string]any{"lang": "pt"}},
		{ID: "c", Vector: []float32{0, 1}, Payload: map[string]any{"lang": "en"}},
	}
	if err := c.UpsertBatch(points); err != nil {
		t.Fatal(err)
	}
	n, _ := c.Count()
	if n != 3 {
		t.Fatalf("count=%d", n)
	}

	// Filtered search: only "en".
	withVec := true
	hits, err := c.Search([]float32{0, 0}, SearchOptions{
		Limit:      10,
		WithVector: &withVec,
		Filter:     map[string]any{"must": []any{map[string]any{"key": "lang", "match": map[string]any{"value": "en"}}}},
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(hits) != 2 {
		t.Fatalf("expected 2 en hits, got %d", len(hits))
	}
	if len(hits[0].Vector) != 2 {
		t.Fatalf("with_vector did not project a vector: %+v", hits[0])
	}

	// Scroll all.
	seen := map[string]bool{}
	cursor := ""
	for {
		page, err := c.Scroll(ScrollOptions{Limit: 2, OffsetID: cursor})
		if err != nil {
			t.Fatal(err)
		}
		for _, p := range page.Points {
			seen[p.ID] = true
		}
		if page.NextCursor == "" {
			break
		}
		cursor = page.NextCursor
	}
	if len(seen) != 3 {
		t.Fatalf("scrolled %d distinct, want 3", len(seen))
	}

	deleted, err := c.DeleteBatch([]string{"a", "c", "missing"})
	if err != nil || deleted != 2 {
		t.Fatalf("delete_batch=%d err=%v", deleted, err)
	}
}

func TestLockedErrorIsDetectable(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "locked.veclite")
	db, err := Open(path, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	// A second open of the same path from this process must report Locked.
	_, err = Open(path, nil)
	if err == nil {
		t.Fatal("expected a locked error on the second open")
	}
	if !errors.Is(err, ErrLocked) {
		t.Fatalf("errors.Is(err, ErrLocked) failed for: %v", err)
	}
	_ = os.Remove(path)
}

func TestAutoEmbedText(t *testing.T) {
	db := Memory()
	defer db.Close()
	c, err := db.CreateCollection("t", CollectionOptions{Dimension: 64, EmbeddingProvider: "bm25"})
	if err != nil {
		t.Fatal(err)
	}
	if err := c.UpsertText("d1", "the quick brown fox", nil); err != nil {
		t.Fatal(err)
	}
	if err := c.UpsertText("d2", "a lazy sleeping dog", map[string]any{"tag": "animal"}); err != nil {
		t.Fatal(err)
	}
	hits, err := c.SearchText("quick fox", SearchOptions{Limit: 2})
	if err != nil {
		t.Fatal(err)
	}
	if len(hits) < 1 {
		t.Fatalf("expected >= 1 text hit, got %d", len(hits))
	}
}
