//! C ABI for VecLite (SPEC-008). Handle-based, panic-safe (`catch_unwind` at
//! every entry point → `VL_ERR_INTERNAL`), with error codes 1:1 to
//! `VecLiteError` and a thread-local last-error message. Structured data crosses
//! as JSON or MessagePack bytes per a codec flag; vectors as `(*const f32,
//! len)`. Library-allocated objects are freed only by the matching `vl_*_free`.
//!
//! This is the phase4a core surface; the cbindgen golden header, the
//! `cargo public-api` freeze snapshot, and the remaining functions
//! (batch/hybrid/scroll/chunk/reindex/refit/payload-index/snapshot/vacuum) are
//! tracked in the phase4 follow-up.
#![allow(non_camel_case_types)]

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::slice;

use serde::Serialize;
use veclite::{
    Collection, CollectionOptions, Filter, Hit, Metric, Point, Quantization, VecLite, VecLiteError,
};

// ── error codes (SPEC-008 §3, 1:1 with VecLiteError) ─────────────────────────
/// Success.
pub const VL_OK: i32 = 0;
pub const VL_ERR_COLLECTION_NOT_FOUND: i32 = -1;
pub const VL_ERR_VECTOR_NOT_FOUND: i32 = -2;
pub const VL_ERR_ALREADY_EXISTS: i32 = -3;
pub const VL_ERR_DIMENSION_MISMATCH: i32 = -4;
pub const VL_ERR_LOCKED: i32 = -5;
pub const VL_ERR_CORRUPT: i32 = -6;
pub const VL_ERR_UNSUPPORTED_FORMAT: i32 = -7;
pub const VL_ERR_UNSUPPORTED_PROVIDER: i32 = -8;
pub const VL_ERR_READ_ONLY: i32 = -9;
pub const VL_ERR_INVALID_ARGUMENT: i32 = -10;
pub const VL_ERR_IO: i32 = -11;
pub const VL_ERR_WAL_PENDING: i32 = -12;
pub const VL_ERR_CLOSED: i32 = -13;
pub const VL_ERR_INTERNAL: i32 = -99;

/// Codec flags for structured payloads (FFI-005).
pub const VL_CODEC_JSON: u8 = 0;
pub const VL_CODEC_MSGPACK: u8 = 1;

/// Map a `VecLiteError` to its stable code. The exhaustive match (and thus the
/// acceptance-3 build guarantee) lives on `VecLiteError::ffi_code` inside the
/// core crate, since `VecLiteError` is `#[non_exhaustive]`.
fn code_for(e: &VecLiteError) -> i32 {
    e.ffi_code()
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_last_error(msg: &str) {
    // Replace interior NULs so the message is always a valid C string.
    let sanitized = msg.replace('\0', " ");
    let c = CString::new(sanitized).unwrap_or_default();
    LAST_ERROR.with(|slot| *slot.borrow_mut() = c);
}

type Result<T> = std::result::Result<T, VecLiteError>;

/// Run an FFI body: catch panics → `VL_ERR_INTERNAL`, map errors to codes, set
/// the thread-local message on any failure (FFI-003).
fn ffi<F: FnOnce() -> Result<()>>(f: F) -> i32 {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(())) => VL_OK,
        Ok(Err(e)) => {
            set_last_error(&e.to_string());
            code_for(&e)
        }
        Err(_) => {
            set_last_error("internal panic caught at the FFI boundary");
            VL_ERR_INTERNAL
        }
    }
}

fn invalid(msg: &str) -> VecLiteError {
    VecLiteError::InvalidArgument(msg.to_owned())
}

// ── handles & result objects ─────────────────────────────────────────────────
/// Opaque database handle.
pub struct vl_db {
    db: VecLite,
}
/// Opaque collection handle.
pub struct vl_collection {
    c: Collection,
}
/// Opaque search-result set.
pub struct vl_hits {
    hits: Vec<HitOwned>,
}

