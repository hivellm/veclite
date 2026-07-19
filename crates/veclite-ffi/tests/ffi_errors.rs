//! Additional FFI coverage (SPEC-008): null-pointer guards, error-code
//! provocation for every mapped `VL_ERR_*`, the JSON/MessagePack codec
//! branches, and the collection/alias/search entry points not already
//! exercised by `ffi.rs`. Each test drives the `extern "C"` surface exactly as
//! a C caller would.

use std::ffi::{CStr, CString};
use std::ptr;

use veclite_ffi::*;

fn cs(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| panic!("nul in {s}"))
}

fn last_error() -> String {
    unsafe { CStr::from_ptr(vl_last_error_message()) }
        .to_string_lossy()
        .into_owned()
}

fn empty_buf() -> vl_buf {
    vl_buf {
        data: ptr::null_mut(),
        len: 0,
    }
}

#[test]
fn meta_functions_return_the_documented_values() {
    unsafe {
        let v = CStr::from_ptr(vl_version()).to_string_lossy();
        assert_eq!(v, env!("CARGO_PKG_VERSION"));
        assert_eq!(vl_abi_version(), 1);
        assert_eq!(vl_format_version(), 1);
    }
}

#[test]
fn vl_open_memory_rejects_null_out_pointer() {
    unsafe {
        assert_eq!(vl_open_memory(ptr::null_mut()), VL_ERR_INVALID_ARGUMENT);
    }
}

#[test]
fn vl_open_creates_a_file_backed_db_and_checkpoints() {
    unsafe {
        let path =
            std::env::temp_dir().join(format!("veclite-ffi-open-{}.veclite", std::process::id()));
        let mut wal = path.clone().into_os_string();
        wal.push("-wal");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&wal);

        let cpath = cs(path.to_str().unwrap_or_else(|| panic!("non-utf8 path")));
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open(cpath.as_ptr(), ptr::null(), 0, &mut db), VL_OK);
        assert_eq!(vl_db_checkpoint(db), VL_OK);
        assert_eq!(vl_db_close(db), VL_OK);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&wal);
    }
}

#[test]
fn vl_collection_create_rejects_null_out_pointer() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let name = cs("docs");
        let opts = br#"{"dimension":3}"#;
        assert_eq!(
            vl_collection_create(
                db,
                name.as_ptr(),
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                ptr::null_mut(),
            ),
            VL_ERR_INVALID_ARGUMENT
        );
        vl_db_close(db);
    }
}

#[test]
fn vl_collection_create_rejects_an_unknown_metric() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let name = cs("docs");
        let opts = br#"{"dimension":3,"metric":"bogus"}"#;
        let mut coll: *mut vl_collection = ptr::null_mut();
        let code = vl_collection_create(
            db,
            name.as_ptr(),
            opts.as_ptr(),
            opts.len(),
            VL_CODEC_JSON,
            &mut coll,
        );
        assert_eq!(code, VL_ERR_INVALID_ARGUMENT);
        assert!(last_error().contains("bogus"), "message: {}", last_error());
        vl_db_close(db);
    }
}

#[test]
fn vl_collection_create_accepts_the_dot_metric_and_explicit_quantization_bits() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let name = cs("docs");
        let opts = br#"{"dimension":3,"metric":"dot","quantization_bits":8}"#;
        let mut coll: *mut vl_collection = ptr::null_mut();
        assert_eq!(
            vl_collection_create(
                db,
                name.as_ptr(),
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                &mut coll,
            ),
            VL_OK
        );
        vl_collection_free(coll);
        vl_db_close(db);
    }
}

fn create_docs(db: *mut vl_db) -> *mut vl_collection {
    unsafe {
        let name = cs("docs");
        let opts = br#"{"dimension":3}"#;
        let mut coll: *mut vl_collection = ptr::null_mut();
        assert_eq!(
            vl_collection_create(
                db,
                name.as_ptr(),
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                &mut coll,
            ),
            VL_OK
        );
        coll
    }
}

