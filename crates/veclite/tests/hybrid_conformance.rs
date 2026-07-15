//! Hybrid RRF conformance corpus (SPEC-007 HYB-020/021/022, gate G3).
//!
//! The fixture pins the **fused ranking** (not just the set) for a range of
//! (alpha, dense-order, sparse-subset) scenarios, computed from the SPEC-007
//! normative formula
//!   `fused = alpha/(rrf_k + dense_rank) + (1-alpha)/(rrf_k + sparse_rank)`,
//! ordered by fused score desc, ties by dense rank asc then id bytewise.
//!
//! NOTE ON SERVER PARITY: SPEC-007 fixes VecLite's fusion as **pure rank-based
//! RRF**. The Vectorizer server ships two divergent hybrid functions
//! (`db/hybrid_search.rs`, `discovery/hybrid.rs`) that each add a raw-score
//! term (`rrf_score + score * alpha`) and disagree with one another; pure RRF
//! is the deterministic, corpus-independent choice SPEC-007 standardizes on, so
//! this corpus pins that formula. Any drift in VecLite's fusion breaks it.

#![cfg(not(target_arch = "wasm32"))]

use veclite::{CollectionOptions, Metric, Point, Quantization, SparseVector, VecLite};

const FIXTURE: &str = include_str!("fixtures/hybrid_rrf_conformance.json");

#[test]
fn fused_rankings_match_the_conformance_corpus() {
    let scenarios: serde_json::Value =
        serde_json::from_str(FIXTURE).unwrap_or_else(|e| panic!("fixture: {e}"));
    let db = VecLite::memory();

    for (n, sc) in scenarios
        .as_array()
        .unwrap_or_else(|| panic!("array"))
        .iter()
        .enumerate()
    {
        let name = sc["name"].as_str().unwrap_or("?");
        let alpha = sc["alpha"].as_f64().unwrap_or(0.5) as f32;
        let dense: Vec<String> = sc["dense"]
            .as_array()
            .unwrap_or_else(|| panic!("dense"))
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_owned())
            .collect();
        let sparse: Vec<String> = sc["sparse"]
            .as_array()
            .unwrap_or_else(|| panic!("sparse"))
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_owned())
            .collect();
        let expected: Vec<String> = sc["expected"]
            .as_array()
            .unwrap_or_else(|| panic!("expected"))
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_owned())
            .collect();

        let c = db
            .create_collection(
                &format!("s{n}"),
                CollectionOptions::new(1, Metric::Euclidean).quantization(Quantization::None),
            )
            .unwrap_or_else(|e| panic!("{e}"));

        // Dense ranking: doc at position r (1-based) sits at distance r from the
        // origin query, so the dense lane ranks them in `dense` order.
        // Sparse ranking: docs in `sparse` carry a single term (index 0) with a
        // weight descending by their sparse rank; docs absent from `sparse` get
        // no sparse lane, so they never enter the sparse ranking.
        let sparse_rank: std::collections::HashMap<&str, usize> = sparse
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        for (i, id) in dense.iter().enumerate() {
            let mut p = Point::new(id.clone(), vec![(i + 1) as f32]);
            if let Some(&sr) = sparse_rank.get(id.as_str()) {
                let weight = (sparse.len() - sr) as f32; // rank 0 -> highest
                p = p.sparse(SparseVector {
                    indices: vec![0],
                    values: vec![weight],
                });
            }
            c.upsert(p).unwrap_or_else(|e| panic!("{e}"));
        }

        let got: Vec<String> = c
            .hybrid_query()
            .dense(&[0.0])
            .sparse(&SparseVector {
                indices: vec![0],
                values: vec![1.0],
            })
            .alpha(alpha)
            .rrf_k(60.0)
            .limit(dense.len())
            .run()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .map(|h| h.id)
            .collect();

        assert_eq!(got, expected, "scenario {name:?} (alpha={alpha})");
    }
}

#[test]
fn hybrid_ordering_is_deterministic_across_repeats() {
    // HYB-021: identical queries return identical orderings, run to run.
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "d",
            CollectionOptions::new(2, Metric::Euclidean).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    for i in 0..12u32 {
        c.upsert(
            Point::new(format!("k{i}"), vec![i as f32, (i % 3) as f32]).sparse(SparseVector {
                indices: vec![i % 4],
                values: vec![1.0 + (i % 5) as f32],
            }),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    let run = || {
        c.hybrid_query()
            .dense(&[2.0, 1.0])
            .sparse(&SparseVector {
                indices: vec![0, 2],
                values: vec![1.0, 2.0],
            })
            .limit(6)
            .run()
            .unwrap_or_else(|e| panic!("{e}"))
            .into_iter()
            .map(|h| (h.id, format!("{:.6}", h.score)))
            .collect::<Vec<_>>()
    };
    let first = run();
    for _ in 0..5 {
        assert_eq!(run(), first, "hybrid ordering must be deterministic");
    }
}
