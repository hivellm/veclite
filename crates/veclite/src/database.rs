//! Database handle and collection registry (SPEC-001 §4, SPEC-004 §1–2).
//!
//! Concurrency model (CORE-051): collection lookups are lock-free reads on a
//! `DashMap`; the rare registry mutations (create/delete/rename) serialize on
//! one mutex so two-key operations like rename stay atomic.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use dashmap::DashMap;
use parking_lot::Mutex;

use crate::collection::{Collection, CollectionInner};
use crate::error::{Result, VecLiteError};
use crate::options::CollectionOptions;
use crate::point::validate_collection_name;

/// Maximum collection dimension (SPEC-002 §8 limits).
const MAX_DIMENSION: usize = 65_536;

struct DatabaseInner {
    collections: DashMap<String, Arc<CollectionInner>>,
    /// Serializes create/delete/rename (registry-level write, CORE-051).
    registry: Mutex<()>,
}

/// Database handle. Cheap to clone; `Send + Sync` (CORE-050). Multiple
/// databases in one process are fully independent — no global state.
///
/// ```
/// use veclite::{CollectionOptions, Metric, Point, VecLite};
///
/// let db = VecLite::memory();
/// let docs = db.create_collection("docs", CollectionOptions::new(3, Metric::Cosine))?;
/// docs.upsert(Point::new("id-1", vec![0.1, 0.2, 0.3]))?;
/// assert_eq!(docs.len(), 1);
/// # Ok::<(), veclite::VecLiteError>(())
/// ```
#[derive(Clone)]
pub struct VecLite {
    inner: Arc<DatabaseInner>,
}

impl VecLite {
    /// Ephemeral in-memory database: no file, no WAL, identical API (FR-02).
    pub fn memory() -> Self {
        VecLite {
            inner: Arc::new(DatabaseInner {
                collections: DashMap::new(),
                registry: Mutex::new(()),
            }),
        }
    }

    /// Create a collection; `AlreadyExists` when the name is taken
    /// (CORE-020).
    pub fn create_collection(&self, name: &str, options: CollectionOptions) -> Result<Collection> {
        validate_collection_name(name)?;
        if options.dimension == 0 || options.dimension > MAX_DIMENSION {
            return Err(VecLiteError::InvalidArgument(format!(
                "dimension must be in 1..={MAX_DIMENSION}, got {}",
                options.dimension
            )));
        }
        let _guard = self.inner.registry.lock();
        if self.inner.collections.contains_key(name) {
            return Err(VecLiteError::AlreadyExists(name.to_owned()));
        }
        let inner = Arc::new(CollectionInner::new(name.to_owned(), options));
        self.inner
            .collections
            .insert(name.to_owned(), Arc::clone(&inner));
        Ok(Collection { inner })
    }

    /// Handle to an existing collection (lock-free lookup, CORE-051).
    pub fn collection(&self, name: &str) -> Result<Collection> {
        match self.inner.collections.get(name) {
            Some(entry) => Ok(Collection {
                inner: Arc::clone(entry.value()),
            }),
            None => Err(VecLiteError::CollectionNotFound(name.to_owned())),
        }
    }

    /// Drop a collection; open handles fail with `CollectionNotFound` from
    /// then on (CORE-021).
    pub fn delete_collection(&self, name: &str) -> Result<()> {
        let _guard = self.inner.registry.lock();
        match self.inner.collections.remove(name) {
            Some((_, inner)) => {
                inner.deleted.store(true, Ordering::Release);
                Ok(())
            }
            None => Err(VecLiteError::CollectionNotFound(name.to_owned())),
        }
    }

