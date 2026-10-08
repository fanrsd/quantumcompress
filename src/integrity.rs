//! QuantumCompress Multi-Layer Integrity System
//!
//! Provides cryptographic and non-cryptographic checksums:
//! - **Block Level**: Fast hardware-accelerated CRC32 for early block corruption detection.
//! - **File Level**: BLAKE3 256-bit hash for bit-exact verification of reconstructed files.
//! - **Archive Level**: SHA-256 hash for global archive tamper detection.

use crate::error::{QCError, QCResult};
use crc32fast::Hasher as Crc32Hasher;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Compute hardware-accelerated CRC32 for a byte slice.
#[inline]
pub fn compute_crc32(data: &[u8]) -> u32 {
    let mut hasher = Crc32Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Verify CRC32 matches expected value; returns [`QCError::BlockCrcMismatch`] on failure.
#[inline]
pub fn verify_crc32(data: &[u8], expected: u32, block_index: u64) -> QCResult<()> {
    let actual = compute_crc32(data);
    if actual != expected {
        Err(QCError::BlockCrcMismatch {
            block_index,
            expected,
            actual,
        })
    } else {
        Ok(())
    }
}

/// Compute BLAKE3 256-bit hash for an in-memory byte slice.
#[inline]
pub fn compute_blake3(data: &[u8]) -> [u8; 32] {
    let hash = blake3::hash(data);
    *hash.as_bytes()
}

/// Compute BLAKE3 hash by streaming a file from disk (handles multi-GB files with minimal RAM).
pub fn compute_file_blake3<P: AsRef<Path>>(path: P) -> QCResult<[u8; 32]> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 65536]; // 64 KB buffer

    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(*hasher.finalize().as_bytes())
}

/// Verify that data matches the expected BLAKE3 hash.
pub fn verify_blake3(data: &[u8], expected: &[u8; 32], path_name: &str) -> QCResult<()> {
    let actual = compute_blake3(data);
    if actual != *expected {
        Err(QCError::IntegrityError {
            path: path_name.to_string(),
            expected: to_hex(expected),
            actual: to_hex(&actual),
        })
    } else {
        Ok(())
    }
}

/// Compute SHA-256 for a byte slice.
pub fn compute_sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

/// Compute SHA-256 of an entire file excluding a trailing number of bytes (useful for archive trailer).
pub fn compute_file_sha256_range<R: Read + Seek>(
    reader: &mut R,
    start_pos: u64,
    length: u64,
) -> QCResult<[u8; 32]> {
    reader.seek(SeekFrom::Start(start_pos))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut remaining = length;

    while remaining > 0 {
        let to_read = std::cmp::min(remaining, buffer.len() as u64) as usize;
        let n = reader.read(&mut buffer[..to_read])?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        remaining -= n as u64;
    }

    Ok(hasher.finalize().into())
}

/// Helper to format byte slices into lowercase hex strings.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
