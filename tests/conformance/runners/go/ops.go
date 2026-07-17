package main

import (
	"encoding/json"
	"errors"
	"math"

	"github.com/hivellm/veclite-go"
)

// execute runs one op and returns its canonical observation. Failures become
// {"error": CODE} using the shared string codes.
func execute(db *veclite.Database, op string, a map[string]any) map[string]any {
	obs, err := dispatch(db, op, a)
	if err != nil {
		var ve *veclite.Error
		if errors.As(err, &ve) {
			return map[string]any{"error": ve.CodeString()}
		}
		return map[string]any{"error": "ERROR"}
	}
	return obs
}

func dispatch(db *veclite.Database, op string, a map[string]any) (map[string]any, error) {
	coll := func() (*veclite.Collection, error) { return db.Collection(str(a["collection"])) }

	switch op {
	case "create_collection":
		opts := veclite.CollectionOptions{Dimension: toInt(a["dimension"]), Metric: veclite.Metric(strOr(a["metric"], "cosine"))}
		if q, ok := a["quantization_bits"]; ok && q != nil {
			b := uint8(toInt(q))
			opts.QuantizationBits = &b
		}
		if ae, ok := a["auto_embed"]; ok && ae != nil {
			opts.EmbeddingProvider = str(ae)
		}
		c, err := db.CreateCollection(str(a["name"]), opts)
		if err != nil {
			return nil, err
		}
		_ = c.Close()
		return map[string]any{}, nil
	case "delete_collection":
		return empty(db.DropCollection(str(a["name"])))
	case "list_collections":
		names, err := db.ListCollections()
		if err != nil {
			return nil, err
		}
		return map[string]any{"ids": names}, nil
	case "create_alias":
		return empty(db.CreateAlias(str(a["alias"]), str(a["target"])))
	case "delete_alias":
		return empty(db.DeleteAlias(str(a["alias"])))
	case "upsert":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		return empty(c.Upsert(veclite.Point{ID: str(a["id"]), Vector: floats(a["vector"]), Payload: a["payload"], Sparse: sparse(a["sparse"])}))
	case "upsert_batch":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		return empty(c.UpsertBatch(points(a["points"])))
	case "upsert_text":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		return empty(c.UpsertText(str(a["id"]), str(a["text"]), a["payload"]))
	case "refit":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		return empty(c.Refit())
	case "get":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		p, err := c.Get(str(a["id"]))
		if err != nil {
			return nil, err
		}
		if p == nil {
			return map[string]any{"result": nil}, nil
		}
		return map[string]any{"result": map[string]any{"id": p.ID, "vector": p.Vector, "payload": p.Payload}}, nil
	case "delete":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		existed, err := c.Delete(str(a["id"]))
		if err != nil {
			return nil, err
		}
		return map[string]any{"value": existed}, nil
	case "len":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		n, err := c.Count()
		if err != nil {
			return nil, err
		}
		return map[string]any{"value": n}, nil
	case "stats":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		s, err := c.Stats()
		if err != nil {
			return nil, err
		}
		return map[string]any{"value": map[string]any{
			"dimension": s["dimension"], "len": s["len"], "tombstones": s["tombstones"], "auto_embed": s["auto_embed"],
		}}, nil
	case "search":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		hits, err := c.Search(floats(a["vector"]), searchOpts(a))
		if err != nil {
			return nil, err
		}
		return hitsObs(hits), nil
	case "search_text":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		hits, err := c.SearchText(str(a["query"]), veclite.SearchOptions{Limit: toIntOr(a["limit"], 10)})
		if err != nil {
			return nil, err
		}
		return hitsObs(hits), nil
	case "hybrid_search":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		hits, err := c.HybridSearch(hybridOpts(a))
		if err != nil {
			return nil, err
		}
		return hitsObs(hits), nil
	case "scroll":
		c, err := coll()
		if err != nil {
			return nil, err
		}
		defer c.Close()
		page, err := c.Scroll(veclite.ScrollOptions{Limit: toInt(a["limit"]), OffsetID: str(a["offset_id"]), Filter: filterMap(a["filter"])})
		if err != nil {
			return nil, err
		}
		ids := make([]string, len(page.Points))
		for i, p := range page.Points {
			ids[i] = p.ID
		}
		var cursor any
		if page.NextCursor != "" {
			cursor = page.NextCursor
		}
		return map[string]any{"ids": ids, "next_cursor": cursor}, nil
	case "chunk":
		chunks, err := veclite.Chunk(str(a["text"]), toInt(a["max_chars"]), toInt(a["overlap"]))
		if err != nil {
			return nil, err
		}
		out := make([]map[string]any, len(chunks))
		for i, ch := range chunks {
			out[i] = map[string]any{"text": ch.Text, "start": ch.Start, "end": ch.End}
		}
		return map[string]any{"result": out}, nil
	default:
		return nil, &veclite.Error{Message: "unknown op " + op}
	}
}