struct HitOwned {
    id: CString,
    score: f32,
    payload: Vec<u8>,
    vector: Option<Vec<f32>>,
}

/// An owned byte buffer returned to the caller; free with `vl_buf_free`.
#[repr(C)]
pub struct vl_buf {
    /// Buffer bytes.
    pub data: *mut u8,
    /// Buffer length.
    pub len: usize,
}

/// A borrowed view of one hit, valid until `vl_hits_free` (FFI-010).
#[repr(C)]
pub struct vl_hit_view {
    /// NUL-terminated id.
    pub id: *const c_char,
    /// Fused/similarity/distance score.
    pub score: f32,
    /// Encoded payload bytes (null when absent).
    pub payload: *const u8,
    /// Payload length.
    pub payload_len: usize,
    /// Whether `vector` is set.
    pub has_vector: bool,
    /// Stored vector (null unless requested).
    pub vector: *const f32,
    /// Vector length.
    pub vector_len: usize,
}

fn buf_from_vec(v: Vec<u8>) -> vl_buf {
    let boxed = v.into_boxed_slice();
    let len = boxed.len();
    let data = Box::into_raw(boxed).cast::<u8>();
    vl_buf { data, len }
}

// ── codec helpers ────────────────────────────────────────────────────────────
fn encode<T: Serialize>(value: &T, codec: u8) -> Result<Vec<u8>> {
    match codec {
        VL_CODEC_JSON => {
            serde_json::to_vec(value).map_err(|e| invalid(&format!("json encode: {e}")))
        }
        VL_CODEC_MSGPACK => {
            rmp_serde::to_vec_named(value).map_err(|e| invalid(&format!("msgpack encode: {e}")))
        }
        _ => Err(invalid("unknown codec (expected 0=json, 1=msgpack)")),
    }
}

fn decode_value(bytes: &[u8], codec: u8) -> Result<serde_json::Value> {
    match codec {
        VL_CODEC_JSON => {
            serde_json::from_slice(bytes).map_err(|e| invalid(&format!("json decode: {e}")))
        }
        VL_CODEC_MSGPACK => {
            rmp_serde::from_slice(bytes).map_err(|e| invalid(&format!("msgpack decode: {e}")))
        }
        _ => Err(invalid("unknown codec (expected 0=json, 1=msgpack)")),
    }
}

// ── safe accessors for raw inputs ────────────────────────────────────────────
/// # Safety
/// `p` must be a valid NUL-terminated UTF-8 C string or null.
unsafe fn cstr<'a>(p: *const c_char) -> Result<&'a str> {
    if p.is_null() {
        return Err(invalid("null string argument"));
    }
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .map_err(|_| invalid("string argument is not valid UTF-8"))
}

/// # Safety
/// `p`/`len` must describe a valid slice, or `p` may be null with `len == 0`.
unsafe fn opt_slice<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if p.is_null() || len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(p, len) }
    }
}

/// # Safety
/// `p` must be a live `vl_db` handle from `vl_open*` (not closed).
unsafe fn db_ref<'a>(p: *const vl_db) -> Result<&'a VecLite> {
    unsafe { p.as_ref() }
        .map(|h| &h.db)
        .ok_or_else(|| invalid("null database handle"))
}

/// # Safety
/// `p` must be a live `vl_collection` handle.
unsafe fn coll_ref<'a>(p: *const vl_collection) -> Result<&'a Collection> {
    unsafe { p.as_ref() }
        .map(|h| &h.c)
        .ok_or_else(|| invalid("null collection handle"))
}

fn metric_from_str(s: &str) -> Result<Metric> {
    match s {
        "" | "cosine" => Ok(Metric::Cosine),
        "euclidean" | "l2" => Ok(Metric::Euclidean),
        "dot" | "dotproduct" | "dot_product" => Ok(Metric::DotProduct),
        other => Err(invalid(&format!("unknown metric '{other}'"))),
    }
}

