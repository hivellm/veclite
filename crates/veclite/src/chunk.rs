//! Text chunker (SPEC-005 §7, EMB-050/051). A pure, deterministic, UTF-8-safe
//! splitter: it never splits a code point, prefers a whitespace/sentence
//! boundary near `max_chars`, and carries `overlap` bytes between chunks. No
//! file discovery, no watchers, no format conversion — a pure function of its
//! input, so bindings can expose the identical chunker.
//!
//! Adapted from `vectorizer/src/file_loader/chunker.rs` (ADR-0001): the
//! boundary-selection algorithm is kept identical; the file-path/metadata
//! shell is dropped in favor of `(text, byte_range)` chunks.

use std::ops::Range;

/// Chunking parameters (SPEC-005 §7).
#[derive(Clone, Copy, Debug)]
pub struct ChunkOptions {
    /// Target maximum chunk size in **bytes** (a boundary search may end a chunk
    /// earlier). Default 2048.
    pub max_chars: usize,
    /// Bytes of overlap carried from the end of one chunk into the next.
    /// Default 128.
    pub overlap: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        ChunkOptions {
            max_chars: 2048,
            overlap: 128,
        }
    }
}

/// One chunk: its (trimmed) text and the byte range it occupies in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    /// The chunk text (leading/trailing whitespace trimmed).
    pub text: String,
    /// The byte range of `text` within the original input.
    pub byte_range: Range<usize>,
}

/// A reusable, stateless chunker.
#[derive(Clone, Debug)]
pub struct Chunker {
    options: ChunkOptions,
}

impl Chunker {
    /// Build a chunker with the given options.
    #[must_use]
    pub fn new(options: ChunkOptions) -> Self {
        Chunker { options }
    }

    /// Split `text` into overlapping chunks. Deterministic for a given
    /// `(text, options)` (EMB-051).
    #[must_use]
    pub fn chunk(&self, text: &str) -> Vec<Chunk> {
        let max = self.options.max_chars.max(1);
        let overlap = self.options.overlap.min(max.saturating_sub(1));

        let mut chunks = Vec::new();
        let mut start = 0usize;
        while start < text.len() {
            let mut end = (start + max).min(text.len());

            // Find a good break point that is also a UTF-8 boundary.
            if end < text.len() {
                while end > start && !text.is_char_boundary(end) {
                    end -= 1;
                }
                if let Some(pos) = text[start..end].rfind(|c: char| {
                    c.is_whitespace() || c == '.' || c == '!' || c == '?' || c == '\n'
                }) {
                    end = start + pos + 1;
                    while end > start && !text.is_char_boundary(end) {
                        end -= 1;
                    }
                }
            }

            // Trim, and record the byte range of the trimmed slice.
            let raw = &text[start..end];
            let lead = raw.len() - raw.trim_start().len();
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                let tstart = start + lead;
                chunks.push(Chunk {
                    text: trimmed.to_owned(),
                    byte_range: tstart..tstart + trimmed.len(),
                });
            }

            // Advance with overlap; guarantee forward progress and a boundary.
            let mut next = if end >= overlap { end - overlap } else { end };
            if next <= start {
                next = end;
            }
            while next < text.len() && !text.is_char_boundary(next) {
                next += 1;
            }
            if next <= start {
                break; // no progress possible (end == start)
            }
            start = next;
        }
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_word_boundaries_within_max() {
        let text = "the quick brown fox jumps over the lazy dog again and again";
        let chunks = Chunker::new(ChunkOptions {
            max_chars: 20,
            overlap: 0,
        })
        .chunk(text);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.text.len() <= 20);
            // The recorded range reconstructs the chunk text exactly.
            assert_eq!(&text[c.byte_range.clone()], c.text);
            // No leading/trailing whitespace.
            assert_eq!(c.text.trim(), c.text);
        }
    }

    #[test]
    fn overlap_carries_context() {
        let text = "aaaa bbbb cccc dddd eeee ffff gggg hhhh";
        let chunks = Chunker::new(ChunkOptions {
            max_chars: 14,
            overlap: 6,
        })
        .chunk(text);
        assert!(chunks.len() >= 2);
        // Consecutive chunks overlap in the source (start of #2 < end of #1).
        assert!(chunks[1].byte_range.start < chunks[0].byte_range.end);
    }

    #[test]
    fn never_splits_a_code_point_utf8_fuzz() {
        // Multi-byte, emoji, and CJK — no panic, no split code point.
        let text = "café ☕ 日本語 テキスト 😀😀😀 emoji नमस्ते Ω≈ç√∫ end";
        for max in 1..=text.len() {
            let chunks = Chunker::new(ChunkOptions {
                max_chars: max,
                overlap: max / 3,
            })
            .chunk(text);
            for c in &chunks {
                // Slicing at these byte offsets must be valid (no panic) and equal.
                assert_eq!(&text[c.byte_range.clone()], c.text);
                assert!(text.is_char_boundary(c.byte_range.start));
                assert!(text.is_char_boundary(c.byte_range.end));
            }
        }
    }

    #[test]
    fn empty_and_whitespace_only() {
        let ch = Chunker::new(ChunkOptions::default());
        assert!(ch.chunk("").is_empty());
        assert!(ch.chunk("    \n\t  ").is_empty());
    }

    #[test]
    fn deterministic() {
        let text = "one two three four five six seven eight nine ten eleven twelve";
        let ch = Chunker::new(ChunkOptions {
            max_chars: 18,
            overlap: 4,
        });
        assert_eq!(ch.chunk(text), ch.chunk(text));
    }
}
