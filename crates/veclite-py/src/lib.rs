//! Python binding for VecLite (SPEC-009), via PyO3 directly on the Rust core.
//! NumPy `float32` buffers are borrowed without an intermediate Python copy on
//! search and batch upsert (PY-020..022), the GIL is released around every core
//! call (PY-030), and every `VecLiteError` variant surfaces as a dedicated
//! exception subclass carrying the identical Rust message (PY-040).

use std::cell::RefCell;

use numpy::{PyArray1, PyArray2, PyArrayMethods};
use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pythonize::{depythonize, pythonize};

use veclite::chunk::{ChunkOptions, Chunker};
use veclite::embedding::Embedder;
use veclite::{CollectionOptions, Filter, Hit, Metric, Point, Quantization, SparseVector};

thread_local! {
    /// When a Python `register_embedder` callback raises, its original `PyErr`
    /// is stashed here so [`to_pyerr`] can chain it as the `__cause__` of the
    /// surfaced `VecLiteError` (PY-013). The core turns the callback failure into
    /// a `VecLiteError` that carries no Python object, so this thread-local
    /// carries the original exception across the Rust boundary. It is set only
    /// immediately before returning the sentinel error and consumed by the very
    /// next `to_pyerr`, so it never goes stale.
    static EMBEDDER_ERR: RefCell<Option<PyErr>> = const { RefCell::new(None) };
}

/// Stash a Python embedder exception and return the sentinel core error that
/// propagates it out through the Rust core.
fn stash_embedder_err(e: PyErr) -> veclite::VecLiteError {
    EMBEDDER_ERR.with(|slot| *slot.borrow_mut() = Some(e));
    veclite::VecLiteError::InvalidArgument("python embedder callback raised".to_owned())
}

// ── exception hierarchy (PY-040) ─────────────────────────────────────────────
create_exception!(veclite, VecLiteError, PyException, "Base VecLite error.");
create_exception!(veclite, CollectionNotFound, VecLiteError);
create_exception!(veclite, VectorNotFound, VecLiteError);
create_exception!(veclite, AlreadyExists, VecLiteError);
create_exception!(veclite, DimensionMismatch, VecLiteError);
create_exception!(veclite, Locked, VecLiteError);
create_exception!(veclite, WalPending, VecLiteError);
create_exception!(veclite, ReadOnly, VecLiteError);
create_exception!(veclite, Closed, VecLiteError);
create_exception!(veclite, Corrupt, VecLiteError);
create_exception!(veclite, UnsupportedFormat, VecLiteError);
create_exception!(veclite, UnsupportedProvider, VecLiteError);
create_exception!(veclite, InvalidArgument, VecLiteError);
create_exception!(veclite, IoError, VecLiteError);

/// Map a core error to its dedicated Python exception with the identical message.
/// If the failure originated in a Python `register_embedder` callback, the
/// original exception is chained as the surfaced error's `__cause__` (PY-013).
fn to_pyerr(e: veclite::VecLiteError) -> PyErr {
    let msg = e.to_string();
    let err = match e {
        veclite::VecLiteError::CollectionNotFound(_) => CollectionNotFound::new_err(msg),
        veclite::VecLiteError::VectorNotFound(_) => VectorNotFound::new_err(msg),
        veclite::VecLiteError::AlreadyExists(_) => AlreadyExists::new_err(msg),
        veclite::VecLiteError::DimensionMismatch { .. } => DimensionMismatch::new_err(msg),
        veclite::VecLiteError::Locked => Locked::new_err(msg),
        veclite::VecLiteError::WalPending => WalPending::new_err(msg),
        veclite::VecLiteError::ReadOnly => ReadOnly::new_err(msg),
        veclite::VecLiteError::Closed => Closed::new_err(msg),
        veclite::VecLiteError::Corrupt(_) => Corrupt::new_err(msg),
        veclite::VecLiteError::UnsupportedFormatVersion { .. } => UnsupportedFormat::new_err(msg),
        veclite::VecLiteError::UnsupportedProvider { .. } => UnsupportedProvider::new_err(msg),
        veclite::VecLiteError::InvalidArgument(_) => InvalidArgument::new_err(msg),
        veclite::VecLiteError::Io(_) => IoError::new_err(msg),
        // `VecLiteError` is #[non_exhaustive]; a future variant maps to the base.
        _ => VecLiteError::new_err(msg),
    };
    // Chain the original Python embedder exception, if this error carried one.
    if let Some(cause) = EMBEDDER_ERR.with(|slot| slot.borrow_mut().take()) {
        Python::with_gil(|py| err.set_cause(py, Some(cause)));
    }
    err
}