// ── meta / errors ────────────────────────────────────────────────────────────
/// Crate semver string (static, never freed).
#[unsafe(no_mangle)]
pub extern "C" fn vl_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// ABI version; bumped only on additive changes (FFI-007).
#[unsafe(no_mangle)]
pub extern "C" fn vl_abi_version() -> u32 {
    1
}

/// On-disk storage format version.
#[unsafe(no_mangle)]
pub extern "C" fn vl_format_version() -> u32 {
    1
}

/// The last error message on the calling thread; valid until the next FFI call
/// on this thread (FFI-020). Never null.
#[unsafe(no_mangle)]
pub extern "C" fn vl_last_error_message() -> *const c_char {
    LAST_ERROR.with(|slot| slot.borrow().as_ptr())
}

// ── lifecycle ────────────────────────────────────────────────────────────────
/// Open (or create) a database at `path`. `opts_*` are currently ignored (open
/// with defaults); tuned opts land with the follow-up.
///
/// # Safety
/// `path` is a valid C string; `out` is a valid pointer to write the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_open(
    path: *const c_char,
    _opts: *const c_char,
    _opts_len: usize,
    out: *mut *mut vl_db,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let path = unsafe { cstr(path) }?;
        let db = VecLite::open(path)?;
        unsafe { *out = Box::into_raw(Box::new(vl_db { db })) };
        Ok(())
    })
}

/// Open an ephemeral in-memory database.
///
/// # Safety
/// `out` is a valid pointer to write the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_open_memory(out: *mut *mut vl_db) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let db = VecLite::memory();
        unsafe { *out = Box::into_raw(Box::new(vl_db { db })) };
        Ok(())
    })
}

/// Close and free a database handle (idempotent for null).
///
/// # Safety
/// `db` is a handle from `vl_open*` that has not already been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_db_close(db: *mut vl_db) -> i32 {
    ffi(|| {
        if !db.is_null() {
            drop(unsafe { Box::from_raw(db) });
        }
        Ok(())
    })
}

/// Force a checkpoint.
///
/// # Safety
/// `db` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_db_checkpoint(db: *mut vl_db) -> i32 {
    ffi(|| unsafe { db_ref(db) }?.checkpoint())
}

// ── collections ──────────────────────────────────────────────────────────────
#[derive(serde::Deserialize)]
struct CreateOpts {
    dimension: usize,
    #[serde(default)]
    metric: String,
    #[serde(default)]
    embedding_provider: Option<String>,
    #[serde(default)]
    quantization_bits: Option<u8>,
}

/// Create a collection. `opts` is a JSON/MessagePack object
/// `{ dimension, metric?, embedding_provider?, quantization_bits? }`.
///
/// # Safety
/// All pointers are valid per their lengths; `out` receives the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_collection_create(
    db: *mut vl_db,
    name: *const c_char,
    opts: *const u8,
    opts_len: usize,
    codec: u8,
    out: *mut *mut vl_collection,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let db = unsafe { db_ref(db) }?;
        let name = unsafe { cstr(name) }?;
        let bytes = unsafe { opt_slice(opts, opts_len) };
        let value = decode_value(bytes, codec)?;
        let co: CreateOpts =
            serde_json::from_value(value).map_err(|e| invalid(&format!("options: {e}")))?;
        let metric = metric_from_str(&co.metric)?;
        let mut options = match &co.embedding_provider {
            Some(p) => CollectionOptions::auto_embed(p, co.dimension),
            None => CollectionOptions::new(co.dimension, metric),
        };
        if let Some(bits) = co.quantization_bits {
            options = options.quantization(if bits == 0 {
                Quantization::None
            } else {
                Quantization::Scalar { bits }
            });
        }
        let c = db.create_collection(name, options)?;
        unsafe { *out = Box::into_raw(Box::new(vl_collection { c })) };
        Ok(())
    })
}

