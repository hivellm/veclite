//! On-disk `.veclite` format v1 (SPEC-002): the 4 KiB header, the immutable
//! segment codec, the table of contents, and the root-pointer-swap commit
//! protocol. Native-only — wasm32 has no file storage (CORE-004); the module
//! is gated off wasm32 in `lib.rs`, so everything here may use `std::fs`,
//! `zstd`, and the other native-only storage dependencies freely.
//!
//! All multi-byte integers are little-endian (SPEC-002 §0). Corruption is
//! reported through [`crate::VecLiteError::Corrupt`] with a locator string
//! (`"header"`, `"toc"`, `"segment@<offset>"`), never a panic.

pub(crate) mod body;
pub(crate) mod compression;
#[cfg(test)]
mod gates;
pub(crate) mod header;
pub(crate) mod iddir;
pub(crate) mod pager;
pub(crate) mod segment;
pub(crate) mod toc;
pub(crate) mod vectors;

/// Little-endian readers over a byte slice, returning `Corrupt(ctx)` on a short
/// buffer instead of panicking (STG-010/021). Offsets are caller-checked to be
/// in range for the fixed-layout structures; these guard the variable ones.
pub(crate) mod le {
    use crate::error::{Result, VecLiteError};

    fn need(buf: &[u8], at: usize, n: usize, ctx: &str) -> Result<()> {
        if at + n > buf.len() {
            return Err(VecLiteError::Corrupt(format!(
                "{ctx}: truncated (need {n} bytes at offset {at}, have {})",
                buf.len()
            )));
        }
        Ok(())
    }

    pub(crate) fn u16(buf: &[u8], at: usize, ctx: &str) -> Result<u16> {
        need(buf, at, 2, ctx)?;
        Ok(u16::from_le_bytes([buf[at], buf[at + 1]]))
    }

    pub(crate) fn u32(buf: &[u8], at: usize, ctx: &str) -> Result<u32> {
        need(buf, at, 4, ctx)?;
        Ok(u32::from_le_bytes([
            buf[at],
            buf[at + 1],
            buf[at + 2],
            buf[at + 3],
        ]))
    }

    pub(crate) fn u64(buf: &[u8], at: usize, ctx: &str) -> Result<u64> {
        need(buf, at, 8, ctx)?;
        let mut b = [0u8; 8];
        b.copy_from_slice(&buf[at..at + 8]);
        Ok(u64::from_le_bytes(b))
    }
}
