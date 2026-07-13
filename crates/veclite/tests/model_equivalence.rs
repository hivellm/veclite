//! Property test (task 2.1): arbitrary CRUD sequences applied to a live
//! collection must stay state-equivalent to a `HashMap` model.
//!
//! Euclidean + `Quantization::None` is chosen so `get` returns the ingested
//! vector bit-for-bit — no cosine normalization, no quantization rounding —
//! which lets the model compare vectors by exact equality.

use std::collections::HashMap;

use proptest::prelude::*;
use veclite::{CollectionOptions, Metric, Point, Quantization, VecLite};

const DIM: usize = 4;
/// Small id alphabet so upserts and deletes collide, exercising the
/// replace-existing and delete-existing paths rather than always appending.
const IDS: &[&str] = &["a", "b", "c", "d", "e"];

#[derive(Clone, Debug)]
enum Op {
    Upsert(String, Vec<f32>),
    Delete(String),
    UpsertBatch(Vec<(String, Vec<f32>)>),
    DeleteBatch(Vec<String>),
}

fn id_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(IDS).prop_map(str::to_owned)
}

fn vector_strategy() -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(-1_000.0f32..1_000.0, DIM)
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (id_strategy(), vector_strategy()).prop_map(|(id, v)| Op::Upsert(id, v)),
        id_strategy().prop_map(Op::Delete),
        prop::collection::vec((id_strategy(), vector_strategy()), 0..4).prop_map(Op::UpsertBatch),
        prop::collection::vec(id_strategy(), 0..4).prop_map(Op::DeleteBatch),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn crud_matches_model_hashmap(ops in prop::collection::vec(op_strategy(), 0..200)) {
        let db = VecLite::memory();
        let c = db
            .create_collection(
                "t",
                CollectionOptions::new(DIM, Metric::Euclidean).quantization(Quantization::None),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        let mut model: HashMap<String, Vec<f32>> = HashMap::new();

        for op in ops {
            match op {
                Op::Upsert(id, v) => {
                    c.upsert(Point::new(id.clone(), v.clone()))
                        .unwrap_or_else(|e| panic!("{e}"));
                    model.insert(id, v);
                }
                Op::Delete(id) => {
                    let engine = c.delete(&id).unwrap_or_else(|e| panic!("{e}"));
                    let model_removed = model.remove(&id).is_some();
                    prop_assert_eq!(engine, model_removed);
                }
                Op::UpsertBatch(items) => {
                    let points = items
                        .iter()
                        .map(|(id, v)| Point::new(id.clone(), v.clone()))
                        .collect();
                    c.upsert_batch(points).unwrap_or_else(|e| panic!("{e}"));
                    // A batch may repeat an id; last write wins — matching the
                    // engine, which tombstones the earlier slot in order.
                    for (id, v) in items {
                        model.insert(id, v);
                    }
                }
                Op::DeleteBatch(ids) => {
                    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                    let engine = c.delete_batch(&refs).unwrap_or_else(|e| panic!("{e}"));
                    // Count existing at the moment of each deletion, so a
                    // repeated id counts once — the engine's semantics.
                    let mut model_n = 0usize;
                    for id in &ids {
                        if model.remove(id).is_some() {
                            model_n += 1;
                        }
                    }
                    prop_assert_eq!(engine, model_n);
                }
            }
            prop_assert_eq!(c.len(), model.len());
        }

        // Full-state equivalence over the entire key space: present ids match
        // by exact vector, absent ids return `None`.
        for &id in IDS {
            let got = c.get(id).unwrap_or_else(|e| panic!("{e}")).map(|p| p.vector);
            prop_assert_eq!(got.as_ref(), model.get(id));
        }
    }
}