/// Get a handle to an existing collection (or alias).
///
/// # Safety
/// Pointers valid; `out` receives the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_collection_get(
    db: *mut vl_db,
    name: *const c_char,
    out: *mut *mut vl_collection,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let db = unsafe { db_ref(db) }?;
        let name = unsafe { cstr(name) }?;
        let c = db.collection(name)?;
        unsafe { *out = Box::into_raw(Box::new(vl_collection { c })) };
        Ok(())
    })
}

/// Drop a collection.
///
/// # Safety
/// `db` live; `name` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_collection_drop(db: *mut vl_db, name: *const c_char) -> i32 {
    ffi(|| unsafe { db_ref(db) }?.delete_collection(unsafe { cstr(name) }?))
}

/// Rename a collection.
///
/// # Safety
/// `db` live; `from`/`to` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_collection_rename(
    db: *mut vl_db,
    from: *const c_char,
    to: *const c_char,
) -> i32 {
    ffi(|| unsafe { db_ref(db) }?.rename_collection(unsafe { cstr(from) }?, unsafe { cstr(to) }?))
}

/// Free a collection handle (does not drop the collection).
///
/// # Safety
/// `c` is a handle from `vl_collection_create`/`get`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_collection_free(c: *mut vl_collection) -> i32 {
    ffi(|| {
        if !c.is_null() {
            drop(unsafe { Box::from_raw(c) });
        }
        Ok(())
    })
}

/// Encode the sorted collection names into `out` (freed with `vl_buf_free`).
///
/// # Safety
/// `db` live; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_collections_list(db: *mut vl_db, codec: u8, out: *mut vl_buf) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let names = unsafe { db_ref(db) }?.list_collections();
        let bytes = encode(&names, codec)?;
        unsafe { *out = buf_from_vec(bytes) };
        Ok(())
    })
}

/// Create an alias.
///
/// # Safety
/// `db` live; `alias`/`target` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_alias_create(
    db: *mut vl_db,
    alias: *const c_char,
    target: *const c_char,
) -> i32 {
    ffi(|| unsafe { db_ref(db) }?.create_alias(unsafe { cstr(alias) }?, unsafe { cstr(target) }?))
}

/// Delete an alias.
///
/// # Safety
/// `db` live; `alias` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_alias_delete(db: *mut vl_db, alias: *const c_char) -> i32 {
    ffi(|| unsafe { db_ref(db) }?.delete_alias(unsafe { cstr(alias) }?))
}

/// Encode `{ name, dimension, len, tombstones, auto_embed }` into `out`.
///
/// # Safety
/// `c` live; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_collection_stats(
    c: *mut vl_collection,
    codec: u8,
    out: *mut vl_buf,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let s = unsafe { coll_ref(c) }?.stats();
        let value = serde_json::json!({
            "name": s.name,
            "dimension": s.dimension,
            "len": s.len,
            "tombstones": s.tombstones,
            "auto_embed": s.auto_embed,
        });
        unsafe { *out = buf_from_vec(encode(&value, codec)?) };
        Ok(())
    })
}

// ── writes ───────────────────────────────────────────────────────────────────
/// Upsert one vector with an optional payload.
///
/// # Safety
/// `vec`/`dim` describe a valid slice; `payload`/`payload_len` a valid slice or
/// null/0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_upsert(
    c: *mut vl_collection,
    id: *const c_char,
    vec: *const f32,
    dim: usize,
    payload: *const u8,
    payload_len: usize,
    codec: u8,
) -> i32 {
    ffi(|| {
        let coll = unsafe { coll_ref(c) }?;
        let id = unsafe { cstr(id) }?;
        if vec.is_null() {
            return Err(invalid("null vector"));
        }
        let vector = unsafe { slice::from_raw_parts(vec, dim) }.to_vec();
        let mut point = Point::new(id, vector);
        let pbytes = unsafe { opt_slice(payload, payload_len) };
        if !pbytes.is_empty() {
            point = point.payload(decode_value(pbytes, codec)?);
        }
        coll.upsert(point)
    })
}

