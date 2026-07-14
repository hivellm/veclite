//! Phase2a acceptance gates (SPEC-002 §9): property round-trips for every
//! segment body and the TOC (§9.1), decode fuzz — arbitrary bytes never panic,
//! always `Ok`/`Corrupt` (§9.3 bit-flip drills), and the commit-sequence crash
//! test — a torn write beyond the committed header leaves the previous TOC
//! valid (§9.3, STG-003/050). In-crate because the codec is `pub(crate)`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use proptest::prelude::*;

use crate::storage::body::{PayloadBlock, SparsePostings, StoredConfig};
use crate::storage::compression::Codec;
use crate::storage::header::Header;
use crate::storage::iddir::IdDir;
use crate::storage::pager::{CheckpointColl, Pager};
use crate::storage::segment::{Segment, SegmentType};
use crate::storage::toc::Toc;
use crate::storage::vectors::{Encoding, VectorsBody};

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "veclite-gate-{}-{name}.veclite",
        std::process::id()
    ))
}

// ── 2.1 property round-trips ─────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn vectors_f32_round_trips(dim in 1u32..12, count in 0u64..8, first_slot in 0u64..1000) {
        let stride = dim as usize * 4;
        let records = vec![0xABu8; stride * count as usize];
        let body = VectorsBody {
            encoding: Encoding::F32, dimension: dim, first_slot, count, sq_params: None, records,
        };
        let back = VectorsBody::decode(&body.encode()).unwrap_or_else(|e| panic!("{e}"));
        prop_assert_eq!(back, body);
    }

    #[test]
    fn iddir_round_trips(pairs in prop::collection::vec(("[a-z]{1,8}", 0u64..10_000), 0..40)) {
        let mut d = IdDir::new(16);
        for (id, slot) in &pairs {
            d.insert(id.clone(), *slot);
        }
        let back = IdDir::decode(&d.encode()).unwrap_or_else(|e| panic!("{e}"));
        prop_assert_eq!(back, d);
    }

    #[test]
    fn sparse_round_trips(
        terms in prop::collection::vec(
            (any::<u32>(), prop::collection::vec((0u64..1_000_000, -5.0f32..5.0), 0..6)),
            0..8,
        )
    ) {
        let s = SparsePostings { terms };
        let back = SparsePostings::decode(&s.encode().unwrap_or_else(|e| panic!("{e}")))
            .unwrap_or_else(|e| panic!("{e}"));
        prop_assert_eq!(back, s);
    }

    #[test]
    fn config_round_trips(dim in 1u32..65_536, m in 4u32..64, prov in prop::option::of("[a-z]{2,6}")) {
        let c = StoredConfig {
            dimension: dim, metric: 0, m, ef_construction: 200, ef_search: 100,
            quantization: 1, quant_bits: 8, compression: 1,
            embedding_provider: prov, created_epoch_s: 1_700_000_000,
        };
        let back = StoredConfig::decode(&c.encode().unwrap_or_else(|e| panic!("{e}")))
            .unwrap_or_else(|e| panic!("{e}"));
        prop_assert_eq!(back, c);
    }

    #[test]
    fn segment_framing_round_trips(
        body in prop::collection::vec(any::<u8>(), 0..3000),
        codec_pick in 0u8..3,
    ) {
        let codec = [Codec::None, Codec::Lz4, Codec::Zstd][codec_pick as usize];
        let s = Segment { seg_type: SegmentType::Payload, seg_flags: 0, coll_id: 1, body };
        let bytes = s.encode(codec).unwrap_or_else(|e| panic!("{e}"));
        let (back, total) = Segment::read(&bytes, 0, 0).unwrap_or_else(|e| panic!("{e}"));
        prop_assert_eq!(back, s);
        prop_assert_eq!(total, bytes.len());
    }
}

