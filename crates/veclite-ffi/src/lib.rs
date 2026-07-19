//! C ABI for VecLite (SPEC-008). Handle-based, panic-safe (`catch_unwind` at
//! every entry point → `VL_ERR_INTERNAL`), with error codes 1:1 to
//! `VecLiteError` and a thread-local last-error message. Structured data crosses
//! as JSON or MessagePack bytes per a codec flag; vectors as `(*const f32,
//! len)`. Library-allocated objects are freed only by the matching `vl_*_free`.
//!
//! The full surface (phase4a core + phase4g): lifecycle, collections, aliases,
//! single and batch writes, get/search/search_text/hybrid/batch-search, scroll,
//! chunk, reindex/refit, payload indexes, and database snapshot/vacuum/info. The
//! header is frozen by the cbindgen golden-file drift test and the ABI is
//! additive-only within a major version (gate loaders on `vl_abi_version()`).
#![allow(non_camel_case_types)]

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char};
use std::slice;

use serde::Serialize;
use veclite::chunk::{ChunkOptions, Chunker};
use veclite::{
    Collection, CollectionOptions, Filter, Hit, Metric, PayloadIndexKind, Point, Quantization,
    SparseVector, VecLite, VecLiteError,
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

/// Payload-index kinds for `vl_payload_index_create` (SPEC-006 §index kinds).
pub const VL_PIDX_KEYWORD: u8 = 0;
pub const VL_PIDX_INTEGER: u8 = 1;
pub const VL_PIDX_FLOAT: u8 = 2;

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
/// `p`/`len` must describe a valid `f32` slice, or `p` may be null with
/// `len == 0`.
unsafe fn opt_f32_slice<'a>(p: *const f32, len: usize) -> &'a [f32] {
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

/// Inverse of [`metric_from_str`], so the stats payloads report the metric with
/// the same spelling the create options accept.
fn metric_to_str(m: Metric) -> &'static str {
    match m {
        Metric::Cosine => "cosine",
        Metric::Euclidean => "euclidean",
        Metric::DotProduct => "dot",
    }
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
        // `auto_embed` takes no metric and falls back to `Metric::default()`,
        // so it used to swallow an explicit `"metric"` in the create options
        // whenever a provider was also given. `.metric()` puts it back.
        let mut options = match &co.embedding_provider {
            Some(p) => CollectionOptions::auto_embed(p, co.dimension).metric(metric),
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
            "metric": metric_to_str(s.metric),
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

#[derive(serde::Deserialize, Default, Clone)]
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

/// Convert core `Hit`s into the owned form the ABI hands out (id as `CString`,
/// payload pre-encoded per `codec`), so views can borrow stable pointers.
fn hits_to_owned(hits: Vec<Hit>, codec: u8) -> Result<Vec<HitOwned>> {
    hits.into_iter()
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
        .collect()
}

fn hits_into_handle(hits: Vec<Hit>, codec: u8) -> Result<*mut vl_hits> {
    Ok(Box::into_raw(Box::new(vl_hits {
        hits: hits_to_owned(hits, codec)?,
    })))
}

/// Borrowed view of one owned hit; valid until its owner (`vl_hits` /
/// `vl_hits_batch`) is freed.
fn hit_view_of(h: &HitOwned) -> vl_hit_view {
    vl_hit_view {
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
    }
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

/// Text search (auto-embed collections). `query_opts` (JSON/MessagePack) is an
/// optional `{ with_payload?, with_vector? }`. The core text path always ranks
/// with the collection's default `ef_search` and does not accept a payload
/// filter, so `ef_search`/`filter` here are rejected with a clear error pointing
/// at `vl_hybrid_search` (which fuses text with a filter). `with_payload=false`
/// strips payloads; `with_vector=true` attaches each hit's stored vector.
///
/// # Safety
/// `query` valid; `query_opts` valid or null/0; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_search_text(
    c: *mut vl_collection,
    query: *const c_char,
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
        let q = unsafe { cstr(query) }?;
        let opts = parse_query_opts(unsafe { opt_slice(query_opts, opts_len) }, codec)?;
        if opts.filter.is_some() || opts.ef_search.is_some() {
            return Err(invalid(
                "text search does not accept filter/ef_search; use vl_hybrid_search \
                 with a text query and a filter",
            ));
        }
        let mut hits = coll.search_text(q, limit as usize)?;
        apply_hit_projection(coll, &mut hits, &opts)?;
        unsafe { *out = hits_into_handle(hits, codec)? };
        Ok(())
    })
}

/// Apply `with_payload`/`with_vector` to hits the core returned with its own
/// defaults (payload on, vector off): drop payloads when not wanted, and fetch
/// each stored vector when the caller asked for it.
fn apply_hit_projection(coll: &Collection, hits: &mut [Hit], opts: &QueryOpts) -> Result<()> {
    if opts.with_payload == Some(false) {
        for h in hits.iter_mut() {
            h.payload = None;
        }
    }
    if opts.with_vector == Some(true) {
        for h in hits.iter_mut() {
            if h.vector.is_none()
                && let Some(p) = coll.get(&h.id)?
            {
                h.vector = Some(p.vector);
            }
        }
    }
    Ok(())
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
        unsafe { *out = hit_view_of(h) };
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

// ── batch writes ─────────────────────────────────────────────────────────────
/// Upsert many points at once. `points` is a JSON/MessagePack array of point
/// objects `{ id, vector, payload?, sparse? }` (the SDK wire shape); the whole
/// batch is validated and applied under one write path (FR-06).
///
/// # Safety
/// `points`/`points_len` describe a valid slice; `c` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_upsert_batch(
    c: *mut vl_collection,
    points: *const u8,
    points_len: usize,
    codec: u8,
) -> i32 {
    ffi(|| {
        let coll = unsafe { coll_ref(c) }?;
        let value = decode_value(unsafe { opt_slice(points, points_len) }, codec)?;
        let batch: Vec<Point> =
            serde_json::from_value(value).map_err(|e| invalid(&format!("points: {e}")))?;
        coll.upsert_batch(batch)
    })
}

/// Delete many ids. `ids` is a JSON/MessagePack array of strings; `out_deleted`
/// (if non-null) receives how many were present.
///
/// # Safety
/// `ids`/`ids_len` describe a valid slice; `out_deleted` is valid or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_delete_batch(
    c: *mut vl_collection,
    ids: *const u8,
    ids_len: usize,
    codec: u8,
    out_deleted: *mut u64,
) -> i32 {
    ffi(|| {
        let coll = unsafe { coll_ref(c) }?;
        let value = decode_value(unsafe { opt_slice(ids, ids_len) }, codec)?;
        let owned: Vec<String> =
            serde_json::from_value(value).map_err(|e| invalid(&format!("ids: {e}")))?;
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        let n = coll.delete_batch(&refs)?;
        if !out_deleted.is_null() {
            unsafe { *out_deleted = n as u64 };
        }
        Ok(())
    })
}

