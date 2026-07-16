//! Full seal → reopen → load round-trip over a *rich* collection: all three
//! payload-index kinds (keyword/integer/float), a per-point sparse lane, JSON
//! payloads, and tombstones. Exercises the seal paths (sparse inverted index,
//! typed posting bytes), every sealed segment type's replay rank, and the load
//! path that rebuilds them (SPEC-002 §3, persist/seal.rs).
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use veclite::{
    CollectionOptions, Condition, Durability, Filter, Metric, OpenOptions, PayloadIndexKind, Point,
    Quantization, SparseVector, VecLite,
};

static N: AtomicU64 = AtomicU64::new(0);

fn tmp() -> PathBuf {
    let n = N.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("veclite-rt-{}-{n}.veclite", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let mut wal = p.file_name().unwrap_or_default().to_os_string();
    wal.push("-wal");
    let _ = std::fs::remove_file(p.with_file_name(wal));
    p
}

#[test]
fn rich_collection_survives_seal_and_reopen() {
    let path = tmp();
    {
        let db = VecLite::open_with(&path, OpenOptions::new().durability(Durability::Full))
            .unwrap_or_else(|e| panic!("{e}"));
        let opts = CollectionOptions::new(2, Metric::Euclidean)
            .quantization(Quantization::None)
            .payload_index("lang", PayloadIndexKind::Keyword)
            .payload_index("year", PayloadIndexKind::Integer)
            .payload_index("price", PayloadIndexKind::Float);
        let c = db
            .create_collection("docs", opts)
            .unwrap_or_else(|e| panic!("{e}"));

        for i in 0..6u32 {
            let payload = serde_json::json!({
                "lang": if i % 2 == 0 { "en" } else { "pt" },
                "year": 2020 + (i as i64 % 3),
                "price": 1.5 * f64::from(i + 1),
            });
            c.upsert(
                Point::new(format!("k{i}"), vec![f32::from(i as u16), 0.0])
                    .payload(payload)
                    .sparse(SparseVector {
                        indices: vec![i % 3],
                        values: vec![1.0 + (i % 4) as f32],
                    }),
            )
            .unwrap_or_else(|e| panic!("{e}"));
        }
        // Tombstone two points so a TOMBSTONE segment is sealed.
        c.delete("k1").unwrap_or_else(|e| panic!("{e}"));
        c.delete("k4").unwrap_or_else(|e| panic!("{e}"));

        db.checkpoint().unwrap_or_else(|e| panic!("{e}")); // seal every segment type
        drop(db);
    }

    // Reopen from the sealed file (no WAL) → the load path rebuilds everything.
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("reopen: {e}"));
    let c = db.collection("docs").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(c.len(), 4, "6 upserts - 2 deletes");

    // Payloads survived.
    let k0 = c
        .get("k0")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("k0"));
    assert_eq!(
        k0.payload
            .as_ref()
            .and_then(|p| p.get("lang"))
            .and_then(|v| v.as_str()),
        Some("en")
    );

    // Keyword index filter.
    let en = c
        .query(&[0.0, 0.0])
        .limit(10)
        .filter(Filter::new().must(Condition::eq("lang", "en")))
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        en.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
        vec!["k0", "k2"]
    ); // k4 deleted

    // Integer range index filter.
    let recent = c
        .query(&[0.0, 0.0])
        .limit(10)
        .filter(Filter::new().must(Condition::range("year", veclite::Range::new().gte(2022.0))))
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!recent.is_empty());

    // Float range index filter.
    let cheap = c
        .query(&[0.0, 0.0])
        .limit(10)
        .filter(Filter::new().must(Condition::range("price", veclite::Range::new().lt(5.0))))
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(!cheap.is_empty());

    // The sparse lane survived: a hybrid query fuses dense + sparse.
    let hy = c
        .hybrid_query()
        .dense(&[0.0, 0.0])
        .sparse(&SparseVector {
            indices: vec![0],
            values: vec![1.0],
        })
        .limit(4)
        .run()
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hy.len(), 4);

    drop(db);
    let _ = std::fs::remove_file(&path);
}