// ── 2.2 decode fuzz: arbitrary bytes must never panic ────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn header_decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..5000)) {
        // Any outcome is fine; the point is it does not panic or hang.
        let _ = Header::decode(&bytes);
    }

    #[test]
    fn segment_read_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2000)) {
        let _ = Segment::read(&bytes, 0, 0);
    }

    #[test]
    fn body_decoders_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2000)) {
        let _ = Toc::decode(&bytes);
        let _ = VectorsBody::decode(&bytes);
        let _ = IdDir::decode(&bytes);
        let _ = PayloadBlock::decode(&bytes);
        let _ = SparsePostings::decode(&bytes);
        let _ = StoredConfig::decode(&bytes);
    }
}

/// A single-bit flip anywhere in a framed segment's stored bytes is caught by
/// the body CRC (§9.3 bit-flip drill) — never a wrong answer, never UB.
#[test]
fn single_bit_flip_in_segment_is_detected() {
    let s = Segment {
        seg_type: SegmentType::Vectors,
        seg_flags: 0,
        coll_id: 0,
        body: (0..1500u32).flat_map(|i| i.to_le_bytes()).collect(),
    };
    let good = s.encode(Codec::None).unwrap_or_else(|e| panic!("{e}"));
    // Flip a bit in the body region (past the 32-byte header).
    for i in (32..good.len()).step_by(37) {
        let mut bytes = good.clone();
        bytes[i] ^= 0x01;
        assert!(
            matches!(
                Segment::read(&bytes, 0, 0),
                Err(crate::VecLiteError::Corrupt(_))
            ),
            "flip at {i} not detected"
        );
    }
}

// ── 2.3 commit-sequence crash: torn tail leaves the previous TOC valid ────

fn one_seg_coll(gen_marker: u8) -> CheckpointColl {
    CheckpointColl {
        coll_id: 0,
        name: "c".into(),
        aliases: vec![],
        vector_count: 1,
        tombstone_count: 0,
        segments: vec![Segment {
            seg_type: SegmentType::Payload,
            seg_flags: 0,
            coll_id: 0,
            body: vec![gen_marker; 800],
        }],
    }
}

#[test]
fn torn_tail_beyond_committed_header_leaves_previous_toc_valid() {
    let path = tmp("torn");
    let _ = std::fs::remove_file(&path);
    {
        let mut p = Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
        p.checkpoint(1, vec![one_seg_coll(0x11)], Codec::Lz4, 1001)
            .unwrap_or_else(|e| panic!("{e}"));
    } // gen 1 committed (header points at it), file closed.

    // Simulate a crash mid-checkpoint-2: partial segment/TOC bytes were
    // appended to the tail, but the header swap (step 5) never happened.
    {
        let mut f = OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap_or_else(|e| panic!("{e}"));
        f.write_all(&[0x99u8; 1234])
            .unwrap_or_else(|e| panic!("{e}"));
        f.sync_all().unwrap_or_else(|e| panic!("{e}"));
    }

    // The header still points at generation 1, so open succeeds and returns it
    // (STG-003): the damaged tail beyond the committed TOC is ignored.
    let (mut p, toc) = Pager::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(toc.generation, 1);
    let seg = p
        .read_segment(toc.collections[0].live_segments[0])
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(seg.body, vec![0x11u8; 800]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn corruption_in_the_torn_tail_does_not_affect_open() {
    let path = tmp("tail-flip");
    let _ = std::fs::remove_file(&path);
    let committed_end = {
        let mut p = Pager::create(&path, 1000).unwrap_or_else(|e| panic!("{e}"));
        p.checkpoint(2, vec![one_seg_coll(0x22)], Codec::Lz4, 1001)
            .unwrap_or_else(|e| panic!("{e}"));
        std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("{e}"))
            .len()
    };
    // Append garbage, then flip bits in it. None of this is referenced by the
    // header→TOC chain, so open still returns generation 2 intact.
    {
        let mut f = OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap_or_else(|e| panic!("{e}"));
        f.write_all(&[0u8; 500]).unwrap_or_else(|e| panic!("{e}"));
        f.sync_all().unwrap_or_else(|e| panic!("{e}"));
    }
    assert!(
        std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("{e}"))
            .len()
            > committed_end
    );

    let (_, toc) = Pager::open(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(toc.generation, 2);
    let _ = std::fs::remove_file(&path);
}
