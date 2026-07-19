//! Stable replay of the committed fuzz corpus (SPEC-015 TST-050).
//!
//! Every input under `fuzz/corpus/<target>/` and `fuzz/regressions/<target>/`
//! runs through the same wrapper the cargo-fuzz binary uses — so a crash
//! found by the coverage-guided runs, once fixed and committed as a
//! regression input, is guarded forever by the normal `cargo test
//! --all-features` gate, on stable, on every platform. The seed builder runs
//! too, so the suite is meaningful even on a fresh checkout.

use std::path::PathBuf;

use veclite::fuzz_api;

fn fuzz_dir() -> PathBuf {
    // crates/veclite -> workspace root -> fuzz/
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz")
}

/// Replay every committed input for `target` from `kind` (`corpus` or
/// `regressions`); returns how many ran. A panic in any input is the failure.
fn replay(kind: &str, target: &str) -> usize {
    let dir = fuzz_dir().join(kind).join(target);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let mut ran = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(
            fuzz_api::run_target(target, &bytes),
            "unknown target {target:?}"
        );
        ran += 1;
    }
    ran
}

#[test]
fn committed_corpus_replays_clean() {
    let mut total = 0;
    for target in fuzz_api::TARGETS {
        total += replay("corpus", target);
        total += replay("regressions", target);
    }
    // The corpus is committed (seeds at minimum); an empty run means the
    // checkout is broken or the corpus moved — fail loudly, not silently.
    assert!(
        total > 0,
        "no fuzz corpus found under {} — run `cargo xtask fuzz-seed`",
        fuzz_dir().display()
    );
    println!("replayed {total} committed fuzz inputs");
}

#[test]
fn fresh_seeds_replay_clean() {
    let seeds = fuzz_api::seed_corpus().unwrap_or_else(|e| panic!("seed corpus: {e}"));
    let mut total = 0;
    for (target, inputs) in seeds {
        for bytes in inputs {
            assert!(fuzz_api::run_target(target, &bytes));
            total += 1;
        }
    }
    assert!(total > 0);
}

/// The SPEC-015 scenario, pinned directly: arbitrary bytes presented as a
/// `.veclite` file return a typed error — deterministic quick-check over
/// structured mutations of a valid image (truncations, bit flips, and length
/// -field corruption at every interesting offset).
#[test]
fn mutated_images_never_panic() {
    let seeds = fuzz_api::seed_corpus().unwrap_or_else(|e| panic!("seed corpus: {e}"));
    let image = seeds
        .iter()
        .find(|(t, _)| *t == "image")
        .and_then(|(_, inputs)| inputs.first())
        .unwrap_or_else(|| panic!("image seed missing"))
        .clone();

    // Truncation sweep.
    for cut in (0..image.len()).step_by(37) {
        fuzz_api::run_image(&image[..cut]);
    }
    // Bit-flip sweep (every 13th byte, all 8 bits on a stride).
    for at in (0..image.len()).step_by(13) {
        for bit in 0..8 {
            let mut mutated = image.clone();
            mutated[at] ^= 1 << bit;
            fuzz_api::run_image(&mutated);
        }
    }
    // Length-field corruption: stamp adversarial u64s across the header page.
    for at in (0..4096.min(image.len().saturating_sub(8))).step_by(8) {
        for value in [u64::MAX, u64::MAX / 2, 4096, 1] {
            let mut mutated = image.clone();
            mutated[at..at + 8].copy_from_slice(&value.to_le_bytes());
            fuzz_api::run_image(&mutated);
        }
    }
}
