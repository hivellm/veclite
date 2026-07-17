package main

import "github.com/hivellm/veclite-go"

// YAML decodes numbers as int or float64 and maps as map[string]any. These
// helpers coerce corpus args into the binding's typed parameters.

func str(v any) string {
	s, _ := v.(string)
	return s
}

func strOr(v any, def string) string {
	if s, ok := v.(string); ok && s != "" {
		return s
	}
	return def
}

func toFloat(v any) float64 {
	switch n := v.(type) {
	case float64:
		return n
	case float32:
		return float64(n)
	case int:
		return float64(n)
	case int64:
		return float64(n)
	default:
		return 0
	}
}

func toInt(v any) int {
	return int(toFloat(v))
}

func toIntOr(v any, def int) int {
	if v == nil {
		return def
	}
	return toInt(v)
}

func boolOf(v any) bool {
	b, _ := v.(bool)
	return b
}

// floats coerces a YAML sequence of numbers into a []float32.
func floats(v any) []float32 {
	seq, ok := v.([]any)
	if !ok {
		return nil
	}
	out := make([]float32, len(seq))
	for i, x := range seq {
		out[i] = float32(toFloat(x))
	}
	return out
}

func uint32s(v any) []uint32 {
	seq, ok := v.([]any)
	if !ok {
		return nil
	}
	out := make([]uint32, len(seq))
	for i, x := range seq {
		out[i] = uint32(toInt(x))
	}
	return out
}

// sparse coerces {indices, values} into a *SparseVector, or nil if absent.
func sparse(v any) *veclite.SparseVector {
	m, ok := v.(map[string]any)
	if !ok || m == nil {
		return nil
	}
	return &veclite.SparseVector{Indices: uint32s(m["indices"]), Values: floats(m["values"])}
}

// filterMap coerces a filter document to map[string]any, or nil if absent.
func filterMap(v any) map[string]any {
	m, ok := v.(map[string]any)
	if !ok {
		return nil
	}
	return m
}

// points coerces a YAML sequence of point objects into []Point.
func points(v any) []veclite.Point {
	seq, ok := v.([]any)
	if !ok {
		return nil
	}
	out := make([]veclite.Point, 0, len(seq))
	for _, x := range seq {
		m, ok := x.(map[string]any)
		if !ok {
			continue
		}
		out = append(out, veclite.Point{
			ID:      str(m["id"]),
			Vector:  floats(m["vector"]),
			Payload: m["payload"],
			Sparse:  sparse(m["sparse"]),
		})
	}
	return out
}
