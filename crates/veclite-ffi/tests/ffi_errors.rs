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
