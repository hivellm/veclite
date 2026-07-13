//! Concurrency smoke test (task 2.2): parallel readers against a single
//! serialized writer on one collection. Also exercises the `Send + Sync +
//! Clone` handle contract (CORE-050) — the handles cross thread boundaries by
//! clone, with no external synchronization.

use std::thread;

use veclite::{CollectionOptions, Metric, Point, Quantization, VecLite};

const N: usize = 500;
const READERS: usize = 4;
const READS_PER_THREAD: usize = 10_000;

#[test]
fn parallel_readers_with_serialized_writer_stay_consistent() {
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "t",
            CollectionOptions::new(4, Metric::Euclidean).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));

    // Single writer (writers serialize on the collection's write lock):
    // insert N points, then delete every even id.
    let writer = {
        let c = c.clone();
        thread::spawn(move || {
            for i in 0..N {
                #[allow(clippy::cast_precision_loss)]
                c.upsert(Point::new(format!("id-{i}"), vec![i as f32, 0.0, 0.0, 0.0]))
                    .unwrap_or_else(|e| panic!("{e}"));
            }
            for i in (0..N).step_by(2) {
                c.delete(&format!("id-{i}"))
                    .unwrap_or_else(|e| panic!("{e}"));
            }
        })
    };

    // Readers run concurrently with the writer. They must never panic or
    // observe a live count outside [0, N] — the only invariant that holds
    // while writes are in flight.
    let readers: Vec<_> = (0..READERS)
        .map(|_| {
            let c = c.clone();
            thread::spawn(move || {
                for _ in 0..READS_PER_THREAD {
                    assert!(c.len() <= N, "live count exceeded the upsert bound");
                    // A concurrent get returns Ok(Some) or Ok(None); both fine.
                    let _ = c.get("id-1").unwrap_or_else(|e| panic!("{e}"));
                }
            })
        })
        .collect();

    writer
        .join()
        .unwrap_or_else(|_| panic!("writer thread panicked"));
    for r in readers {
        r.join()
            .unwrap_or_else(|_| panic!("reader thread panicked"));
    }

    // After the writer joins the state is deterministic: even ids deleted,
    // odd ids present.
    assert_eq!(c.len(), N / 2);
    for i in 0..N {
        let got = c.get(&format!("id-{i}")).unwrap_or_else(|e| panic!("{e}"));
        if i % 2 == 0 {
            assert!(got.is_none(), "even id-{i} should have been deleted");
        } else {
            assert!(got.is_some(), "odd id-{i} should still be present");
        }
    }
}