#[test]
fn vl_collection_get_rejects_null_out_pointer_and_fetches_an_existing_collection() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        vl_collection_free(coll);

        let name = cs("docs");
        assert_eq!(
            vl_collection_get(db, name.as_ptr(), ptr::null_mut()),
            VL_ERR_INVALID_ARGUMENT
        );

        let mut fetched: *mut vl_collection = ptr::null_mut();
        assert_eq!(vl_collection_get(db, name.as_ptr(), &mut fetched), VL_OK);
        assert!(!fetched.is_null());
        vl_collection_free(fetched);
        vl_db_close(db);
    }
}

#[test]
fn vl_collection_drop_removes_the_collection() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        vl_collection_free(coll);

        let name = cs("docs");
        assert_eq!(vl_collection_drop(db, name.as_ptr()), VL_OK);

        let mut fetched: *mut vl_collection = ptr::null_mut();
        assert_eq!(
            vl_collection_get(db, name.as_ptr(), &mut fetched),
            VL_ERR_COLLECTION_NOT_FOUND
        );
        vl_db_close(db);
    }
}

#[test]
fn vl_collection_rename_moves_a_collection_to_the_new_name() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        vl_collection_free(coll);

        let from = cs("docs");
        let to = cs("docs2");
        assert_eq!(vl_collection_rename(db, from.as_ptr(), to.as_ptr()), VL_OK);

        let mut fetched: *mut vl_collection = ptr::null_mut();
        assert_eq!(vl_collection_get(db, to.as_ptr(), &mut fetched), VL_OK);
        vl_collection_free(fetched);
        vl_db_close(db);
    }
}

#[test]
fn vl_collections_list_encodes_the_sorted_names() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        vl_collection_free(coll);

        let mut buf = empty_buf();
        assert_eq!(
            vl_collections_list(db, VL_CODEC_JSON, ptr::null_mut()),
            VL_ERR_INVALID_ARGUMENT
        );
        assert_eq!(vl_collections_list(db, VL_CODEC_JSON, &mut buf), VL_OK);
        assert!(buf.len > 0);
        let names: Vec<String> =
            serde_json::from_slice(std::slice::from_raw_parts(buf.data as *const u8, buf.len))
                .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(names, vec!["docs".to_owned()]);
        vl_buf_free(&mut buf);
        vl_db_close(db);
    }
}

#[test]
fn vl_alias_create_and_delete_round_trip() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        vl_collection_free(coll);

        let alias = cs("d");
        let target = cs("docs");
        assert_eq!(vl_alias_create(db, alias.as_ptr(), target.as_ptr()), VL_OK);

        let mut fetched: *mut vl_collection = ptr::null_mut();
        assert_eq!(vl_collection_get(db, alias.as_ptr(), &mut fetched), VL_OK);
        vl_collection_free(fetched);

        assert_eq!(vl_alias_delete(db, alias.as_ptr()), VL_OK);
        let mut missing: *mut vl_collection = ptr::null_mut();
        assert_eq!(
            vl_collection_get(db, alias.as_ptr(), &mut missing),
            VL_ERR_COLLECTION_NOT_FOUND
        );
        vl_db_close(db);
    }
}