// ── batch search ─────────────────────────────────────────────────────────────
/// One query's outcome inside a `vl_hits_batch`: a status code plus its hits
/// (empty when the code is not `VL_OK`).
struct BatchItem {
    code: i32,
    hits: Vec<HitOwned>,
}

/// Opaque result of `vl_search_batch`: one `BatchItem` per query, in order.
pub struct vl_hits_batch {
    items: Vec<BatchItem>,
}

/// Batch k-NN search. `vecs` is `n` contiguous query vectors of `dim` floats
/// each; every query uses the same `limit` and the shared `query_opts`
/// (`{ ef_search?, filter?, with_payload?, with_vector? }`). Per-query failures
/// (e.g. a dimension mismatch) are reported per item, not as a whole-call error.
///
/// # Safety
/// `vecs` points at `n * dim` valid floats; `query_opts` valid or null/0; `out`
/// is valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_search_batch(
    c: *mut vl_collection,
    vecs: *const f32,
    n: usize,
    dim: usize,
    limit: u32,
    query_opts: *const u8,
    opts_len: usize,
    codec: u8,
    out: *mut *mut vl_hits_batch,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let coll = unsafe { coll_ref(c) }?;
        if vecs.is_null() && n != 0 {
            return Err(invalid("null query vectors"));
        }
        let flat = unsafe { opt_f32_slice(vecs, n.saturating_mul(dim)) };
        let queries: Vec<&[f32]> = (0..n).map(|i| &flat[i * dim..(i + 1) * dim]).collect();
        let opts = parse_query_opts(unsafe { opt_slice(query_opts, opts_len) }, codec)?;

        // When the opts affect *which* results come back (a filter or a custom
        // ef_search), each query must run through the full builder. Otherwise the
        // core's parallel `search_batch` is used and payload/vector projection is
        // applied after.
        let needs_builder = opts.filter.is_some() || opts.ef_search.is_some();
        let items = if needs_builder {
            let mut items = Vec::with_capacity(n);
            for q in &queries {
                items.push(match run_query(coll, q, limit, opts.clone()) {
                    Ok(hits) => BatchItem {
                        code: VL_OK,
                        hits: hits_to_owned(hits, codec)?,
                    },
                    Err(e) => batch_error_item(&e),
                });
            }
            items
        } else {
            let owned: Vec<Vec<f32>> = queries.iter().map(|q| q.to_vec()).collect();
            let mut items = Vec::with_capacity(n);
            for r in coll.search_batch(&owned, limit as usize) {
                items.push(match r {
                    Ok(mut hits) => {
                        apply_hit_projection(coll, &mut hits, &opts)?;
                        BatchItem {
                            code: VL_OK,
                            hits: hits_to_owned(hits, codec)?,
                        }
                    }
                    Err(e) => batch_error_item(&e),
                });
            }
            items
        };
        unsafe { *out = Box::into_raw(Box::new(vl_hits_batch { items })) };
        Ok(())
    })
}

