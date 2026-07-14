# Advisory file locking must go on the same handle the layer already does I/O on

**Category**: code
**Tags**: storage, locking, windows, fs4, veclite, phase2c

## Description

On Windows, `LockFileEx` (what fs4's `try_lock_exclusive` calls) blocks I/O from OTHER file handles to the locked byte range — including other handles owned by the SAME process. If you open a second `File` just to hold the lock (a dedicated FileLock struct) while the pager reads/writes the database through its own separate handle, every pager read fails with ERROR_LOCK_VIOLATION (os error 33): "another process has locked a portion of the file". POSIX `flock` is more forgiving, but the correct cross-platform design is to take the advisory lock on the very handle that does the reads and writes. Same-handle I/O is always permitted under its own lock; a different process opening the file gets `Locked` (fail-fast), which is exactly the intent. Also lock BEFORE the first read so a conflict surfaces as `Locked`, not a mid-read I/O error.

## Example

// storage/pager.rs — lock the pager's OWN handle, not a side fd
fn lock_file(file: &std::fs::File, exclusive: bool) -> Result<()> {
    use fs4::fs_std::FileExt; // UFCS: std's inherent try_lock_* (1.89, TryLockError) doesn't exist on MSRV 1.85
    let acquired = if exclusive { FileExt::try_lock_exclusive(file)? } else { FileExt::try_lock_shared(file)? };
    if acquired { Ok(()) } else { Err(VecLiteError::Locked) }
}
// Pager::open: lock_file(&file, exclusive)?  BEFORE any seek/read_exact.
// Anti-pattern: struct FileLock { _f: File }  // second handle → self-inflicted ERROR_LOCK_VIOLATION on Windows

## When to Use

Any time you add advisory locking to a file that the same code already reads/writes through an owned handle (pagers, WAL, single-writer DBs).

## When NOT to Use

When the lock guards a resource you never do byte I/O on from the same process (e.g. a pure lockfile sentinel with no data in it).