#[test]
fn vl_collection_stats_rejects_null_out_and_encodes_the_summary() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);

        assert_eq!(
            vl_collection_stats(coll, VL_CODEC_JSON, ptr::null_mut()),
            VL_ERR_INVALID_ARGUMENT
        );

        let mut buf = empty_buf();
        assert_eq!(vl_collection_stats(coll, VL_CODEC_JSON, &mut buf), VL_OK);
        let value: serde_json::Value =
            serde_json::from_slice(std::slice::from_raw_parts(buf.data as *const u8, buf.len))
                .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(value["name"], "docs");
        assert_eq!(value["dimension"], 3);
        assert_eq!(value["len"], 0);
        vl_buf_free(&mut buf);
        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn vl_upsert_rejects_a_null_vector_pointer() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        let id = cs("a");
        let code = vl_upsert(
            coll,
            id.as_ptr(),
            ptr::null(),
            3,
            ptr::null(),
            0,
            VL_CODEC_JSON,
        );
        assert_eq!(code, VL_ERR_INVALID_ARGUMENT);
        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn vl_upsert_and_vl_get_round_trip_through_the_msgpack_codec() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        let id = cs("a");
        let vec = [1.0f32, 2.0, 3.0];
        let payload = rmp_serde::to_vec_named(&serde_json::json!({"lang": "en"}))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            vl_upsert(
                coll,
                id.as_ptr(),
                vec.as_ptr(),
                3,
                payload.as_ptr(),
                payload.len(),
                VL_CODEC_MSGPACK,
            ),
            VL_OK
        );

        let mut buf = empty_buf();
        assert_eq!(vl_get(coll, id.as_ptr(), VL_CODEC_MSGPACK, &mut buf), VL_OK);
        assert!(buf.len > 0);
        let value: serde_json::Value =
            rmp_serde::from_slice(std::slice::from_raw_parts(buf.data as *const u8, buf.len))
                .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(value["payload"]["lang"], "en");
        vl_buf_free(&mut buf);
        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn vl_upsert_rejects_an_unknown_codec_for_a_non_empty_payload() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        let id = cs("a");
        let vec = [1.0f32, 2.0, 3.0];
        let payload = br#"{"lang":"en"}"#;
        let code = vl_upsert(
            coll,
            id.as_ptr(),
            vec.as_ptr(),
            3,
            payload.as_ptr(),
            payload.len(),
            2,
        );
        assert_eq!(code, VL_ERR_INVALID_ARGUMENT);
        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn vl_get_rejects_null_out_pointer() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        let id = cs("a");
        assert_eq!(
            vl_get(coll, id.as_ptr(), VL_CODEC_JSON, ptr::null_mut()),
            VL_ERR_INVALID_ARGUMENT
        );
        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn vl_delete_reports_whether_the_id_existed() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        let id = cs("a");
        let vec = [1.0f32, 2.0, 3.0];
        assert_eq!(
            vl_upsert(
                coll,
                id.as_ptr(),
                vec.as_ptr(),
                3,
                ptr::null(),
                0,
                VL_CODEC_JSON
            ),
            VL_OK
        );

        let mut existed = false;
        assert_eq!(vl_delete(coll, id.as_ptr(), &mut existed), VL_OK);
        assert!(existed);

        // Deleting again: not present, still VL_OK, `existed` flips to false.
        assert_eq!(vl_delete(coll, id.as_ptr(), &mut existed), VL_OK);
        assert!(!existed);

        // A null `existed` pointer is accepted (write is skipped, not required).
        let missing = cs("nope");
        assert_eq!(vl_delete(coll, missing.as_ptr(), ptr::null_mut()), VL_OK);

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn vl_count_rejects_null_out_pointer() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        assert_eq!(vl_count(coll, ptr::null_mut()), VL_ERR_INVALID_ARGUMENT);
        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn vl_collection_create_rejects_an_unknown_codec_for_the_opts_bytes() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let name = cs("docs");
        let opts = br#"{"dimension":3}"#;
        let mut coll: *mut vl_collection = ptr::null_mut();
        let code = vl_collection_create(db, name.as_ptr(), opts.as_ptr(), opts.len(), 2, &mut coll);
        assert_eq!(code, VL_ERR_INVALID_ARGUMENT);
        assert!(last_error().contains("codec"), "message: {}", last_error());
        vl_db_close(db);
    }
}

// ── Null out-pointer guards on the remaining result-returning entry points ──
//
// Every one of these must report `VL_ERR_INVALID_ARGUMENT` rather than
// dereferencing the null and taking the caller's process down with it. A C
// caller that forgets to check a return code should still get a diagnosable
// failure, not a segfault (SPEC-008 FFI-003).