fn metric_from_str(s: &str) -> PyResult<Metric> {
    match s {
        "cosine" => Ok(Metric::Cosine),
        "euclidean" | "l2" => Ok(Metric::Euclidean),
        "dot" | "dotproduct" | "dot_product" => Ok(Metric::DotProduct),
        other => Err(InvalidArgument::new_err(format!(
            "unknown metric '{other}'"
        ))),
    }
}

/// Extract a query/upsert vector: a NumPy `float32` array (borrowed) or any
/// Python sequence of floats (copied).
fn extract_vector(obj: &Bound<'_, PyAny>) -> PyResult<Vec<f32>> {
    if let Ok(arr) = obj.downcast::<PyArray1<f32>>() {
        return Ok(arr.readonly().as_slice()?.to_vec());
    }
    obj.extract::<Vec<f32>>()
}

/// Convert a JSON payload to a Python object (or `None`).
fn payload_to_py(py: Python<'_>, payload: Option<serde_json::Value>) -> PyResult<PyObject> {
    match payload {
        Some(v) => Ok(pythonize(py, &v)?.unbind()),
        None => Ok(py.None()),
    }
}

/// Convert a Python payload object (dict/`None`) to a JSON value.
fn payload_from_py(obj: Option<&Bound<'_, PyAny>>) -> PyResult<Option<serde_json::Value>> {
    match obj {
        None => Ok(None),
        Some(o) if o.is_none() => Ok(None),
        Some(o) => Ok(Some(depythonize(o)?)),
    }
}

fn hit_to_py(py: Python<'_>, h: Hit) -> PyResult<PyObject> {
    let d = PyDict::new(py);
    d.set_item("id", h.id)?;
    d.set_item("score", h.score)?;
    d.set_item("payload", payload_to_py(py, h.payload)?)?;
    if let Some(v) = h.vector {
        d.set_item("vector", v)?;
    }
    Ok(d.into())
}

fn hits_to_py(py: Python<'_>, hits: Vec<Hit>) -> PyResult<Vec<PyObject>> {
    hits.into_iter().map(|h| hit_to_py(py, h)).collect()
}

// ── custom Python embedders (PY-013) ─────────────────────────────────────────
/// A `veclite::Embedder` backed by a Python object. The object must expose
/// `embed(str) -> list[float] | np.ndarray` and a `dimension` property; `fit`,
/// `export_state`, and `import_state` are used when present, else treated as
/// no-ops (a stateless embedder). Every callback runs under the GIL, which the
/// core has released via `allow_threads` before reaching here, so re-acquiring
/// is safe. A raised Python exception is stashed (see [`stash_embedder_err`]) and
/// surfaces as a chained `VecLiteError`.
struct PyEmbedder {
    obj: Py<PyAny>,
    /// The dimension is read once at registration; embedders have a fixed width.
    dimension: usize,
}

/// Convert an embedder's return value — a `float32`/`float64` NumPy array or any
/// sequence of floats — into a `Vec<f32>`.
fn extract_embedding(obj: &Bound<'_, PyAny>) -> PyResult<Vec<f32>> {
    if let Ok(arr) = obj.downcast::<PyArray1<f32>>() {
        return Ok(arr.readonly().as_slice()?.to_vec());
    }
    obj.extract::<Vec<f32>>()
}

impl Embedder for PyEmbedder {
    fn embed(&self, text: &str) -> veclite::error::Result<Vec<f32>> {
        Python::with_gil(|py| {
            self.obj
                .bind(py)
                .call_method1("embed", (text,))
                .and_then(|r| extract_embedding(&r))
                .map_err(stash_embedder_err)
        })
    }

