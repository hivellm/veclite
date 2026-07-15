//! Error type for all fallible VecLite operations.
//!
//! One `thiserror` enum with stable, matchable variants (SPEC-004 §6).
//! Variants and their FFI codes (SPEC-008 §3) never change meaning within a
//! major version; new variants may be added in minors (`#[non_exhaustive]`).

/// Convenience alias used across the crate and re-exported at the root.
pub type Result<T> = core::result::Result<T, VecLiteError>;

/// Every error VecLite can return.
///
/// Display strings are part of the cross-binding contract: the conformance
/// corpus (SPEC-015 §3) pins them, and bindings surface them verbatim.
#[non_exhaustive]
#[derive(thiserror::Error, Debug)]
pub enum VecLiteError {
    /// The named collection does not exist (nor as an alias).
    #[error("collection not found: {0}")]
    CollectionNotFound(String),

    /// No vector with the given id exists in the collection.
    #[error("vector not found: {0}")]
    VectorNotFound(String),

    /// A collection (or alias) with this name already exists.
    #[error("collection already exists: {0}")]
    AlreadyExists(String),

    /// A vector's length does not match the collection dimension.
    /// Never silently coerced (SPEC-001 CORE-012).
    #[error("dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch {
        /// The collection's configured dimension.
        expected: usize,
        /// The length of the vector that was supplied.
        got: usize,
    },

    /// Another process holds a conflicting advisory lock (SPEC-002 STG-060).
    #[error("database is locked by another process")]
    Locked,

    /// A read-only open found a non-empty WAL (SPEC-003 WAL-043).
    #[error(
        "write-ahead log has pending entries; open read-write to recover \
         or set read_only_ignore_wal"
    )]
    WalPending,

    /// A mutating call was made on a read-only database.
    #[error("read-only database")]
    ReadOnly,

    /// The database handle was closed; the operation cannot proceed.
    #[error("database is closed")]
    Closed,

    /// Integrity failure: checksum mismatch or malformed structure. The
    /// message names the damaged element (e.g. `segment@<offset>`).
    #[error("file is corrupt: {0}")]
    Corrupt(String),

    /// The file was written by a newer format than this build reads
    /// (SPEC-002 header `min_reader_version`).
    #[error("format version {found} newer than supported {supported}")]
    UnsupportedFormatVersion {
        /// `min_reader_version` recorded in the file header.
        found: u32,
        /// Highest format version this build supports.
        supported: u32,
    },

    /// An embedding provider name is unknown or unavailable in this build.
    /// Never falls back silently (SPEC-005 EMB-021).
    #[error("unknown embedding provider: {requested}; available: {available:?}")]
    UnsupportedProvider {
        /// The provider name that was requested.
        requested: String,
        /// Provider names available in this build/database.
        available: Vec<String>,
    },

    /// Caller misuse detected by validation (bad name, bad bounds, NaN
    /// vector, unsupported filter feature, ...). The message says what.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// An underlying I/O operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl VecLiteError {
    /// The stable C-ABI error code for this variant (SPEC-008 §3). The match is
    /// exhaustive, so adding a new `VecLiteError` variant without assigning it a
    /// code fails the build (SPEC-008 acceptance 3). Codes are never renumbered
    /// within a major version. `0` means success and is never returned here.
    #[must_use]
    pub fn ffi_code(&self) -> i32 {
        match self {
            VecLiteError::CollectionNotFound(_) => -1,
            VecLiteError::VectorNotFound(_) => -2,
            VecLiteError::AlreadyExists(_) => -3,
            VecLiteError::DimensionMismatch { .. } => -4,
            VecLiteError::Locked => -5,
            VecLiteError::Corrupt(_) => -6,
            VecLiteError::UnsupportedFormatVersion { .. } => -7,
            VecLiteError::UnsupportedProvider { .. } => -8,
            VecLiteError::ReadOnly => -9,
            VecLiteError::InvalidArgument(_) => -10,
            VecLiteError::Io(_) => -11,
            VecLiteError::WalPending => -12,
            VecLiteError::Closed => -13,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Display strings are contract (SPEC-015 TST-021): pin every variant.
    #[test]
    fn display_strings_are_stable() {
        let cases: Vec<(VecLiteError, &str)> = vec![
            (
                VecLiteError::CollectionNotFound("docs".into()),
                "collection not found: docs",
            ),
            (
                VecLiteError::VectorNotFound("id-1".into()),
                "vector not found: id-1",
            ),
            (
                VecLiteError::AlreadyExists("docs".into()),
                "collection already exists: docs",
            ),
            (
                VecLiteError::DimensionMismatch {
                    expected: 384,
                    got: 100,
                },
                "dimension mismatch: expected 384, got 100",
            ),
            (
                VecLiteError::Locked,
                "database is locked by another process",
            ),
            (
                VecLiteError::WalPending,
                "write-ahead log has pending entries; open read-write to \
                 recover or set read_only_ignore_wal",
            ),
            (VecLiteError::ReadOnly, "read-only database"),
            (VecLiteError::Closed, "database is closed"),
            (
                VecLiteError::Corrupt("segment@4096".into()),
                "file is corrupt: segment@4096",
            ),
            (
                VecLiteError::UnsupportedFormatVersion {
                    found: 2,
                    supported: 1,
                },
                "format version 2 newer than supported 1",
            ),
            (
                VecLiteError::UnsupportedProvider {
                    requested: "bm52".into(),
                    available: vec!["bm25".into()],
                },
                "unknown embedding provider: bm52; available: [\"bm25\"]",
            ),
            (
                VecLiteError::InvalidArgument("limit must be > 0".into()),
                "invalid argument: limit must be > 0",
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn io_errors_convert() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let err: VecLiteError = io.into();
        assert!(matches!(err, VecLiteError::Io(_)));
        assert_eq!(err.to_string(), "gone");
    }

    /// Pins the full FFI error-code table (SPEC-008 acceptance 3): one code
    /// per variant, stable within a major version.
    #[test]
    fn ffi_codes_are_pinned_per_variant() {
        let io = std::io::Error::other("x");
        let cases: Vec<(VecLiteError, i32)> = vec![
            (VecLiteError::CollectionNotFound("c".into()), -1),
            (VecLiteError::VectorNotFound("v".into()), -2),
            (VecLiteError::AlreadyExists("c".into()), -3),
            (
                VecLiteError::DimensionMismatch {
                    expected: 3,
                    got: 2,
                },
                -4,
            ),
            (VecLiteError::Locked, -5),
            (VecLiteError::Corrupt("bad".into()), -6),
            (
                VecLiteError::UnsupportedFormatVersion {
                    found: 9,
                    supported: 1,
                },
                -7,
            ),
            (
                VecLiteError::UnsupportedProvider {
                    requested: "x".into(),
                    available: vec!["bm25".into()],
                },
                -8,
            ),
            (VecLiteError::ReadOnly, -9),
            (VecLiteError::InvalidArgument("a".into()), -10),
            (VecLiteError::Io(io), -11),
            (VecLiteError::WalPending, -12),
            (VecLiteError::Closed, -13),
        ];
        for (err, want) in cases {
            assert_eq!(err.ffi_code(), want, "{err}");
            assert!(!err.to_string().is_empty());
        }
    }
}