#[test]
fn search_entry_points_reject_a_null_out_pointer() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        let v = [1.0f32, 0.0, 0.0];

        assert_eq!(
            vl_search(
                coll,
                v.as_ptr(),
                v.len(),
                1,
                ptr::null(),
                0,
                VL_CODEC_JSON,
                ptr::null_mut()
            ),
            VL_ERR_INVALID_ARGUMENT
        );
        let q = cs("anything");
        assert_eq!(
            vl_search_text(
                coll,
                q.as_ptr(),
                1,
                ptr::null(),
                0,
                VL_CODEC_JSON,
                ptr::null_mut()
            ),
            VL_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            vl_search_batch(
                coll,
                v.as_ptr(),
                1,
                v.len(),
                1,
                ptr::null(),
                0,
                VL_CODEC_JSON,
                ptr::null_mut()
            ),
            VL_ERR_INVALID_ARGUMENT
        );
        let opts = br#"{"text":"x"}"#;
        assert_eq!(
            vl_hybrid_search(
                coll,
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                ptr::null_mut()
            ),
            VL_ERR_INVALID_ARGUMENT
        );

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn result_accessors_report_empty_for_a_null_handle() {
    unsafe {
        // The length/cursor accessors return a value, not a status code, so a
        // null handle has to degrade to "empty" — there is no channel to report
        // an error through, and returning garbage would be worse.
        assert_eq!(vl_hits_len(ptr::null()), 0);
        assert_eq!(vl_hits_batch_len(ptr::null()), 0);
        assert_eq!(vl_hits_batch_hits_len(ptr::null(), 0), 0);
        assert_eq!(vl_page_len(ptr::null()), 0);
        assert!(vl_page_cursor(ptr::null()).is_null());

        // The ones that do return a status code report it.
        assert_eq!(
            vl_hits_get(ptr::null(), 0, ptr::null_mut()),
            VL_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            vl_hits_batch_hit(ptr::null(), 0, 0, ptr::null_mut()),
            VL_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            vl_page_point(ptr::null(), 0, ptr::null_mut()),
            VL_ERR_INVALID_ARGUMENT
        );
    }
}