    fn embed_batch(&self, texts: &[&str]) -> veclite::error::Result<Vec<Vec<f32>>> {
        Python::with_gil(|py| {
            let obj = self.obj.bind(py);
            // Prefer a batch method when the object provides one; else fall back
            // to per-text embed (the trait default, but kept on one GIL hold).
            if obj.hasattr("embed_batch").unwrap_or(false) {
                let out = obj
                    .call_method1("embed_batch", (texts.to_vec(),))
                    .map_err(stash_embedder_err)?;
                let rows = out.try_iter().map_err(stash_embedder_err)?;
                rows.map(|row| {
                    let row = row.map_err(stash_embedder_err)?;
                    extract_embedding(&row).map_err(stash_embedder_err)
                })
                .collect()
            } else {
                texts
                    .iter()
                    .map(|t| {
                        obj.call_method1("embed", (*t,))
                            .and_then(|r| extract_embedding(&r))
                            .map_err(stash_embedder_err)
                    })
                    .collect()
            }
        })
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn fit(&mut self, corpus: &[&str]) -> veclite::error::Result<()> {
        Python::with_gil(|py| {
            let obj = self.obj.bind(py);
            if !obj.hasattr("fit").unwrap_or(false) {
                return Ok(()); // stateless embedder
            }
            obj.call_method1("fit", (corpus.to_vec(),))
                .map(|_| ())
                .map_err(stash_embedder_err)
        })
    }

    fn export_state(&self) -> veclite::error::Result<Vec<u8>> {
        Python::with_gil(|py| {
            let obj = self.obj.bind(py);
            if !obj.hasattr("export_state").unwrap_or(false) {
                return Ok(Vec::new()); // stateless: nothing to persist
            }
            obj.call_method0("export_state")
                .and_then(|r| r.extract::<Vec<u8>>())
                .map_err(stash_embedder_err)
        })
    }

    fn import_state(&mut self, state: &[u8]) -> veclite::error::Result<()> {
        Python::with_gil(|py| {
            let obj = self.obj.bind(py);
            if !obj.hasattr("import_state").unwrap_or(false) {
                return Ok(());
            }
            obj.call_method1("import_state", (state.to_vec(),))
                .map(|_| ())
                .map_err(stash_embedder_err)
        })
    }
}

// ── Collection ───────────────────────────────────────────────────────────────
/// A handle to a collection (SPEC-009).
#[pyclass]
struct Collection {
    inner: veclite::Collection,
}

#[pymethods]
impl Collection {
    /// Insert-or-replace one point with an optional dict payload and an optional
    /// `{indices, values}` sparse lane for hybrid search (SPEC-007).
    #[pyo3(signature = (id, vector, payload=None, sparse=None))]
    fn upsert(
        &self,
        py: Python<'_>,
        id: &str,
        vector: &Bound<'_, PyAny>,
        payload: Option<&Bound<'_, PyAny>>,
        sparse: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let vec = extract_vector(vector)?;
        let payload = payload_from_py(payload)?;
        let mut point = Point::new(id, vec);
        if let Some(p) = payload {
            point = point.payload(p);
        }
        if let Some(s) = sparse.filter(|s| !s.is_none()) {
            let sv: SparseVector = depythonize(s)?;
            point = point.sparse(sv);
        }
        py.allow_threads(|| self.inner.upsert(point))
            .map_err(to_pyerr)
    }

    /// Batch upsert from a `(n, dim)` `float32` NumPy array (or a list of lists),
    /// borrowing the buffer without a per-row Python copy (PY-020).
    #[pyo3(signature = (ids, vectors, payloads=None))]
    fn upsert_batch(
        &self,
        py: Python<'_>,
        ids: Vec<String>,
        vectors: &Bound<'_, PyAny>,
        payloads: Option<Vec<Bound<'_, PyAny>>>,
    ) -> PyResult<()> {
        let rows: Vec<Vec<f32>> = if let Ok(arr) = vectors.downcast::<PyArray2<f32>>() {
            let ro = arr.readonly();
            let view = ro.as_array();
            if view.nrows() != ids.len() {
                return Err(InvalidArgument::new_err(format!(
                    "{} ids but {} vector rows",
                    ids.len(),
                    view.nrows()
                )));
            }
            view.rows().into_iter().map(|r| r.to_vec()).collect()
        } else {
            vectors.extract()?
        };
        if rows.len() != ids.len() {
            return Err(InvalidArgument::new_err("ids and vectors length mismatch"));
        }
        let payloads = match payloads {
            Some(ps) => {
                if ps.len() != ids.len() {
                    return Err(InvalidArgument::new_err("ids and payloads length mismatch"));
                }
                ps.iter()
                    .map(|p| payload_from_py(Some(p)))
                    .collect::<PyResult<Vec<_>>>()?
            }
            None => vec![None; ids.len()],
        };
        let points: Vec<Point> = ids
            .into_iter()
            .zip(rows)
            .zip(payloads)
            .map(|((id, vec), payload)| {
                let mut p = Point::new(id, vec);
                if let Some(pl) = payload {
                    p = p.payload(pl);
                }
                p
            })
            .collect();
        py.allow_threads(|| self.inner.upsert_batch(points))
            .map_err(to_pyerr)
    }