func empty(err error) (map[string]any, error) {
	if err != nil {
		return nil, err
	}
	return map[string]any{}, nil
}

func hitsObs(hits []veclite.Hit) map[string]any {
	ids := make([]string, len(hits))
	scores := make([]float64, len(hits))
	for i, h := range hits {
		ids[i] = h.ID
		scores[i] = float64(h.Score)
	}
	return map[string]any{"ids": ids, "scores": scores}
}

func searchOpts(a map[string]any) veclite.SearchOptions {
	o := veclite.SearchOptions{Limit: toInt(a["limit"]), Filter: filterMap(a["filter"])}
	if v, ok := a["ef_search"]; ok && v != nil {
		n := toInt(v)
		o.EfSearch = &n
	}
	if v, ok := a["with_payload"]; ok && v != nil {
		b := boolOf(v)
		o.WithPayload = &b
	}
	if v, ok := a["with_vector"]; ok && v != nil {
		b := boolOf(v)
		o.WithVector = &b
	}
	return o
}

func hybridOpts(a map[string]any) veclite.HybridOptions {
	o := veclite.HybridOptions{Limit: toInt(a["limit"]), Text: str(a["text"]), Filter: filterMap(a["filter"])}
	if v, ok := a["dense"]; ok && v != nil {
		o.Dense = floats(v)
	}
	if v := sparse(a["sparse"]); v != nil {
		o.Sparse = v
	}
	if v, ok := a["alpha"]; ok && v != nil {
		f := float32(toFloat(v))
		o.Alpha = &f
	}
	if v, ok := a["rrf_k"]; ok && v != nil {
		f := float32(toFloat(v))
		o.RRFK = &f
	}
	return o
}

// ── comparison (golden with numeric tolerance 1e-5) ──────────────────────────

// canonical JSON-normalizes an observation so it compares against golden values
// (which are JSON): all numbers become float64, structs become maps/slices.
func canonical(m map[string]any) any {
	b, _ := json.Marshal(m)
	var v any
	_ = json.Unmarshal(b, &v)
	return v
}

func eqTol(want, got any) bool {
	switch w := want.(type) {
	case map[string]any:
		g, ok := got.(map[string]any)
		if !ok || len(g) != len(w) {
			return false
		}
		for k, wv := range w {
			gv, ok := g[k]
			if !ok || !eqTol(wv, gv) {
				return false
			}
		}
		return true
	case []any:
		g, ok := got.([]any)
		if !ok || len(g) != len(w) {
			return false
		}
		for i := range w {
			if !eqTol(w[i], g[i]) {
				return false
			}
		}
		return true
	case float64:
		gf, ok := numeric(got)
		return ok && math.Abs(w-gf) <= tol
	case nil:
		return got == nil
	case bool:
		gb, ok := got.(bool)
		return ok && gb == w
	case string:
		gs, ok := got.(string)
		return ok && gs == w
	default:
		return false
	}
}

// matchesSubset checks that want (from `expect`) is a subset of got: object keys
// present and matching, arrays same length, scalars equal within tolerance.
func matchesSubset(want, got any) bool {
	want = normalize(want)
	got = normalize(got)
	switch w := want.(type) {
	case map[string]any:
		g, ok := got.(map[string]any)
		if !ok {
			return false
		}
		for k, wv := range w {
			if !matchesSubset(wv, g[k]) {
				return false
			}
		}
		return true
	case []any:
		g, ok := got.([]any)
		if !ok || len(g) != len(w) {
			return false
		}
		for i := range w {
			if !matchesSubset(w[i], g[i]) {
				return false
			}
		}
		return true
	case float64:
		gf, ok := numeric(got)
		return ok && math.Abs(w-gf) <= tol
	default:
		return eqTol(want, got)
	}
}

func normalize(v any) any {
	b, _ := json.Marshal(v)
	var out any
	_ = json.Unmarshal(b, &out)
	return out
}

func numeric(v any) (float64, bool) {
	switch n := v.(type) {
	case float64:
		return n, true
	case float32:
		return float64(n), true
	case int:
		return float64(n), true
	case int64:
		return float64(n), true
	default:
		return 0, false
	}
}