/// A failed-query batch item: record the code (authoritative) and stash the
/// message in the thread-local (best-effort — last write wins).
fn batch_error_item(e: &VecLiteError) -> BatchItem {
    set_last_error(&e.to_string());
    BatchItem {
        code: code_for(e),
        hits: Vec::new(),
    }
}

/// Number of per-query results.
///
/// # Safety
/// `batch` is a live `vl_hits_batch` (or null → 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_hits_batch_len(batch: *const vl_hits_batch) -> u32 {
    match unsafe { batch.as_ref() } {
        Some(b) => b.items.len() as u32,
        None => 0,
    }
}

/// Status code for query `i` (`VL_OK` or a `VL_ERR_*`); `VL_ERR_INVALID_ARGUMENT`
/// if `i` is out of range.
///
/// # Safety
/// `batch` is a live handle (or null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_hits_batch_code(batch: *const vl_hits_batch, i: u32) -> i32 {
    match unsafe { batch.as_ref() }.and_then(|b| b.items.get(i as usize)) {
        Some(item) => item.code,
        None => VL_ERR_INVALID_ARGUMENT,
    }
}

/// Number of hits for query `i` (0 if out of range or the query failed).
///
/// # Safety
/// `batch` is a live handle (or null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_hits_batch_hits_len(batch: *const vl_hits_batch, i: u32) -> u32 {
    match unsafe { batch.as_ref() }.and_then(|b| b.items.get(i as usize)) {
        Some(item) => item.hits.len() as u32,
        None => 0,
    }
}

/// Fill `out` with a borrowed view of hit `hit` in query `query`; returns
/// `VL_ERR_INVALID_ARGUMENT` if either index is out of range. Views are valid
/// until `vl_hits_batch_free`.
///
/// # Safety
/// `batch` live; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_hits_batch_hit(
    batch: *const vl_hits_batch,
    query: u32,
    hit: u32,
    out: *mut vl_hit_view,
) -> i32 {
    ffi(|| {
        let batch = unsafe { batch.as_ref() }.ok_or_else(|| invalid("null hits-batch handle"))?;
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let item = batch
            .items
            .get(query as usize)
            .ok_or_else(|| invalid("query index out of range"))?;
        let h = item
            .hits
            .get(hit as usize)
            .ok_or_else(|| invalid("hit index out of range"))?;
        unsafe { *out = hit_view_of(h) };
        Ok(())
    })
}

/// Free a batch-search result set.
///
/// # Safety
/// `batch` is a handle from `vl_search_batch` (or null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_hits_batch_free(batch: *mut vl_hits_batch) {
    if !batch.is_null() {
        drop(unsafe { Box::from_raw(batch) });
    }
}

