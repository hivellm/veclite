//! FFI coverage for the phase4g surface (SPEC-008 §2 full surface): batch
//! upsert/delete, batch search + `vl_hits_batch`, hybrid search, scroll +
//! `vl_page`, chunk, reindex/refit, payload-index creation, and database
//! snapshot/vacuum/info. Each test drives the `extern "C"` surface exactly as a
//! C caller would.

use std::ffi::{CStr, CString};
use std::ptr;

use veclite_ffi::*;

fn cs(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| panic!("nul in {s}"))
}

unsafe fn memory_db() -> *mut vl_db {
    let mut db: *mut vl_db = ptr::null_mut();
    assert_eq!(unsafe { vl_open_memory(&mut db) }, VL_OK);
    db
}

unsafe fn create(db: *mut vl_db, name: &str, opts_json: &str) -> *mut vl_collection {
    let cname = cs(name);
    let opts = opts_json.as_bytes();
    let mut coll: *mut vl_collection = ptr::null_mut();
    assert_eq!(
        unsafe {
            vl_collection_create(
                db,
                cname.as_ptr(),
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                &mut coll,
            )
        },
        VL_OK,
    );
    coll
}

unsafe fn count(coll: *mut vl_collection) -> u64 {
    let mut n: u64 = 0;
    assert_eq!(unsafe { vl_count(coll, &mut n) }, VL_OK);
    n
}

