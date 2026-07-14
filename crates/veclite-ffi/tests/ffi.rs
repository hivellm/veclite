//! FFI smoke + safety tests (SPEC-008): a full round trip through the C ABI,
//! error-code mapping with the thread-local message, and the panic-injection
//! boundary (acceptance 2). These call the `extern "C"` functions exactly as a
//! C caller would.

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

#[test]
fn full_round_trip() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);

        let opts = br#"{"dimension":3,"metric":"euclidean","quantization_bits":0}"#;
        let name = cs("docs");
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

        // Upsert two points, one with a payload.
        let id_a = cs("a");
        let va = [1.0f32, 2.0, 3.0];
        let payload = br#"{"lang":"en"}"#;
        assert_eq!(
            vl_upsert(
                coll,
                id_a.as_ptr(),
                va.as_ptr(),
                3,
                payload.as_ptr(),
                payload.len(),
                VL_CODEC_JSON,
            ),
            VL_OK
        );
        let id_b = cs("b");
        let vb = [9.0f32, 9.0, 9.0];
        assert_eq!(
            vl_upsert(
                coll,
                id_b.as_ptr(),
                vb.as_ptr(),
                3,
                ptr::null(),
                0,
                VL_CODEC_JSON
            ),
            VL_OK
        );

        let mut n: u64 = 0;
        assert_eq!(vl_count(coll, &mut n), VL_OK);
        assert_eq!(n, 2);

        // Search for the neighbour of `a`.
        let q = [1.0f32, 2.0, 3.0];
        let mut hits: *mut vl_hits = ptr::null_mut();
        assert_eq!(
            vl_search(
                coll,
                q.as_ptr(),
                3,
                1,
                ptr::null(),
                0,
                VL_CODEC_JSON,
                &mut hits
            ),
            VL_OK
        );
        assert_eq!(vl_hits_len(hits), 1);
        let mut view = vl_hit_view {
            id: ptr::null(),
            score: 0.0,
            payload: ptr::null(),
            payload_len: 0,
            has_vector: false,
            vector: ptr::null(),
            vector_len: 0,
        };
        assert_eq!(vl_hits_get(hits, 0, &mut view), VL_OK);
        let hit_id = CStr::from_ptr(view.id).to_string_lossy();
        assert_eq!(hit_id, "a");
        assert!(view.payload_len > 0); // payload was requested by default
        vl_hits_free(hits);

        // get → buffer round trips.
        let mut buf = vl_buf {
            data: ptr::null_mut(),
            len: 0,
        };
        assert_eq!(vl_get(coll, id_a.as_ptr(), VL_CODEC_JSON, &mut buf), VL_OK);
        assert!(buf.len > 0);
        vl_buf_free(&mut buf);

        // A missing id → empty buffer, still OK.
        let miss = cs("nope");
        assert_eq!(vl_get(coll, miss.as_ptr(), VL_CODEC_JSON, &mut buf), VL_OK);
        assert_eq!(buf.len, 0);
        vl_buf_free(&mut buf);

        vl_collection_free(coll);
        assert_eq!(vl_db_close(db), VL_OK);
    }
}

#[test]
fn errors_map_to_codes_and_set_the_message() {
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);

        // Missing collection → VL_ERR_COLLECTION_NOT_FOUND, message set.
        let missing = cs("ghost");
        let mut coll: *mut vl_collection = ptr::null_mut();
        let code = vl_collection_get(db, missing.as_ptr(), &mut coll);
        assert_eq!(code, VL_ERR_COLLECTION_NOT_FOUND);
        assert!(last_error().contains("ghost"), "message: {}", last_error());

        // Unknown provider → VL_ERR_UNSUPPORTED_PROVIDER.
        let opts = br#"{"dimension":8,"embedding_provider":"bm52"}"#;
        let name = cs("t");
        let code = vl_collection_create(
            db,
            name.as_ptr(),
            opts.as_ptr(),
            opts.len(),
            VL_CODEC_JSON,
            &mut coll,
        );
        assert_eq!(code, VL_ERR_UNSUPPORTED_PROVIDER);

        // A dimension mismatch → VL_ERR_DIMENSION_MISMATCH.
        let good = br#"{"dimension":3}"#;
        let docs = cs("docs");
        assert_eq!(
            vl_collection_create(
                db,
                docs.as_ptr(),
                good.as_ptr(),
                good.len(),
                VL_CODEC_JSON,
                &mut coll
            ),
            VL_OK
        );
        let id = cs("x");
        let wrong = [1.0f32, 2.0]; // dim 2 into a dim-3 collection
        let code = vl_upsert(
            coll,
            id.as_ptr(),
            wrong.as_ptr(),
            2,
            ptr::null(),
            0,
            VL_CODEC_JSON,
        );
        assert_eq!(code, VL_ERR_DIMENSION_MISMATCH);
        assert!(last_error().contains('3') && last_error().contains('2'));

        vl_collection_free(coll);
        vl_db_close(db);
    }
}

#[test]
fn panic_at_the_boundary_returns_internal_and_stays_healthy() {
    let code = vl__test_force_panic();
    assert_eq!(code, VL_ERR_INTERNAL);
    assert!(last_error().contains("panic"), "message: {}", last_error());
    // The process is still healthy: a normal call works afterward.
    unsafe {
        let mut db: *mut vl_db = ptr::null_mut();
        assert_eq!(vl_open_memory(&mut db), VL_OK);
        assert_eq!(vl_db_close(db), VL_OK);
    }
}

#[test]
fn ffi_consts_mirror_the_core_error_codes() {
    use veclite::VecLiteError as E;
    // The exhaustive source of truth is E::ffi_code (inside the core crate); the
    // public FFI consts must mirror it.
    assert_eq!(
        E::CollectionNotFound(String::new()).ffi_code(),
        VL_ERR_COLLECTION_NOT_FOUND
    );
    assert_eq!(
        E::AlreadyExists(String::new()).ffi_code(),
        VL_ERR_ALREADY_EXISTS
    );
    assert_eq!(E::Locked.ffi_code(), VL_ERR_LOCKED);
    assert_eq!(E::WalPending.ffi_code(), VL_ERR_WAL_PENDING);
    assert_eq!(E::ReadOnly.ffi_code(), VL_ERR_READ_ONLY);
    assert_eq!(E::Closed.ffi_code(), VL_ERR_CLOSED);
    assert_eq!(
        E::InvalidArgument(String::new()).ffi_code(),
        VL_ERR_INVALID_ARGUMENT
    );
    assert_eq!(E::Corrupt(String::new()).ffi_code(), VL_ERR_CORRUPT);
}

#[test]
fn null_handles_are_rejected_not_crashed() {
    unsafe {
        let mut n: u64 = 0;
        assert_eq!(vl_count(ptr::null_mut(), &mut n), VL_ERR_INVALID_ARGUMENT);
        // Freeing null is a no-op.
        assert_eq!(vl_db_close(ptr::null_mut()), VL_OK);
        vl_hits_free(ptr::null_mut());
        vl_buf_free(ptr::null_mut());
    }
}