    /// Insert-or-replace one text document (auto-embed collections).
    #[pyo3(signature = (id, text, payload=None))]
    fn upsert_text(
        &self,
        py: Python<'_>,
        id: &str,
        text: &str,
        payload: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        match payload_from_py(payload)? {
            Some(p) => py.allow_threads(|| self.inner.upsert_text_with(id, text, p)),
            None => py.allow_threads(|| self.inner.upsert_text(id, text)),
        }
        .map_err(to_pyerr)
    }

    /// Fetch one point as a dict, or `None` if absent.
    fn get(&self, py: Python<'_>, id: &str) -> PyResult<Option<PyObject>> {
        let point = py.allow_threads(|| self.inner.get(id)).map_err(to_pyerr)?;
        match point {
            None => Ok(None),
            Some(p) => {
                let d = PyDict::new(py);
                d.set_item("id", p.id)?;
                d.set_item("vector", p.vector)?;
                d.set_item("payload", payload_to_py(py, p.payload)?)?;
                Ok(Some(d.into()))
            }
        }
    }

    /// Delete one id; returns whether it existed.
    fn delete(&self, py: Python<'_>, id: &str) -> PyResult<bool> {
        py.allow_threads(|| self.inner.delete(id)).map_err(to_pyerr)
    }

    /// Number of live vectors.
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// k-NN search. `filter` is a Qdrant-style dict (SPEC-006).
    #[pyo3(signature = (vector, limit=10, ef_search=None, with_payload=true, with_vector=false, filter=None))]
    #[allow(clippy::too_many_arguments)]
    fn search(
        &self,
        py: Python<'_>,
        vector: &Bound<'_, PyAny>,
        limit: usize,
        ef_search: Option<usize>,
        with_payload: bool,
        with_vector: bool,
        filter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Vec<PyObject>> {
        let filter = match filter {
            Some(f) if !f.is_none() => Some(Filter::from_json(&depythonize(f)?).map_err(to_pyerr)?),
            _ => None,
        };
        // Zero-copy borrow for a float32 NumPy array; copy for a list.
        let hits = if let Ok(arr) = vector.downcast::<PyArray1<f32>>() {
            let ro = arr.readonly();
            let slice = ro.as_slice()?;
            py.allow_threads(|| {
                run_query(
                    &self.inner,
                    slice,
                    limit,
                    ef_search,
                    with_payload,
                    with_vector,
                    filter,
                )
            })
        } else {
            let v = vector.extract::<Vec<f32>>()?;
            py.allow_threads(|| {
                run_query(
                    &self.inner,
                    &v,
                    limit,
                    ef_search,
                    with_payload,
                    with_vector,
                    filter,
                )
            })
        }
        .map_err(to_pyerr)?;
        hits_to_py(py, hits)
    }

    /// Text search (auto-embed collections).
    #[pyo3(signature = (query, limit=10))]
    fn search_text(&self, py: Python<'_>, query: &str, limit: usize) -> PyResult<Vec<PyObject>> {
        let hits = py
            .allow_threads(|| self.inner.search_text(query, limit))
            .map_err(to_pyerr)?;
        hits_to_py(py, hits)
    }

    /// Hybrid dense+sparse search. `sparse` is `{indices, values}` (SPEC-007).
    #[pyo3(signature = (vector, sparse, limit=10, alpha=0.5, rrf_k=60.0))]
    fn hybrid_search(
        &self,
        py: Python<'_>,
        vector: &Bound<'_, PyAny>,
        sparse: &Bound<'_, PyAny>,
        limit: usize,
        alpha: f32,
        rrf_k: f32,
    ) -> PyResult<Vec<PyObject>> {
        let dense = extract_vector(vector)?;
        let sv: SparseVector = depythonize(sparse)?;
        let hits = py
            .allow_threads(|| {
                self.inner
                    .hybrid_query()
                    .dense(&dense)
                    .sparse(&sv)
                    .limit(limit)
                    .alpha(alpha)
                    .rrf_k(rrf_k)
                    .run()
            })
            .map_err(to_pyerr)?;
        hits_to_py(py, hits)
    }

    /// Cursor-based pagination over live points in stable slot order (API-022).
    /// Returns `{points: [{id, vector, payload}], next_cursor}`; pass
    /// `next_cursor` as `offset_id` for the next page (`None` when exhausted).
    #[pyo3(signature = (limit=100, offset_id=None, filter=None))]
    fn scroll(
        &self,
        py: Python<'_>,
        limit: usize,
        offset_id: Option<String>,
        filter: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyObject> {
        let filter = match filter {
            Some(f) if !f.is_none() => Some(Filter::from_json(&depythonize(f)?).map_err(to_pyerr)?),
            _ => None,
        };
        let page = py
            .allow_threads(|| {
                self.inner
                    .scroll(offset_id.as_deref(), limit, filter.as_ref())
            })
            .map_err(to_pyerr)?;
        let points: Vec<PyObject> = page
            .points
            .into_iter()
            .map(|p| {
                let d = PyDict::new(py);
                d.set_item("id", p.id)?;
                d.set_item("vector", p.vector)?;
                d.set_item("payload", payload_to_py(py, p.payload)?)?;
                Ok::<PyObject, PyErr>(d.into())
            })
            .collect::<PyResult<_>>()?;
        let out = PyDict::new(py);
        out.set_item("points", points)?;
        out.set_item("next_cursor", page.next_cursor)?;
        Ok(out.into())
    }

    /// Force a full recompute of an auto-embed collection's vocabulary.
    fn refit(&self, py: Python<'_>) -> PyResult<()> {
        py.allow_threads(|| self.inner.refit()).map_err(to_pyerr)
    }

    /// `{name, dimension, len, tombstones, auto_embed}`.
    fn stats(&self, py: Python<'_>) -> PyResult<PyObject> {
        let s = self.inner.stats();
        let d = PyDict::new(py);
        d.set_item("name", s.name)?;
        d.set_item("dimension", s.dimension)?;
        d.set_item("len", s.len)?;
        d.set_item("tombstones", s.tombstones)?;
        d.set_item("auto_embed", s.auto_embed)?;
        Ok(d.into())
    }
}

#[allow(clippy::too_many_arguments)]
fn run_query(
    coll: &veclite::Collection,
    vector: &[f32],
    limit: usize,
    ef_search: Option<usize>,
    with_payload: bool,
    with_vector: bool,
    filter: Option<Filter>,
) -> Result<Vec<Hit>, veclite::VecLiteError> {
    let mut qb = coll
        .query(vector)
        .limit(limit)
        .with_payload(with_payload)
        .with_vector(with_vector);
    if let Some(ef) = ef_search {
        qb = qb.ef_search(ef);
    }
    if let Some(f) = filter {
        qb = qb.filter(f);
    }
    qb.run()
}

// ── Database ─────────────────────────────────────────────────────────────────
/// A VecLite database (SPEC-009).
#[pyclass]
struct Database {
    inner: veclite::VecLite,
}

#[pymethods]
impl Database {
    /// Open (or create) a durable single-file database.
    #[staticmethod]
    fn open(py: Python<'_>, path: &str) -> PyResult<Self> {
        let inner = py
            .allow_threads(|| veclite::VecLite::open(path))
            .map_err(to_pyerr)?;
        Ok(Database { inner })
    }

    /// Open an ephemeral in-memory database.
    #[staticmethod]
    fn memory() -> Self {
        Database {
            inner: veclite::VecLite::memory(),
        }
    }

    /// Create a collection.
    #[pyo3(signature = (name, dimension, metric="cosine", quantization_bits=None, embedding_provider=None))]
    fn create_collection(
        &self,
        py: Python<'_>,
        name: &str,
        dimension: usize,
        metric: &str,
        quantization_bits: Option<u8>,
        embedding_provider: Option<&str>,
    ) -> PyResult<Collection> {
        let mut options = match embedding_provider {
            Some(p) => CollectionOptions::auto_embed(p, dimension),
            None => CollectionOptions::new(dimension, metric_from_str(metric)?),
        };
        if let Some(bits) = quantization_bits {
            options = options.quantization(if bits == 0 {
                Quantization::None
            } else {
                Quantization::Scalar { bits }
            });
        }
        let inner = py
            .allow_threads(|| self.inner.create_collection(name, options))
            .map_err(to_pyerr)?;
        Ok(Collection { inner })
    }

    /// Get a collection by name or alias.
    fn collection(&self, name: &str) -> PyResult<Collection> {
        let inner = self.inner.collection(name).map_err(to_pyerr)?;
        Ok(Collection { inner })
    }

    /// Drop a collection.
    fn delete_collection(&self, py: Python<'_>, name: &str) -> PyResult<()> {
        py.allow_threads(|| self.inner.delete_collection(name))
            .map_err(to_pyerr)
    }

    /// Sorted collection names.
    fn list_collections(&self) -> Vec<String> {
        self.inner.list_collections()
    }

    /// Create an alias resolving to `target`.
    fn create_alias(&self, alias: &str, target: &str) -> PyResult<()> {
        self.inner.create_alias(alias, target).map_err(to_pyerr)
    }

    /// Delete an alias.
    fn delete_alias(&self, alias: &str) -> PyResult<()> {
        self.inner.delete_alias(alias).map_err(to_pyerr)
    }

    /// `(alias, target)` pairs.
    fn aliases(&self) -> Vec<(String, String)> {
        self.inner.aliases()
    }

    /// Force a checkpoint.
    fn checkpoint(&self, py: Python<'_>) -> PyResult<()> {
        py.allow_threads(|| self.inner.checkpoint())
            .map_err(to_pyerr)
    }

    /// Register a custom embedding provider implemented in Python (PY-013). `obj`
    /// must expose `embed(str) -> list[float] | np.ndarray` and a `dimension`
    /// property; `embed_batch`, `fit`, `export_state`, and `import_state` are
    /// used when present. Auto-embed collections created with
    /// `embedding_provider="name"` then route through `obj`. Exceptions raised in
    /// the callback surface as a `VecLiteError` with the original chained.
    fn register_embedder(&self, name: &str, obj: Bound<'_, PyAny>) -> PyResult<()> {
        if !obj.hasattr("embed")? {
            return Err(InvalidArgument::new_err(
                "embedder object must define embed(text)",
            ));
        }
        let dimension = obj
            .getattr("dimension")
            .map_err(|_| InvalidArgument::new_err("embedder object must have a `dimension`"))?
            .extract::<usize>()
            .map_err(|_| InvalidArgument::new_err("embedder `dimension` must be an int"))?;
        if dimension == 0 {
            return Err(InvalidArgument::new_err("embedder `dimension` must be > 0"));
        }
        let embedder = PyEmbedder {
            obj: obj.unbind(),
            dimension,
        };
        self.inner
            .register_embedder(name, Box::new(embedder))
            .map_err(to_pyerr)
    }
}

/// Split `text` into overlapping, UTF-8-safe chunks (SPEC-005 §7). Returns a
/// list of `{text, start, end}` dicts; deterministic for a given input.
#[pyfunction]
#[pyo3(signature = (text, max_chars=2048, overlap=128))]
fn chunk(py: Python<'_>, text: &str, max_chars: usize, overlap: usize) -> PyResult<Vec<PyObject>> {
    Chunker::new(ChunkOptions { max_chars, overlap })
        .chunk(text)
        .into_iter()
        .map(|c| {
            let d = PyDict::new(py);
            d.set_item("text", c.text)?;
            d.set_item("start", c.byte_range.start)?;
            d.set_item("end", c.byte_range.end)?;
            Ok(d.into())
        })
        .collect()
}

/// The compiled `veclite._veclite` extension module. The pure-Python `veclite`
/// package (python/veclite/__init__.py) re-exports it and adds the lazy
/// `veclite.aio` async facade (PY-031).
#[pymodule]
#[pyo3(name = "_veclite")]
fn veclite_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("format_version", 1u32)?;
    m.add_class::<Database>()?;
    m.add_class::<Collection>()?;
    m.add_function(wrap_pyfunction!(chunk, m)?)?;

    let py = m.py();
    m.add("VecLiteError", py.get_type::<VecLiteError>())?;
    m.add("CollectionNotFound", py.get_type::<CollectionNotFound>())?;
    m.add("VectorNotFound", py.get_type::<VectorNotFound>())?;
    m.add("AlreadyExists", py.get_type::<AlreadyExists>())?;
    m.add("DimensionMismatch", py.get_type::<DimensionMismatch>())?;
    m.add("Locked", py.get_type::<Locked>())?;
    m.add("WalPending", py.get_type::<WalPending>())?;
    m.add("ReadOnly", py.get_type::<ReadOnly>())?;
    m.add("Closed", py.get_type::<Closed>())?;
    m.add("Corrupt", py.get_type::<Corrupt>())?;
    m.add("UnsupportedFormat", py.get_type::<UnsupportedFormat>())?;
    m.add("UnsupportedProvider", py.get_type::<UnsupportedProvider>())?;
    m.add("InvalidArgument", py.get_type::<InvalidArgument>())?;
    m.add("IoError", py.get_type::<IoError>())?;
    Ok(())
}