/// Upsert one text document (auto-embed collections).
///
/// # Safety
/// `id`/`text` valid; `payload` valid or null/0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_upsert_text(
    c: *mut vl_collection,
    id: *const c_char,
    text: *const c_char,
    payload: *const u8,
    payload_len: usize,
    codec: u8,
) -> i32 {
    ffi(|| {
        let coll = unsafe { coll_ref(c) }?;
        let id = unsafe { cstr(id) }?;
        let text = unsafe { cstr(text) }?;
        let pbytes = unsafe { opt_slice(payload, payload_len) };
        if pbytes.is_empty() {
            coll.upsert_text(id, text)
        } else {
            coll.upsert_text_with(id, text, decode_value(pbytes, codec)?)
        }
    })
}

/// Delete one id; `existed` (if non-null) receives whether it was present.
///
/// # Safety
/// `id` valid; `existed` valid or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_delete(
    c: *mut vl_collection,
    id: *const c_char,
    existed: *mut bool,
) -> i32 {
    ffi(|| {
        let coll = unsafe { coll_ref(c) }?;
        let was = coll.delete(unsafe { cstr(id) }?)?;
        if !existed.is_null() {
            unsafe { *existed = was };
        }
        Ok(())
    })
}

/// Number of live vectors.
///
/// # Safety
/// `c` live; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_count(c: *mut vl_collection, out: *mut u64) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let n = unsafe { coll_ref(c) }?.len() as u64;
        unsafe { *out = n };
        Ok(())
    })
}

// ── reads & search ───────────────────────────────────────────────────────────
/// Fetch one point encoded into `out`; an empty buffer means absent.
///
/// # Safety
/// `id` valid; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_get(
    c: *mut vl_collection,
    id: *const c_char,
    codec: u8,
    out: *mut vl_buf,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let coll = unsafe { coll_ref(c) }?;
        let bytes = match coll.get(unsafe { cstr(id) }?)? {
            Some(p) => {
                let value = serde_json::json!({
                    "id": p.id,
                    "vector": p.vector,
                    "payload": p.payload,
                });
                encode(&value, codec)?
            }
            None => Vec::new(),
        };
        unsafe { *out = buf_from_vec(bytes) };
        Ok(())
    })
}

#[derive(serde::Deserialize, Default)]
struct QueryOpts {
    ef_search: Option<usize>,
    with_payload: Option<bool>,
    with_vector: Option<bool>,
    filter: Option<serde_json::Value>,
}

fn run_query(coll: &Collection, vector: &[f32], limit: u32, opts: QueryOpts) -> Result<Vec<Hit>> {
    let mut qb = coll
        .query(vector)
        .limit(limit as usize)
        .with_payload(opts.with_payload.unwrap_or(true))
        .with_vector(opts.with_vector.unwrap_or(false));
    if let Some(ef) = opts.ef_search {
        qb = qb.ef_search(ef);
    }
    if let Some(fdoc) = opts.filter {
        qb = qb.filter(Filter::from_json(&fdoc)?);
    }
    qb.run()
}

fn hits_into_handle(hits: Vec<Hit>, codec: u8) -> Result<*mut vl_hits> {
    let owned: Result<Vec<HitOwned>> = hits
        .into_iter()
        .map(|h| {
            let payload = match &h.payload {
                Some(v) => encode(v, codec)?,
                None => Vec::new(),
            };
            Ok(HitOwned {
                id: CString::new(h.id).map_err(|_| invalid("hit id has an interior NUL"))?,
                score: h.score,
                payload,
                vector: h.vector,
            })
        })
        .collect();
    Ok(Box::into_raw(Box::new(vl_hits { hits: owned? })))
}