// ── hybrid search ────────────────────────────────────────────────────────────
#[derive(serde::Deserialize, Default)]
struct HybridOpts {
    #[serde(default)]
    dense: Option<Vec<f32>>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    sparse: Option<SparseVector>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    alpha: Option<f32>,
    #[serde(default)]
    rrf_k: Option<f32>,
    #[serde(default)]
    with_payload: Option<bool>,
    #[serde(default)]
    with_vector: Option<bool>,
    #[serde(default)]
    filter: Option<serde_json::Value>,
}

/// Hybrid (dense + sparse + text, RRF-fused) search. `opts` (JSON/MessagePack)
/// is `{ dense?, text?, sparse?{indices,values}, limit?, alpha?, rrf_k?,
/// with_payload?, with_vector?, filter? }` — at least one of dense/text/sparse
/// must be present.
///
/// # Safety
/// `opts`/`opts_len` describe a valid slice; `out` is valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_hybrid_search(
    c: *mut vl_collection,
    opts: *const u8,
    opts_len: usize,
    codec: u8,
    out: *mut *mut vl_hits,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let coll = unsafe { coll_ref(c) }?;
        let value = decode_value(unsafe { opt_slice(opts, opts_len) }, codec)?;
        let o: HybridOpts =
            serde_json::from_value(value).map_err(|e| invalid(&format!("hybrid opts: {e}")))?;
        if o.dense.is_none() && o.text.is_none() && o.sparse.is_none() {
            return Err(invalid(
                "hybrid search needs at least one of dense/text/sparse",
            ));
        }
        // Owned locals outlive the borrowing builder within this scope.
        let dense = o.dense;
        let text = o.text;
        let sparse = o.sparse;

        let mut qb = coll.hybrid_query();
        if let Some(d) = dense.as_deref() {
            qb = qb.dense(d);
        }
        if let Some(t) = text.as_deref() {
            qb = qb.text(t);
        }
        if let Some(s) = sparse.as_ref() {
            qb = qb.sparse(s);
        }
        if let Some(l) = o.limit {
            qb = qb.limit(l);
        }
        if let Some(a) = o.alpha {
            qb = qb.alpha(a);
        }
        if let Some(k) = o.rrf_k {
            qb = qb.rrf_k(k);
        }
        if let Some(p) = o.with_payload {
            qb = qb.with_payload(p);
        }
        if let Some(v) = o.with_vector {
            qb = qb.with_vector(v);
        }
        if let Some(f) = o.filter {
            qb = qb.filter(Filter::from_json(&f)?);
        }
        let hits = qb.run()?;
        unsafe { *out = hits_into_handle(hits, codec)? };
        Ok(())
    })
}

// ── scroll (paginated full scan) ─────────────────────────────────────────────
/// Opaque scroll page: pre-encoded points plus the cursor for the next call.
pub struct vl_page {
    points: Vec<Vec<u8>>,
    next_cursor: Option<CString>,
}

#[derive(serde::Deserialize, Default)]
struct ScrollOpts {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    filter: Option<serde_json::Value>,
}

/// Default scroll page size when `scroll_opts.limit` is absent.
const DEFAULT_SCROLL_LIMIT: usize = 100;

