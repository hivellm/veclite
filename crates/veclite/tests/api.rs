//! API surface (SPEC-004): scroll pagination totality (API-022), search_batch
//! (FR-35), and collection stats (FR-08/13).

use veclite::{
    CollectionOptions, Condition, Filter, Metric, Point, Quantization, VecLite, VecLiteError,
};

fn coll(n: u32) -> (VecLite, veclite::Collection) {
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "docs",
            CollectionOptions::new(2, Metric::Euclidean).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    for i in 0..n {
        #[allow(clippy::cast_precision_loss)]
        c.upsert(Point::new(format!("k{i}"), vec![i as f32, 0.0]))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    (db, c)
}

#[test]
fn scroll_covers_every_live_vector_exactly_once() {
    let (_db, c) = coll(50);
    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = c
            .scroll(cursor.as_deref(), 7, None)
            .unwrap_or_else(|e| panic!("{e}"));
        for p in &page.points {
            seen.push(p.id.clone());
        }
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 50, "every live vector covered exactly once");
    // No page exceeded the limit.
    let page = c.scroll(None, 7, None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(page.points.len(), 7);
}

#[test]
fn scroll_skips_deleted_and_stays_stable() {
    let (_db, c) = coll(20);
    for i in (0..20).step_by(2) {
        c.delete(&format!("k{i}")).unwrap_or_else(|e| panic!("{e}")); // delete evens
    }
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = c
            .scroll(cursor.as_deref(), 3, None)
            .unwrap_or_else(|e| panic!("{e}"));
        seen.extend(page.points.iter().map(|p| p.id.clone()));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 10, "only the 10 odd ids remain");
    assert!(seen.iter().all(|id| {
        let n: u32 = id.trim_start_matches('k').parse().unwrap_or(0);
        n % 2 == 1
    }));
}

#[test]
fn filtered_scroll_restricts_the_page() {
    let db = VecLite::memory();
    let c = db
        .create_collection(
            "docs",
            CollectionOptions::new(2, Metric::Euclidean).quantization(Quantization::None),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    for i in 0..10u32 {
        let lang = if i % 2 == 0 { "en" } else { "pt" };
        #[allow(clippy::cast_precision_loss)]
        c.upsert(
            Point::new(format!("k{i}"), vec![i as f32, 0.0])
                .payload(serde_json::json!({"lang": lang})),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    }
    let f = Filter::new().must(Condition::eq("lang", "en"));
    let mut total = 0;
    let mut cursor = None;
    loop {
        let page = c
            .scroll(cursor.as_deref(), 2, Some(&f))
            .unwrap_or_else(|e| panic!("{e}"));
        total += page.points.len();
        assert!(page.points.iter().all(|p| {
            p.payload
                .as_ref()
                .and_then(|v| v.get("lang"))
                .and_then(|v| v.as_str())
                == Some("en")
        }));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(total, 5, "5 en documents");
}

#[test]
fn search_batch_matches_individual_search() {
    let (_db, c) = coll(30);
    let queries = vec![vec![0.0, 0.0], vec![15.0, 0.0], vec![29.0, 0.0]];
    let batch = c.search_batch(&queries, 3);
    assert_eq!(batch.len(), 3);
    for (q, res) in queries.iter().zip(&batch) {
        let one = c.search(q, 3).unwrap_or_else(|e| panic!("{e}"));
        let got = res.as_ref().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(got, &one);
    }
}

#[test]
fn alias_blue_green_swap_resolves_transparently() {
    let db = VecLite::memory();
    let v1 = db
        .create_collection("docs_v1", opts_e())
        .unwrap_or_else(|e| panic!("{e}"));
    v1.upsert(Point::new("old", vec![1.0, 0.0]))
        .unwrap_or_else(|e| panic!("{e}"));
    let v2 = db
        .create_collection("docs_v2", opts_e())
        .unwrap_or_else(|e| panic!("{e}"));
    v2.upsert(Point::new("new", vec![1.0, 0.0]))
        .unwrap_or_else(|e| panic!("{e}"));

    db.create_alias("docs", "docs_v1")
        .unwrap_or_else(|e| panic!("{e}"));
    let hits = db
        .collection("docs")
        .unwrap_or_else(|e| panic!("{e}"))
        .search(&[1.0, 0.0], 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "old");

    // Swap the alias to v2 (delete then re-create) — callers keep using "docs".
    db.delete_alias("docs").unwrap_or_else(|e| panic!("{e}"));
    db.create_alias("docs", "docs_v2")
        .unwrap_or_else(|e| panic!("{e}"));
    let hits = db
        .collection("docs")
        .unwrap_or_else(|e| panic!("{e}"))
        .search(&[1.0, 0.0], 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "new");
    assert_eq!(
        db.aliases(),
        vec![("docs".to_owned(), "docs_v2".to_owned())]
    );
}

#[test]
fn alias_errors() {
    let db = VecLite::memory();
    db.create_collection("real", opts_e())
        .unwrap_or_else(|e| panic!("{e}"));
    // Missing target.
    assert!(matches!(
        db.create_alias("a", "ghost"),
        Err(VecLiteError::CollectionNotFound(_))
    ));
    db.create_alias("a", "real")
        .unwrap_or_else(|e| panic!("{e}"));
    // Duplicate alias, and an alias shadowing a real collection.
    assert!(matches!(
        db.create_alias("a", "real"),
        Err(VecLiteError::AlreadyExists(_))
    ));
    assert!(matches!(
        db.create_alias("real", "real"),
        Err(VecLiteError::AlreadyExists(_))
    ));
    // Deleting the target collection drops the alias.
    db.delete_collection("real")
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(db.aliases().is_empty());
}

#[test]
fn aliases_survive_reopen() {
    let path = std::env::temp_dir().join(format!("veclite-alias-{}.veclite", std::process::id()));
    let mut wal = path.clone().into_os_string();
    wal.push("-wal");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&wal);
    {
        let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
        let c = db
            .create_collection("docs_v2", opts_e())
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("x", vec![1.0, 0.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_alias("docs", "docs_v2")
            .unwrap_or_else(|e| panic!("{e}"));
        db.checkpoint().unwrap_or_else(|e| panic!("{e}"));
    }
    let db = VecLite::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        db.aliases(),
        vec![("docs".to_owned(), "docs_v2".to_owned())]
    );
    let hits = db
        .collection("docs")
        .unwrap_or_else(|e| panic!("{e}"))
        .search(&[1.0, 0.0], 1)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hits[0].id, "x");
    drop(db);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&wal);
}

fn opts_e() -> CollectionOptions {
    CollectionOptions::new(2, Metric::Euclidean).quantization(Quantization::None)
}

#[test]
fn stats_report_live_and_tombstones() {
    let (_db, c) = coll(10);
    c.delete("k0").unwrap_or_else(|e| panic!("{e}"));
    c.delete("k1").unwrap_or_else(|e| panic!("{e}"));
    let s = c.stats();
    assert_eq!(s.name, "docs");
    assert_eq!(s.dimension, 2);
    assert_eq!(s.len, 8);
    assert_eq!(s.tombstones, 2);
    assert!(!s.auto_embed);
}