    /// Rename a collection. Metadata-only, O(1) in vector count (CORE-022);
    /// existing handles keep working under the new name.
    pub fn rename_collection(&self, from: &str, to: &str) -> Result<()> {
        validate_collection_name(to)?;
        let _guard = self.inner.registry.lock();
        if self.inner.collections.contains_key(to) {
            return Err(VecLiteError::AlreadyExists(to.to_owned()));
        }
        match self.inner.collections.remove(from) {
            Some((_, inner)) => {
                *inner.name.write() = to.to_owned();
                self.inner.collections.insert(to.to_owned(), inner);
                Ok(())
            }
            None => Err(VecLiteError::CollectionNotFound(from.to_owned())),
        }
    }

    /// Names of all collections, sorted for deterministic output.
    pub fn list_collections(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .inner
            .collections
            .iter()
            .map(|e| e.key().clone())
            .collect();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::Metric;
    use crate::point::Point;

    fn opts() -> CollectionOptions {
        CollectionOptions::new(2, Metric::Euclidean)
    }

    #[test]
    fn create_get_list_delete() {
        let db = VecLite::memory();
        db.create_collection("b", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        db.create_collection("a", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(db.list_collections(), vec!["a", "b"]);
        assert!(db.collection("a").is_ok());
        db.delete_collection("a").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(db.list_collections(), vec!["b"]);
        assert!(matches!(
            db.collection("a"),
            Err(VecLiteError::CollectionNotFound(_))
        ));
    }

    #[test]
    fn duplicate_names_rejected() {
        let db = VecLite::memory();
        db.create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            db.create_collection("docs", opts()),
            Err(VecLiteError::AlreadyExists(_))
        ));
    }

    #[test]
    fn dimension_bounds_enforced_at_create() {
        let db = VecLite::memory();
        assert!(matches!(
            db.create_collection("zero", CollectionOptions::new(0, Metric::Euclidean)),
            Err(VecLiteError::InvalidArgument(_))
        ));
        assert!(matches!(
            db.create_collection("huge", CollectionOptions::new(65_537, Metric::Euclidean)),
            Err(VecLiteError::InvalidArgument(_))
        ));
        assert!(
            db.create_collection("max", CollectionOptions::new(65_536, Metric::Euclidean))
                .is_ok()
        );
    }

    #[test]
    fn invalid_names_rejected_at_create() {
        let db = VecLite::memory();
        assert!(matches!(
            db.create_collection("a/b", opts()),
            Err(VecLiteError::InvalidArgument(_))
        ));
    }

    #[test]
    fn stale_handle_errors_after_delete() {
        let db = VecLite::memory();
        let c = db
            .create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![1.0, 2.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.delete_collection("docs")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            c.upsert(Point::new("b", vec![1.0, 2.0])),
            Err(VecLiteError::CollectionNotFound(_))
        ));
        assert!(matches!(
            c.get("a"),
            Err(VecLiteError::CollectionNotFound(_))
        ));
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn rename_is_metadata_only_and_handles_survive() {
        let db = VecLite::memory();
        let c = db
            .create_collection("old", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        c.upsert(Point::new("a", vec![1.0, 2.0]))
            .unwrap_or_else(|e| panic!("{e}"));
        db.rename_collection("old", "new")
            .unwrap_or_else(|e| panic!("{e}"));

        assert!(matches!(
            db.collection("old"),
            Err(VecLiteError::CollectionNotFound(_))
        ));
        assert_eq!(c.name(), "new");
        assert_eq!(c.len(), 1);
        assert_eq!(
            db.collection("new").unwrap_or_else(|e| panic!("{e}")).len(),
            1
        );

        // Renaming onto an existing name fails.
        db.create_collection("other", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(
            db.rename_collection("new", "other"),
            Err(VecLiteError::AlreadyExists(_))
        ));
        // Renaming a missing collection fails.
        assert!(matches!(
            db.rename_collection("ghost", "x"),
            Err(VecLiteError::CollectionNotFound(_))
        ));
    }

    #[test]
    fn databases_are_independent() {
        let db1 = VecLite::memory();
        let db2 = VecLite::memory();
        db1.create_collection("docs", opts())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(db2.collection("docs").is_err());
    }
}