/// Scroll the collection in id order. `scroll_opts` (optional JSON/MessagePack)
/// is `{ cursor?, limit?, filter? }`: `cursor` is absent on the first call, then
/// the value from `vl_page_cursor` for each subsequent page. Points in the page
/// are encoded with `codec`, fetched one at a time via `vl_page_point`.
///
/// # Safety
/// `scroll_opts` valid or null/0; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_scroll(
    c: *mut vl_collection,
    scroll_opts: *const u8,
    len: usize,
    codec: u8,
    out: *mut *mut vl_page,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let coll = unsafe { coll_ref(c) }?;
        let obytes = unsafe { opt_slice(scroll_opts, len) };
        let opts: ScrollOpts = if obytes.is_empty() {
            ScrollOpts::default()
        } else {
            serde_json::from_value(decode_value(obytes, codec)?)
                .map_err(|e| invalid(&format!("scroll opts: {e}")))?
        };
        let filter = match opts.filter {
            Some(f) => Some(Filter::from_json(&f)?),
            None => None,
        };
        let limit = opts.limit.unwrap_or(DEFAULT_SCROLL_LIMIT);
        let page = coll.scroll(opts.cursor.as_deref(), limit, filter.as_ref())?;
        let points: Result<Vec<Vec<u8>>> = page
            .points
            .iter()
            .map(|p| {
                let value = serde_json::json!({
                    "id": p.id,
                    "vector": p.vector,
                    "payload": p.payload,
                    "sparse": p.sparse,
                });
                encode(&value, codec)
            })
            .collect();
        let next_cursor = match page.next_cursor {
            Some(s) => Some(CString::new(s).map_err(|_| invalid("cursor has an interior NUL"))?),
            None => None,
        };
        unsafe {
            *out = Box::into_raw(Box::new(vl_page {
                points: points?,
                next_cursor,
            }));
        }
        Ok(())
    })
}

/// Number of points in the page.
///
/// # Safety
/// `page` is a live handle (or null → 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_page_len(page: *const vl_page) -> u32 {
    match unsafe { page.as_ref() } {
        Some(p) => p.points.len() as u32,
        None => 0,
    }
}

/// Copy point `i`'s encoded bytes into `out` (owned; free with `vl_buf_free`).
/// `VL_ERR_INVALID_ARGUMENT` if `i` is out of range.
///
/// # Safety
/// `page` live; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_page_point(page: *const vl_page, i: u32, out: *mut vl_buf) -> i32 {
    ffi(|| {
        let page = unsafe { page.as_ref() }.ok_or_else(|| invalid("null page handle"))?;
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let bytes = page
            .points
            .get(i as usize)
            .ok_or_else(|| invalid("point index out of range"))?;
        unsafe { *out = buf_from_vec(bytes.clone()) };
        Ok(())
    })
}

/// The cursor for the next page, or null when the scan is exhausted. Borrowed;
/// valid until `vl_page_free`.
///
/// # Safety
/// `page` is a live handle (or null → null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_page_cursor(page: *const vl_page) -> *const c_char {
    match unsafe { page.as_ref() } {
        Some(p) => p
            .next_cursor
            .as_ref()
            .map_or(std::ptr::null(), |c| c.as_ptr()),
        None => std::ptr::null(),
    }
}

/// Free a scroll page.
///
/// # Safety
/// `page` is a handle from `vl_scroll` (or null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_page_free(page: *mut vl_page) {
    if !page.is_null() {
        drop(unsafe { Box::from_raw(page) });
    }
}

// ── maintenance ──────────────────────────────────────────────────────────────
/// Rebuild the ANN index from the live vectors (exact, potentially slow).
///
/// # Safety
/// `c` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_collection_reindex(c: *mut vl_collection) -> i32 {
    ffi(|| unsafe { coll_ref(c) }?.reindex())
}

/// Refit the text embedder's vocabulary and re-embed every text document.
///
/// # Safety
/// `c` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_collection_refit(c: *mut vl_collection) -> i32 {
    ffi(|| unsafe { coll_ref(c) }?.refit())
}

/// Declare a payload index on `key`. `kind` is one of `VL_PIDX_KEYWORD` (0),
/// `VL_PIDX_INTEGER` (1), or `VL_PIDX_FLOAT` (2).
///
/// # Safety
/// `c` live; `key` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_payload_index_create(
    c: *mut vl_collection,
    key: *const c_char,
    kind: u8,
) -> i32 {
    ffi(|| {
        let coll = unsafe { coll_ref(c) }?;
        let key = unsafe { cstr(key) }?;
        coll.create_payload_index(key, payload_index_kind(kind)?)
    })
}

fn payload_index_kind(kind: u8) -> Result<PayloadIndexKind> {
    match kind {
        VL_PIDX_KEYWORD => Ok(PayloadIndexKind::Keyword),
        VL_PIDX_INTEGER => Ok(PayloadIndexKind::Integer),
        VL_PIDX_FLOAT => Ok(PayloadIndexKind::Float),
        other => Err(invalid(&format!(
            "unknown payload index kind {other} (0=keyword, 1=integer, 2=float)"
        ))),
    }
}