#[test]
fn scroll_chunk_and_db_info_reject_a_null_out_pointer() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);

        let opts = br#"{"limit":10}"#;
        assert_eq!(
            vl_scroll(
                coll,
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                ptr::null_mut()
            ),
            VL_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            vl_db_info(db, VL_CODEC_JSON, ptr::null_mut()),
            VL_ERR_INVALID_ARGUMENT
        );
        let text = cs("some text to split");
        assert_eq!(
            vl_chunk(
                text.as_ptr(),
                ptr::null(),
                0,
                VL_CODEC_JSON,
                ptr::null_mut()
            ),
            VL_ERR_INVALID_ARGUMENT
        );

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn hybrid_search_applies_every_optional_field() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let name = cs("docs");
        // Dimension 3 so the dense channel can be given by hand.
        let copts = br#"{"dimension":3,"metric":"cosine"}"#;
        let mut coll: *mut vl_collection = ptr::null_mut();
        assert_eq!(
            vl_collection_create(
                db,
                name.as_ptr(),
                copts.as_ptr(),
                copts.len(),
                VL_CODEC_JSON,
                &mut coll
            ),
            VL_OK
        );

        for (id, v) in [("a", [1.0f32, 0.0, 0.0]), ("b", [0.0, 1.0, 0.0])] {
            let cid = cs(id);
            let payload = br#"{"lang":"en"}"#;
            assert_eq!(
                vl_upsert(
                    coll,
                    cid.as_ptr(),
                    v.as_ptr(),
                    v.len(),
                    payload.as_ptr(),
                    payload.len(),
                    VL_CODEC_JSON
                ),
                VL_OK
            );
        }

        // Every optional field set at once, so each `if let Some(..)` arm in the
        // options-to-builder translation is taken: dense, sparse, limit, alpha,
        // rrf_k, with_payload, with_vector and filter.
        let opts = br#"{
            "dense":[1.0,0.0,0.0],
            "sparse":{"indices":[0,2],"values":[0.5,0.5]},
            "limit":2,
            "alpha":0.5,
            "rrf_k":60.0,
            "with_payload":true,
            "with_vector":true,
            "filter":{"must":[{"key":"lang","match":{"value":"en"}}]}
        }"#;
        let mut hits: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_hybrid_search(coll, opts.as_ptr(), opts.len(), VL_CODEC_JSON, &mut hits),
            VL_OK,
            "last error: {}",
            last_error()
        );
        assert!(vl_hits_len(hits) >= 1);

        // with_vector was requested, so the borrowed view carries the vector.
        let mut view = std::mem::zeroed::<vl_hit_view>();
        assert_eq!(vl_hits_get(hits, 0, &mut view), VL_OK);
        assert_eq!(view.vector_len, 3);

        vl_hits_free(hits);
        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn search_text_with_vector_backfills_the_stored_vector() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let name = cs("docs");
        let copts = br#"{"dimension":64,"embedding_provider":"bm25"}"#;
        let mut coll: *mut vl_collection = ptr::null_mut();
        assert_eq!(
            vl_collection_create(
                db,
                name.as_ptr(),
                copts.as_ptr(),
                copts.len(),
                VL_CODEC_JSON,
                &mut coll
            ),
            VL_OK
        );
        let id = cs("d1");
        let text = cs("the quick brown fox");
        assert_eq!(
            vl_upsert_text(
                coll,
                id.as_ptr(),
                text.as_ptr(),
                ptr::null(),
                0,
                VL_CODEC_JSON
            ),
            VL_OK
        );

        // The lexical path does not carry vectors through the ranking, so
        // `with_vector` makes the entry point re-read each hit's stored vector.
        let opts = br#"{"with_vector":true}"#;
        let q = cs("quick fox");
        let mut hits: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_search_text(
                coll,
                q.as_ptr(),
                5,
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                &mut hits
            ),
            VL_OK,
            "last error: {}",
            last_error()
        );
        assert!(vl_hits_len(hits) >= 1);
        let mut view = std::mem::zeroed::<vl_hit_view>();
        assert_eq!(vl_hits_get(hits, 0, &mut view), VL_OK);
        assert_eq!(view.vector_len, 64, "with_vector should back-fill");

        vl_hits_free(hits);
        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn search_batch_runs_every_query_and_rejects_null_vectors() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        let coll = create_docs(db);
        for (id, v) in [("a", [1.0f32, 0.0, 0.0]), ("b", [0.0, 1.0, 0.0])] {
            let cid = cs(id);
            assert_eq!(
                vl_upsert(
                    coll,
                    cid.as_ptr(),
                    v.as_ptr(),
                    v.len(),
                    ptr::null(),
                    0,
                    VL_CODEC_JSON
                ),
                VL_OK
            );
        }

        // A non-empty batch drives the per-query loop and the owned-hit
        // conversion; the two queries come back as two independent result sets.
        let queries = [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut batch: *mut vl_hits_batch = ptr::null_mut();
        assert_eq!(
            vl_search_batch(
                coll,
                queries.as_ptr(),
                2,
                3,
                1,
                ptr::null(),
                0,
                VL_CODEC_JSON,
                &mut batch
            ),
            VL_OK,
            "last error: {}",
            last_error()
        );
        assert_eq!(vl_hits_batch_len(batch), 2);
        assert_eq!(vl_hits_batch_hits_len(batch, 0), 1);
        assert_eq!(vl_hits_batch_hits_len(batch, 1), 1);
        vl_hits_batch_free(batch);

        // A null vector pointer with a non-zero count is rejected rather than
        // read.
        let mut none: *mut vl_hits_batch = ptr::null_mut();
        assert_eq!(
            vl_search_batch(
                coll,
                ptr::null(),
                2,
                3,
                1,
                ptr::null(),
                0,
                VL_CODEC_JSON,
                &mut none
            ),
            VL_ERR_INVALID_ARGUMENT
        );

        vl_collection_free(coll);
        vl_db_close(db);
    }
}
