//! The fixed 4 KiB file header (SPEC-002 §2). Offset 0, always exactly 4096
//! bytes, little-endian. The header is the root pointer: it names the current
//! TOC, and rewriting it (a single 4 KiB write + fsync) is the atomic commit
//! point (STG-011, §5).

use crate::error::{Result, VecLiteError};
use crate::storage::le;

/// `VECL` — the file magic at offset 0.
pub(crate) const MAGIC: [u8; 4] = *b"VECL";
/// The only format version this build writes and reads (SPEC-002 §2).
pub(crate) const FORMAT_VERSION: u32 = 1;
/// Highest `min_reader_version` this build can read. A file demanding more
/// fails with `UnsupportedFormatVersion` (STG-010).
pub(crate) const SUPPORTED_READER_VERSION: u32 = 1;
/// The header occupies a full 4 KiB page.
pub(crate) const HEADER_SIZE: usize = 4096;
/// `flags` bit 0: the database was closed cleanly (no WAL replay needed).
pub(crate) const FLAG_CLEAN_CLOSE: u64 = 1 << 0;

/// Byte range covered by `header_crc32`: `[0,12)` and `[16,256)` — everything
/// in the first 256 bytes except the CRC field itself (SPEC-002 §2).
const CRC_HEAD_END: usize = 12;
const CRC_FIELD_END: usize = 16;
const CRC_COVER_END: usize = 256;

/// Parsed 4 KiB header. The reserved tail (256..4096) is not represented; it is
/// written as zeros and ignored on read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub(crate) format_version: u32,
    pub(crate) min_reader_version: u32,
    pub(crate) flags: u64,
    pub(crate) toc_offset: u64,
    pub(crate) toc_len: u64,
    pub(crate) toc_crc32: u32,
    pub(crate) file_uuid: [u8; 16],
    pub(crate) created_epoch_s: u64,
    pub(crate) modified_epoch_s: u64,
}

impl Header {
    /// A fresh header for a new file: version 1, no TOC yet, clean flag unset.
    pub(crate) fn new(file_uuid: [u8; 16], created_epoch_s: u64) -> Self {
        Header {
            format_version: FORMAT_VERSION,
            min_reader_version: SUPPORTED_READER_VERSION,
            flags: 0,
            toc_offset: 0,
            toc_len: 0,
            toc_crc32: 0,
            file_uuid,
            created_epoch_s,
            modified_epoch_s: created_epoch_s,
        }
    }

    /// Serialize to the fixed 4 KiB page, computing and embedding the CRC.
    pub(crate) fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut b = [0u8; HEADER_SIZE];
        b[0..4].copy_from_slice(&MAGIC);
        b[4..8].copy_from_slice(&self.format_version.to_le_bytes());
        b[8..12].copy_from_slice(&self.min_reader_version.to_le_bytes());
        // 12..16 is the CRC, filled in last.
        b[16..24].copy_from_slice(&self.flags.to_le_bytes());
        b[24..32].copy_from_slice(&self.toc_offset.to_le_bytes());
        b[32..40].copy_from_slice(&self.toc_len.to_le_bytes());
        b[40..44].copy_from_slice(&self.toc_crc32.to_le_bytes());
        // 44..48 reserved (0)
        b[48..64].copy_from_slice(&self.file_uuid);
        b[64..72].copy_from_slice(&self.created_epoch_s.to_le_bytes());
        b[72..80].copy_from_slice(&self.modified_epoch_s.to_le_bytes());
        let crc = header_crc(&b);
        b[12..16].copy_from_slice(&crc.to_le_bytes());
        b
    }

    /// Parse and validate a header. Order per STG-010: magic, CRC,
    /// `min_reader_version`. A malformed header is `Corrupt("header")`; a
    /// too-new one is `UnsupportedFormatVersion`.
    pub(crate) fn decode(buf: &[u8]) -> Result<Header> {
        if buf.len() < HEADER_SIZE {
            return Err(corrupt("header too short"));
        }
        if buf[0..4] != MAGIC {
            return Err(corrupt("bad magic"));
        }
        let stored_crc = le::u32(buf, 12, "header")?;
        if header_crc(buf) != stored_crc {
            return Err(corrupt("crc mismatch"));
        }
        let min_reader_version = le::u32(buf, 8, "header")?;
        if min_reader_version > SUPPORTED_READER_VERSION {
            return Err(VecLiteError::UnsupportedFormatVersion {
                found: min_reader_version,
                supported: SUPPORTED_READER_VERSION,
            });
        }
        let mut file_uuid = [0u8; 16];
        file_uuid.copy_from_slice(&buf[48..64]);
        Ok(Header {
            format_version: le::u32(buf, 4, "header")?,
            min_reader_version,
            flags: le::u64(buf, 16, "header")?,
            toc_offset: le::u64(buf, 24, "header")?,
            toc_len: le::u64(buf, 32, "header")?,
            toc_crc32: le::u32(buf, 40, "header")?,
            file_uuid,
            created_epoch_s: le::u64(buf, 64, "header")?,
            modified_epoch_s: le::u64(buf, 72, "header")?,
        })
    }

    pub(crate) fn clean_close(&self) -> bool {
        self.flags & FLAG_CLEAN_CLOSE != 0
    }
}

fn corrupt(what: &str) -> VecLiteError {
    VecLiteError::Corrupt(format!("header: {what}"))
}

/// CRC32 over `[0,12) ∪ [16,256)` (the CRC field is excluded).
fn header_crc(buf: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(&buf[0..CRC_HEAD_END]);
    h.update(&buf[CRC_FIELD_END..CRC_COVER_END]);
    h.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Header {
        let mut h = Header::new([7u8; 16], 1_700_000_000);
        h.flags = FLAG_CLEAN_CLOSE;
        h.toc_offset = 4096;
        h.toc_len = 321;
        h.toc_crc32 = 0xDEAD_BEEF;
        h.modified_epoch_s = 1_700_000_500;
        h
    }

    #[test]
    fn round_trip() {
        let h = sample();
        let bytes = h.encode();
        assert_eq!(bytes.len(), HEADER_SIZE);
        let back = Header::decode(&bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, h);
        assert!(back.clean_close());
        // Reserved tail is zero.
        assert!(bytes[256..].iter().all(|&x| x == 0));
    }

    #[test]
    fn bad_magic_is_corrupt() {
        let mut bytes = sample().encode();
        bytes[1] = b'X';
        let Err(VecLiteError::Corrupt(m)) = Header::decode(&bytes) else {
            panic!("expected Corrupt")
        };
        assert!(m.contains("header"));
    }

    #[test]
    fn any_bit_flip_in_covered_region_fails_crc() {
        // Flip one bit in each covered byte; every flip must be caught.
        let good = sample().encode();
        for i in (0..12).chain(16..256) {
            let mut bytes = good;
            bytes[i] ^= 0x01;
            assert!(
                matches!(Header::decode(&bytes), Err(VecLiteError::Corrupt(_))),
                "bit flip at {i} not detected"
            );
        }
    }

    #[test]
    fn too_new_reader_version_is_unsupported() {
        let mut h = sample();
        h.min_reader_version = SUPPORTED_READER_VERSION + 1;
        let bytes = h.encode();
        assert!(matches!(
            Header::decode(&bytes),
            Err(VecLiteError::UnsupportedFormatVersion { .. })
        ));
    }

    #[test]
    fn truncated_header_is_corrupt() {
        let bytes = sample().encode();
        assert!(matches!(
            Header::decode(&bytes[..100]),
            Err(VecLiteError::Corrupt(_))
        ));
    }
}
