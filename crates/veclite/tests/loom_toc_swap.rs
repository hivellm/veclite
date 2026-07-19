//! Targeted loom models for the checkpoint/reader interleavings (SPEC-001
//! CORE-054 / SPEC-015 TST-052).
//!
//! The engine's commit protocol is file I/O (SPEC-002 §5: append segments →
//! fsync → append TOC → fsync → rewrite the header root pointer), and its
//! in-process concurrency is RwLock-guarded state plus atomic maintenance
//! flags. loom cannot interleave fsyncs — the crash suite owns that axis —
//! but it exhaustively interleaves the *memory-ordering discipline* those
//! protocols rely on. Each model here is the protocol skeleton with the same
//! ordering choices the implementation makes; loom proves every interleaving
//! upholds the invariant (and demonstrably fails if the ordering is weakened
//! — see the `_relaxed_publish_would_fail` note in the first model).

use loom::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use loom::sync::{Arc, RwLock};
use loom::thread;

/// Root-pointer swap (SPEC-002 §5, STG-050): the writer fully publishes a new
/// TOC generation *before* swinging the root pointer (release), and a reader
/// that observes the new root (acquire) must observe the complete TOC — never
/// a half-written generation. This is the in-memory analogue of the
/// fsync-before-header-rewrite rule.
#[test]
fn reader_never_observes_unpublished_toc() {
    loom::model(|| {
        // Two TOC slots (generations); 0 = "not yet written".
        let slots = Arc::new([AtomicU64::new(1), AtomicU64::new(0)]);
        // Root pointer: which slot is committed.
        let root = Arc::new(AtomicUsize::new(0));

        let writer_slots = Arc::clone(&slots);
        let writer_root = Arc::clone(&root);
        let writer = thread::spawn(move || {
            // Checkpoint: write the new generation's TOC body…
            writer_slots[1].store(2, Ordering::Relaxed);
            // …then publish it with release ordering (the header rewrite).
            writer_root.store(1, Ordering::Release);
        });

        // Reader: load the root with acquire (open/read path), then read the
        // TOC it names.
        let slot = root.load(Ordering::Acquire);
        let generation = slots[slot].load(Ordering::Relaxed);
        // Whichever root the reader saw, the TOC it points at is complete:
        // slot 0 always holds gen 1; slot 1 is only reachable after its body
        // was fully stored (release/acquire pairing).
        assert!(
            generation != 0,
            "reader observed the new root before its TOC was published"
        );
        if slot == 1 {
            assert_eq!(generation, 2);
        }

        writer.join().unwrap_or_else(|_| panic!("writer panicked"));
        // NOTE: weakening the publish to `Ordering::Relaxed` makes loom find
        // the interleaving where the reader sees slot 1 with generation 0 —
        // the model is sensitive to exactly the discipline it pins.
    });
}

/// Checkpoint snapshot vs concurrent writer (CORE-051): writers mutate two
/// tables that must stay mutually consistent (vector store + id directory)
/// under the write lock; the checkpoint takes the read lock and seals a
/// snapshot. Every interleaving must hand the checkpoint a consistent pair —
/// a torn snapshot (ids ≠ vectors) would corrupt the sealed segments.
#[test]
fn checkpoint_snapshot_is_never_torn() {
    loom::model(|| {
        // (vector_count, iddir_count) — the invariant is equality.
        let state = Arc::new(RwLock::new((0u32, 0u32)));

        let writer_state = Arc::clone(&state);
        let writer = thread::spawn(move || {
            for _ in 0..2 {
                let mut guard = writer_state.write().unwrap_or_else(|_| panic!("poisoned"));
                guard.0 += 1; // upsert: vector store first…
                guard.1 += 1; // …then the id directory, same critical section
            }
        });

        // Checkpoint thread: seal a snapshot under the read lock.
        let (vectors, iddir) = *state.read().unwrap_or_else(|_| panic!("poisoned"));
        assert_eq!(
            vectors, iddir,
            "checkpoint observed a torn store/iddir pair"
        );

        writer.join().unwrap_or_else(|_| panic!("writer panicked"));
        let (vectors, iddir) = *state.read().unwrap_or_else(|_| panic!("poisoned"));
        assert_eq!(vectors, 2);
        assert_eq!(iddir, 2);
    });
}

/// WAL-size checkpoint trigger (SPEC-003 WAL-030a): concurrent writers whose
/// appends cross the size threshold must trigger the checkpoint **exactly
/// once** per crossing — the in-flight flag is claimed with a CAS, so two
/// simultaneous crossings never run overlapping checkpoints, and a crossing
/// is never dropped entirely.
#[test]
fn threshold_checkpoint_runs_exactly_once() {
    loom::model(|| {
        let wal_len = Arc::new(AtomicUsize::new(0));
        let in_checkpoint = Arc::new(AtomicBool::new(false));
        let checkpoints = Arc::new(AtomicUsize::new(0));

        let spawn_writer = || {
            let wal_len = Arc::clone(&wal_len);
            let in_checkpoint = Arc::clone(&in_checkpoint);
            let checkpoints = Arc::clone(&checkpoints);
            thread::spawn(move || {
                // Append one WAL entry, then check the threshold (2 entries).
                let len = wal_len.fetch_add(1, Ordering::AcqRel) + 1;
                if len >= 2
                    && in_checkpoint
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    // "Checkpoint": truncate the WAL, count the run.
                    wal_len.store(0, Ordering::Release);
                    checkpoints.fetch_add(1, Ordering::AcqRel);
                    in_checkpoint.store(false, Ordering::Release);
                }
            })
        };
        let a = spawn_writer();
        let b = spawn_writer();
        a.join().unwrap_or_else(|_| panic!("writer a panicked"));
        b.join().unwrap_or_else(|_| panic!("writer b panicked"));

        let runs = checkpoints.load(Ordering::Acquire);
        let pending = wal_len.load(Ordering::Acquire);
        // Exactly one checkpoint ran for the crossing, and nothing was lost:
        // either the WAL was truncated by that one run (0 or 1 late entries
        // pending) or — in the interleaving where the second writer appended
        // after the truncation — its entry is still pending for the next
        // crossing. Never two overlapping runs, never a stuck flag.
        assert_eq!(runs, 1, "threshold crossing must checkpoint exactly once");
        assert!(pending <= 1, "no acked append may vanish without a run");
        assert!(
            !in_checkpoint.load(Ordering::Acquire),
            "in-flight flag must clear"
        );
    });
}