/// k-NN search. `query_opts` (JSON/MessagePack) is an optional
/// `{ ef_search?, with_payload?, with_vector?, filter? }`.
///
/// # Safety
/// `vec`/`dim` valid; `query_opts` valid or null/0; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_search(
    c: *mut vl_collection,
    vec: *const f32,
    dim: usize,
    limit: u32,
    query_opts: *const u8,
    opts_len: usize,
    codec: u8,
    out: *mut *mut vl_hits,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let coll = unsafe { coll_ref(c) }?;
        if vec.is_null() {
            return Err(invalid("null query vector"));
        }
        let vector = unsafe { slice::from_raw_parts(vec, dim) };
        let opts = parse_query_opts(unsafe { opt_slice(query_opts, opts_len) }, codec)?;
        let hits = run_query(coll, vector, limit, opts)?;
        unsafe { *out = hits_into_handle(hits, codec)? };
        Ok(())
    })
}

/// Text search (auto-embed collections).
///
/// # Safety
/// `query` valid; `query_opts` valid or null/0; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_search_text(
    c: *mut vl_collection,
    query: *const c_char,
    limit: u32,
    _query_opts: *const u8,
    _opts_len: usize,
    codec: u8,
    out: *mut *mut vl_hits,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let coll = unsafe { coll_ref(c) }?;
        let q = unsafe { cstr(query) }?;
        let hits = coll.search_text(q, limit as usize)?;
        unsafe { *out = hits_into_handle(hits, codec)? };
        Ok(())
    })
}

fn parse_query_opts(bytes: &[u8], codec: u8) -> Result<QueryOpts> {
    if bytes.is_empty() {
        return Ok(QueryOpts::default());
    }
    let value = decode_value(bytes, codec)?;
    serde_json::from_value(value).map_err(|e| invalid(&format!("query_opts: {e}")))
}

// ── results ──────────────────────────────────────────────────────────────────
/// Number of hits.
///
/// # Safety
/// `hits` is a live `vl_hits` (or null → 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_hits_len(hits: *const vl_hits) -> u32 {
    match unsafe { hits.as_ref() } {
        Some(h) => h.hits.len() as u32,
        None => 0,
    }
}

/// Fill `out` with a borrowed view of hit `i`; returns `VL_ERR_INVALID_ARGUMENT`
/// if `i` is out of range. Views are valid until `vl_hits_free`.
///
/// # Safety
/// `hits` live; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_hits_get(hits: *const vl_hits, i: u32, out: *mut vl_hit_view) -> i32 {
    ffi(|| {
        let hits = unsafe { hits.as_ref() }.ok_or_else(|| invalid("null hits handle"))?;
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let h = hits
            .hits
            .get(i as usize)
            .ok_or_else(|| invalid("hit index out of range"))?;
        let view = vl_hit_view {
            id: h.id.as_ptr(),
            score: h.score,
            payload: if h.payload.is_empty() {
                std::ptr::null()
            } else {
                h.payload.as_ptr()
            },
            payload_len: h.payload.len(),
            has_vector: h.vector.is_some(),
            vector: h.vector.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            vector_len: h.vector.as_ref().map_or(0, Vec::len),
        };
        unsafe { *out = view };
        Ok(())
    })
}

/// Free a search-result set.
///
/// # Safety
/// `hits` is a handle from a search function (or null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_hits_free(hits: *mut vl_hits) {
    if !hits.is_null() {
        drop(unsafe { Box::from_raw(hits) });
    }
}

/// Free a byte buffer returned by the library.
///
/// # Safety
/// `buf` points at a `vl_buf` filled by the library (or null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_buf_free(buf: *mut vl_buf) {
    if buf.is_null() {
        return;
    }
    let b = unsafe { &mut *buf };
    if !b.data.is_null() && b.len > 0 {
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(b.data, b.len)) });
    }
    b.data = std::ptr::null_mut();
    b.len = 0;
}

/// Test-only hook: forces a panic to exercise the `catch_unwind` boundary
/// (SPEC-008 acceptance 2). Not part of the stable surface.
#[doc(hidden)]
#[unsafe(no_mangle)]
pub extern "C" fn vl__test_force_panic() -> i32 {
    ffi(|| {
        panic!("forced panic for the FFI boundary test");
    })
}
