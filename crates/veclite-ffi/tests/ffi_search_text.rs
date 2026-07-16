//! FFI coverage for the text/search/query-opts surface (SPEC-008) not exercised
//! by `ffi.rs`/`ffi_errors.rs`: `vl_upsert_text` + `vl_search_text` on an
//! auto-embed collection, `vl_search` with a full query-opts document (ef_search,
//! with_vector, with_payload, filter), the MessagePack codec round-trip, and the
//! unknown-codec error branch. Drives the `extern "C"` surface as a C caller.

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

#[test]
fn upsert_text_and_search_text_on_an_auto_embed_collection() {
    unsafe {
        let db = memory_db();
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

        let q = cs("quick fox");
        let mut hits: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_search_text(
                coll,
                q.as_ptr(),
                2,
                ptr::null(),
                0,
                VL_CODEC_JSON,
                &mut hits
            ),
            VL_OK,
        );
        assert!(vl_hits_len(hits) >= 1);
        let mut view = std::mem::zeroed::<vl_hit_view>();
        assert_eq!(vl_hits_get(hits, 0, &mut view), VL_OK);
        let id = CStr::from_ptr(view.id).to_string_lossy();
        assert!(id == "d1" || id == "d2");
        vl_hits_free(hits);

        // vl_upsert_text with a payload takes the `upsert_text_with` branch.
        let cid = cs("d3");
        let ctext = cs("quick foxes run");
        let payload = br#"{"tag":"animal"}"#;
        assert_eq!(
            vl_upsert_text(
                coll,
                cid.as_ptr(),
                ctext.as_ptr(),
                payload.as_ptr(),
                payload.len(),
                VL_CODEC_JSON,
            ),
            VL_OK,
        );

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn search_with_full_query_opts_projects_vector_and_applies_filter() {
    unsafe {
        let db = memory_db();
        let coll = create(
            db,
            "v",
            r#"{"dimension":2,"metric":"euclidean","quantization_bits":0}"#,
        );

        for (i, lang) in ["en", "pt", "en"].iter().enumerate() {
            let id = cs(&format!("k{i}"));
            let vec = [i as f32, 0.0f32];
            let payload = format!(r#"{{"lang":"{lang}"}}"#);
            assert_eq!(
                vl_upsert(
                    coll,
                    id.as_ptr(),
                    vec.as_ptr(),
                    2,
                    payload.as_bytes().as_ptr(),
                    payload.len(),
                    VL_CODEC_JSON,
                ),
                VL_OK,
            );
        }

        // Query opts: ef_search + with_vector + with_payload + a keyword filter.
        let opts = br#"{"ef_search":32,"with_vector":true,"with_payload":true,"filter":{"must":[{"key":"lang","match":{"value":"en"}}]}}"#;
        let query = [0.0f32, 0.0];
        let mut hits: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_search(
                coll,
                query.as_ptr(),
                2,
                10,
                opts.as_ptr(),
                opts.len(),
                VL_CODEC_JSON,
                &mut hits,
            ),
            VL_OK,
        );
        // Only the two "en" docs match; nearest is k0.
        assert_eq!(vl_hits_len(hits), 2);
        let mut view = std::mem::zeroed::<vl_hit_view>();
        assert_eq!(vl_hits_get(hits, 0, &mut view), VL_OK);
        assert_eq!(CStr::from_ptr(view.id).to_string_lossy(), "k0");
        assert!(view.has_vector, "with_vector projected the stored vector");
        assert_eq!(view.vector_len, 2);
        assert!(
            !view.payload.is_null(),
            "with_payload projected the payload"
        );
        vl_hits_free(hits);

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn msgpack_codec_round_trips_and_unknown_codec_is_rejected() {
    unsafe {
        let db = memory_db();
        // Create the collection with a MessagePack-encoded options document.
        let opts_val =
            serde_json::json!({ "dimension": 2, "metric": "euclidean", "quantization_bits": 0 });
        let opts_mp = rmp_serde::to_vec_named(&opts_val).unwrap_or_else(|e| panic!("{e}"));
        let cname = cs("mp");
        let mut coll: *mut vl_collection = ptr::null_mut();
        assert_eq!(
            vl_collection_create(
                db,
                cname.as_ptr(),
                opts_mp.as_ptr(),
                opts_mp.len(),
                VL_CODEC_MSGPACK,
                &mut coll,
            ),
            VL_OK,
        );

        // Upsert with a MessagePack payload, then read it back MessagePack-encoded.
        let payload_val = serde_json::json!({ "n": 7 });
        let payload_mp = rmp_serde::to_vec_named(&payload_val).unwrap_or_else(|e| panic!("{e}"));
        let id = cs("a");
        let vec = [1.0f32, 0.0];
        assert_eq!(
            vl_upsert(
                coll,
                id.as_ptr(),
                vec.as_ptr(),
                2,
                payload_mp.as_ptr(),
                payload_mp.len(),
                VL_CODEC_MSGPACK
            ),
            VL_OK,
        );

        let query = [1.0f32, 0.0];
        let mut hits: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_search(
                coll,
                query.as_ptr(),
                2,
                1,
                ptr::null(),
                0,
                VL_CODEC_MSGPACK,
                &mut hits
            ),
            VL_OK,
        );
        let mut view = std::mem::zeroed::<vl_hit_view>();
        assert_eq!(vl_hits_get(hits, 0, &mut view), VL_OK);
        let bytes = std::slice::from_raw_parts(view.payload, view.payload_len);
        let back: serde_json::Value =
            rmp_serde::from_slice(bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.get("n").and_then(|v| v.as_i64()), Some(7));
        vl_hits_free(hits);

        // An unknown codec byte is rejected, not UB.
        let opts = br#"{"dimension":2}"#;
        let cname2 = cs("bad");
        let mut coll2: *mut vl_collection = ptr::null_mut();
        assert_eq!(
            vl_collection_create(
                db,
                cname2.as_ptr(),
                opts.as_ptr(),
                opts.len(),
                99,
                &mut coll2
            ),
            VL_ERR_INVALID_ARGUMENT,
        );

        vl_collection_free(coll);
        vl_db_close(db);
    }
}