// ── database maintenance ─────────────────────────────────────────────────────
/// Write a consistent snapshot of the whole database to `path`.
///
/// # Safety
/// `db` live; `path` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_db_snapshot(db: *mut vl_db, path: *const c_char) -> i32 {
    ffi(|| {
        let db = unsafe { db_ref(db) }?;
        db.snapshot(unsafe { cstr(path) }?)
    })
}

/// Reclaim space from tombstoned/rewritten data.
///
/// # Safety
/// `db` is a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_db_vacuum(db: *mut vl_db) -> i32 {
    ffi(|| unsafe { db_ref(db) }?.vacuum())
}

/// Encode a database overview into `out` (freed with `vl_buf_free`):
/// `{ format_version, collections: [{ name, dimension, len, tombstones,
/// auto_embed, payload_indexes: [[field, kind]] }], aliases: [[alias, target]] }`.
///
/// # Safety
/// `db` live; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_db_info(db: *mut vl_db, codec: u8, out: *mut vl_buf) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let db = unsafe { db_ref(db) }?;
        let mut collections = Vec::new();
        for name in db.list_collections() {
            let s = db.collection(&name)?.stats();
            let indexes: Vec<serde_json::Value> = s
                .payload_indexes
                .iter()
                .map(|(field, kind)| serde_json::json!([field, payload_index_kind_str(*kind)]))
                .collect();
            collections.push(serde_json::json!({
                "name": s.name,
                "dimension": s.dimension,
                "len": s.len,
                "tombstones": s.tombstones,
                "auto_embed": s.auto_embed,
                "metric": metric_to_str(s.metric),
                "payload_indexes": indexes,
            }));
        }
        let aliases: Vec<serde_json::Value> = db
            .aliases()
            .into_iter()
            .map(|(a, t)| serde_json::json!([a, t]))
            .collect();
        let value = serde_json::json!({
            "format_version": vl_format_version(),
            "collections": collections,
            "aliases": aliases,
        });
        unsafe { *out = buf_from_vec(encode(&value, codec)?) };
        Ok(())
    })
}

fn payload_index_kind_str(kind: PayloadIndexKind) -> &'static str {
    match kind {
        PayloadIndexKind::Keyword => "keyword",
        PayloadIndexKind::Integer => "integer",
        PayloadIndexKind::Float => "float",
    }
}

// ── chunking (pure text utility) ─────────────────────────────────────────────
#[derive(serde::Deserialize, Default)]
struct ChunkOpts {
    #[serde(default)]
    max_chars: Option<usize>,
    #[serde(default)]
    overlap: Option<usize>,
}

/// Split `text` into overlapping chunks. `opts` (optional JSON/MessagePack) is
/// `{ max_chars?, overlap? }`. Encodes `[{ text, start, end }]` into `out`
/// (freed with `vl_buf_free`). A pure function — no database needed.
///
/// # Safety
/// `text` valid; `opts` valid or null/0; `out` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vl_chunk(
    text: *const c_char,
    opts: *const u8,
    opts_len: usize,
    codec: u8,
    out: *mut vl_buf,
) -> i32 {
    ffi(|| {
        if out.is_null() {
            return Err(invalid("null out pointer"));
        }
        let text = unsafe { cstr(text) }?;
        let obytes = unsafe { opt_slice(opts, opts_len) };
        let co: ChunkOpts = if obytes.is_empty() {
            ChunkOpts::default()
        } else {
            serde_json::from_value(decode_value(obytes, codec)?)
                .map_err(|e| invalid(&format!("chunk opts: {e}")))?
        };
        let mut options = ChunkOptions::default();
        if let Some(m) = co.max_chars {
            options.max_chars = m;
        }
        if let Some(o) = co.overlap {
            options.overlap = o;
        }
        let chunks: Vec<serde_json::Value> = Chunker::new(options)
            .chunk(text)
            .into_iter()
            .map(|ch| {
                serde_json::json!({
                    "text": ch.text,
                    "start": ch.byte_range.start,
                    "end": ch.byte_range.end,
                })
            })
            .collect();
        unsafe { *out = buf_from_vec(encode(&chunks, codec)?) };
        Ok(())
    })
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