#[test]
fn upsert_batch_then_delete_batch() {
    unsafe {
        let db = memory_db();
        let coll = create(
            db,
            "v",
            r#"{"dimension":2,"metric":"euclidean","quantization_bits":0}"#,
        );

        let points = br#"[
            {"id":"a","vector":[0.0,0.0],"payload":{"n":1}},
            {"id":"b","vector":[1.0,0.0]},
            {"id":"c","vector":[0.0,1.0],"payload":{"n":3}}
        ]"#;
        assert_eq!(
            vl_upsert_batch(coll, points.as_ptr(), points.len(), VL_CODEC_JSON),
            VL_OK,
        );
        assert_eq!(count(coll), 3);

        let ids = br#"["a","c","missing"]"#;
        let mut deleted: u64 = 0;
        assert_eq!(
            vl_delete_batch(coll, ids.as_ptr(), ids.len(), VL_CODEC_JSON, &mut deleted),
            VL_OK,
        );
        assert_eq!(deleted, 2, "only a and c existed");
        assert_eq!(count(coll), 1);

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn search_batch_reports_per_query_hits_and_errors() {
    unsafe {
        let db = memory_db();
        let coll = create(
            db,
            "v",
            r#"{"dimension":2,"metric":"euclidean","quantization_bits":0}"#,
        );
        let points = br#"[{"id":"a","vector":[0.0,0.0]},{"id":"b","vector":[5.0,5.0]}]"#;
        assert_eq!(
            vl_upsert_batch(coll, points.as_ptr(), points.len(), VL_CODEC_JSON),
            VL_OK,
        );

        // Two well-formed 2-d queries in one flat (n=2, dim=2) buffer.
        let vecs = [0.0f32, 0.0, 5.0, 5.0];
        let mut batch: *mut vl_hits_batch = ptr::null_mut();
        assert_eq!(
            vl_search_batch(
                coll,
                vecs.as_ptr(),
                2,
                2,
                1,
                ptr::null(),
                0,
                VL_CODEC_JSON,
                &mut batch,
            ),
            VL_OK,
        );
        assert_eq!(vl_hits_batch_len(batch), 2);

        // Query 0 → nearest is "a".
        assert_eq!(vl_hits_batch_code(batch, 0), VL_OK);
        assert_eq!(vl_hits_batch_hits_len(batch, 0), 1);
        let mut view = std::mem::zeroed::<vl_hit_view>();
        assert_eq!(vl_hits_batch_hit(batch, 0, 0, &mut view), VL_OK);
        assert_eq!(CStr::from_ptr(view.id).to_string_lossy(), "a");

        // Query 1 → nearest is "b".
        assert_eq!(vl_hits_batch_hit(batch, 1, 0, &mut view), VL_OK);
        assert_eq!(CStr::from_ptr(view.id).to_string_lossy(), "b");

        // Out-of-range indices are rejected, not UB.
        assert_eq!(vl_hits_batch_code(batch, 9), VL_ERR_INVALID_ARGUMENT);
        assert_eq!(
            vl_hits_batch_hit(batch, 9, 0, &mut view),
            VL_ERR_INVALID_ARGUMENT
        );
        vl_hits_batch_free(batch);

        // A wrong-dimension batch: every item reports the mismatch per-query,
        // not as a whole-call failure.
        let wrong = [1.0f32, 2.0, 3.0];
        let mut bad: *mut vl_hits_batch = ptr::null_mut();
        assert_eq!(
            vl_search_batch(
                coll,
                wrong.as_ptr(),
                1,
                3,
                1,
                ptr::null(),
                0,
                VL_CODEC_JSON,
                &mut bad,
            ),
            VL_OK,
        );
        assert_eq!(vl_hits_batch_len(bad), 1);
        assert_eq!(vl_hits_batch_code(bad, 0), VL_ERR_DIMENSION_MISMATCH);
        assert_eq!(vl_hits_batch_hits_len(bad, 0), 0);
        vl_hits_batch_free(bad);

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn hybrid_search_dense_and_text() {
    unsafe {
        let db = memory_db();
        // Auto-embed collection so text and dense both have meaning.
        let coll = create(
            db,
            "docs",
            r#"{"dimension":64,"embedding_provider":"bm25"}"#,
        );
        for (id, text) in [("d1", "the quick brown fox"), ("d2", "a lazy sleeping dog")] {
            let cid = cs(id);
            let ctext = cs(text);
            assert_eq!(
                vl_upsert_text(
                    coll,
                    cid.as_ptr(),
                    ctext.as_ptr(),
                    ptr::null(),
                    0,
                    VL_CODEC_JSON
                ),
                VL_OK,
            );
        }

        // A text-only hybrid query fuses the lexical channel; at least one hit.
        let opts = br#"{"text":"quick fox","limit":2,"with_payload":true}"#;
        let mut hits: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_hybrid_search(coll, opts.as_ptr(), opts.len(), VL_CODEC_JSON, &mut hits),
            VL_OK,
        );
        assert!(vl_hits_len(hits) >= 1);
        vl_hits_free(hits);

        // No channel at all is rejected.
        let empty = br#"{"limit":2}"#;
        let mut none: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_hybrid_search(coll, empty.as_ptr(), empty.len(), VL_CODEC_JSON, &mut none),
            VL_ERR_INVALID_ARGUMENT,
        );

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn scroll_paginates_the_whole_collection() {
    unsafe {
        let db = memory_db();
        let coll = create(
            db,
            "v",
            r#"{"dimension":2,"metric":"euclidean","quantization_bits":0}"#,
        );
        for i in 0..5 {
            let id = cs(&format!("k{i}"));
            let vec = [i as f32, 0.0f32];
            assert_eq!(
                vl_upsert(
                    coll,
                    id.as_ptr(),
                    vec.as_ptr(),
                    2,
                    ptr::null(),
                    0,
                    VL_CODEC_JSON
                ),
                VL_OK,
            );
        }

        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            // scroll_opts blob: { limit, cursor? } — cursor absent on page 1.
            let opts = match &cursor {
                Some(c) => format!(r#"{{"limit":2,"cursor":"{c}"}}"#),
                None => r#"{"limit":2}"#.to_owned(),
            };
            let obytes = opts.as_bytes();
            let mut page: *mut vl_page = ptr::null_mut();
            assert_eq!(
                vl_scroll(
                    coll,
                    obytes.as_ptr(),
                    obytes.len(),
                    VL_CODEC_JSON,
                    &mut page,
                ),
                VL_OK,
            );
            let n = vl_page_len(page);
            for i in 0..n {
                let mut buf = vl_buf {
                    data: ptr::null_mut(),
                    len: 0,
                };
                assert_eq!(vl_page_point(page, i, &mut buf), VL_OK);
                let bytes = std::slice::from_raw_parts(buf.data, buf.len);
                let v: serde_json::Value =
                    serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{e}"));
                seen.push(v["id"].as_str().unwrap_or("").to_owned());
                vl_buf_free(&mut buf);
            }
            let next = vl_page_cursor(page);
            let done = next.is_null();
            cursor = (!next.is_null()).then(|| CStr::from_ptr(next).to_string_lossy().into_owned());
            vl_page_free(page);
            if done {
                break;
            }
        }
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 5, "every point was scrolled exactly once");

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn chunk_splits_text_with_offsets() {
    unsafe {
        let text = cs("abcdefghijklmnopqrstuvwxyz0123456789");
        let opts = br#"{"max_chars":10,"overlap":2}"#;
        let mut buf = vl_buf {
            data: ptr::null_mut(),
            len: 0,
        };
        assert_eq!(
            vl_chunk(
                text.as_ptr(),
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                &mut buf
            ),
            VL_OK,
        );
        let bytes = std::slice::from_raw_parts(buf.data, buf.len);
        let chunks: serde_json::Value =
            serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{e}"));
        let arr = chunks.as_array().unwrap_or_else(|| panic!("array"));
        assert!(arr.len() >= 2, "long text splits into multiple chunks");
        for ch in arr {
            assert!(ch.get("text").is_some());
            assert!(ch.get("start").is_some());
            assert!(ch.get("end").is_some());
        }
        vl_buf_free(&mut buf);
    }
}

#[test]
fn reindex_and_refit_run() {
    unsafe {
        let db = memory_db();
        let vec_coll = create(
            db,
            "v",
            r#"{"dimension":2,"metric":"euclidean","quantization_bits":0}"#,
        );
        let id = cs("a");
        let vec = [1.0f32, 0.0];
        assert_eq!(
            vl_upsert(
                vec_coll,
                id.as_ptr(),
                vec.as_ptr(),
                2,
                ptr::null(),
                0,
                VL_CODEC_JSON
            ),
            VL_OK,
        );
        assert_eq!(vl_collection_reindex(vec_coll), VL_OK);
        // refit on a non-auto-embed collection is rejected (InvalidArgument).
        assert_eq!(vl_collection_refit(vec_coll), VL_ERR_INVALID_ARGUMENT);
        vl_collection_free(vec_coll);

        let txt = create(
            db,
            "docs",
            r#"{"dimension":64,"embedding_provider":"bm25"}"#,
        );
        let tid = cs("d1");
        let ttext = cs("hello world");
        assert_eq!(
            vl_upsert_text(
                txt,
                tid.as_ptr(),
                ttext.as_ptr(),
                ptr::null(),
                0,
                VL_CODEC_JSON
            ),
            VL_OK,
        );
        assert_eq!(vl_collection_refit(txt), VL_OK);
        vl_collection_free(txt);

        vl_db_close(db);
    }
}

#[test]
fn payload_index_create_shows_in_db_info() {
    unsafe {
        let db = memory_db();
        let coll = create(
            db,
            "v",
            r#"{"dimension":2,"metric":"euclidean","quantization_bits":0}"#,
        );
        let field = cs("lang");
        assert_eq!(
            vl_payload_index_create(coll, field.as_ptr(), VL_PIDX_KEYWORD),
            VL_OK
        );
        // An unknown kind byte is rejected.
        assert_eq!(
            vl_payload_index_create(coll, field.as_ptr(), 99),
            VL_ERR_INVALID_ARGUMENT,
        );
        vl_collection_free(coll);

        let mut buf = vl_buf {
            data: ptr::null_mut(),
            len: 0,
        };
        assert_eq!(vl_db_info(db, VL_CODEC_JSON, &mut buf), VL_OK);
        let bytes = std::slice::from_raw_parts(buf.data, buf.len);
        let info: serde_json::Value =
            serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(info["format_version"].as_u64(), Some(1));
        let cols = info["collections"]
            .as_array()
            .unwrap_or_else(|| panic!("collections"));
        let v = cols
            .iter()
            .find(|c| c["name"] == "v")
            .unwrap_or_else(|| panic!("collection v"));
        let idx = v["payload_indexes"]
            .as_array()
            .unwrap_or_else(|| panic!("indexes"));
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0][0].as_str(), Some("lang"));
        assert_eq!(idx[0][1].as_str(), Some("keyword"));
        vl_buf_free(&mut buf);

        vl_db_close(db);
    }
}

#[test]
fn db_snapshot_and_vacuum() {
    unsafe {
        let db = memory_db();
        let coll = create(
            db,
            "v",
            r#"{"dimension":2,"metric":"euclidean","quantization_bits":0}"#,
        );
        let id = cs("a");
        let vec = [1.0f32, 0.0];
        assert_eq!(
            vl_upsert(
                coll,
                id.as_ptr(),
                vec.as_ptr(),
                2,
                ptr::null(),
                0,
                VL_CODEC_JSON
            ),
            VL_OK,
        );
        vl_collection_free(coll);

        assert_eq!(vl_db_vacuum(db), VL_OK);

        let dir = std::env::temp_dir();
        let path = dir.join(format!("veclite-ffi-snap-{}.veclite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let cpath = cs(&path.to_string_lossy());
        assert_eq!(vl_db_snapshot(db, cpath.as_ptr()), VL_OK);
        assert!(path.exists(), "snapshot wrote the file");

        // Reopen the snapshot and confirm the point survived.
        let mut reopened: *mut vl_db = ptr::null_mut();
        assert_eq!(
            vl_open(cpath.as_ptr(), ptr::null(), 0, &mut reopened),
            VL_OK
        );
        let mut got: *mut vl_collection = ptr::null_mut();
        let vname = cs("v");
        assert_eq!(vl_collection_get(reopened, vname.as_ptr(), &mut got), VL_OK);
        assert_eq!(count(got), 1);
        vl_collection_free(got);
        vl_db_close(reopened);
        vl_db_close(db);
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn search_text_query_opts_strip_payload() {
    unsafe {
        let db = memory_db();
        let coll = create(
            db,
            "docs",
            r#"{"dimension":64,"embedding_provider":"bm25"}"#,
        );
        let cid = cs("d1");
        let ctext = cs("quick brown fox");
        let payload = br#"{"tag":"animal"}"#;
        assert_eq!(
            vl_upsert_text(
                coll,
                cid.as_ptr(),
                ctext.as_ptr(),
                payload.as_ptr(),
                payload.len(),
                VL_CODEC_JSON
            ),
            VL_OK,
        );

        let q = cs("quick fox");
        // with_payload:false strips the payload from the returned view.
        let opts = br#"{"with_payload":false}"#;
        let mut hits: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_search_text(
                coll,
                q.as_ptr(),
                1,
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                &mut hits
            ),
            VL_OK,
        );
        let mut view = std::mem::zeroed::<vl_hit_view>();
        assert_eq!(vl_hits_get(hits, 0, &mut view), VL_OK);
        assert!(
            view.payload.is_null(),
            "with_payload:false dropped the payload"
        );
        vl_hits_free(hits);

        // filter on text search is rejected, pointing at hybrid.
        let filtered = br#"{"filter":{"must":[]}}"#;
        let mut nope: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_search_text(
                coll,
                q.as_ptr(),
                1,
                filtered.as_ptr(),
                filtered.len(),
                VL_CODEC_JSON,
                &mut nope
            ),
            VL_ERR_INVALID_ARGUMENT,
        );

        vl_collection_free(coll);
        vl_db_close(db);
    }
}
