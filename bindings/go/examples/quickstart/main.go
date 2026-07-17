// Quickstart for the veclite-go binding. Build with a C toolchain:
//
//	CC="zig cc" go run .
package main

import (
	"fmt"
	"log"

	"github.com/hivellm/veclite-go"
)

func main() {
	db := veclite.Memory()
	defer db.Close()

	bits := uint8(0)
	docs, err := db.CreateCollection("docs", veclite.CollectionOptions{
		Dimension: 3, Metric: veclite.Euclidean, QuantizationBits: &bits,
	})
	if err != nil {
		log.Fatal(err)
	}
	defer docs.Close()

	if err := docs.Upsert(veclite.Point{ID: "a", Vector: []float32{1, 0, 0}, Payload: map[string]any{"lang": "en"}}); err != nil {
		log.Fatal(err)
	}
	if err := docs.Upsert(veclite.Point{ID: "b", Vector: []float32{0, 1, 0}}); err != nil {
		log.Fatal(err)
	}

	hits, err := docs.Search([]float32{0.9, 0.1, 0}, veclite.SearchOptions{Limit: 5})
	if err != nil {
		log.Fatal(err)
	}
	for _, h := range hits {
		fmt.Printf("%s score=%.4f payload=%v\n", h.ID, h.Score, h.Payload)
	}
	fmt.Printf("veclite %s (abi %d, format %d)\n", veclite.Version(), veclite.AbiVersion(), veclite.FormatVersion())
}
