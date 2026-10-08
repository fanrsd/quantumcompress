//! QuantumCompress Compression & Decompression Engine
//!
//! Encapsulates block-level compression algorithms (LZMA2, Zstandard, Store),
//! preprocessing transforms (Delta), and hardware-verified decompression routines.

use crate::error::{QCError, QCResult};
use crate::format::{Algorithm, BlockHeader, Preprocessor, Profile};
use crate::integrity::{compute_crc32, verify_crc32};
use std::io::{Read, Write};

/// Apply preprocessor to raw byte slice.
pub fn apply_preprocessor(data: &[u8], preprocessor: Preprocessor) -> Vec<u8> {
    match preprocessor {
        Preprocessor::None => data.to_vec(),
        Preprocessor::Delta => {
            if data.is_empty() {
                return Vec::new();
            }
            let mut out = Vec::with_capacity(data.len());
            out.push(data[0]);
            let mut prev = data[0];
            for &curr in &data[1..] {
                out.push(curr.wrapping_sub(prev));
                prev = curr;
            }
            out
        }
    }
}

/// Reverse preprocessor transformation to recover original byte sequence.
pub fn undo_preprocessor(data: &[u8], preprocessor: Preprocessor) -> Vec<u8> {
    match preprocessor {
        Preprocessor::None => data.to_vec(),
        Preprocessor::Delta => {
            if data.is_empty() {
                return Vec::new();
            }
            let mut out = Vec::with_capacity(data.len());
            out.push(data[0]);
            let mut prev = data[0];
            for &delta in &data[1..] {
                let curr = prev.wrapping_add(delta);
                out.push(curr);
                prev = curr;
            }
            out
        }
    }
}

/// Compress byte slice with LZMA2 (via xz2 crate).
pub fn compress_lzma2(data: &[u8], level: u32) -> QCResult<Vec<u8>> {
    let mut encoder = xz2::write::XzEncoder::new(Vec::new(), level);
    encoder
        .write_all(data)
        .map_err(|e| QCError::CompressionFailed(format!("LZMA2 write failed: {}", e)))?;
    encoder
        .finish()
        .map_err(|e| QCError::CompressionFailed(format!("LZMA2 finish failed: {}", e)))
}

/// Decompress byte slice with LZMA2.
pub fn decompress_lzma2(data: &[u8]) -> QCResult<Vec<u8>> {
    let mut decoder = xz2::read::XzDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| QCError::DecompressionFailed(format!("LZMA2 decompression error: {}", e)))?;
    Ok(out)
}

/// Compress byte slice with Zstandard.
pub fn compress_zstd(data: &[u8], level: i32) -> QCResult<Vec<u8>> {
    zstd::bulk::compress(data, level)
        .map_err(|e| QCError::CompressionFailed(format!("Zstd compression failed: {}", e)))
}

/// Decompress byte slice with Zstandard.
pub fn decompress_zstd(data: &[u8], capacity: usize) -> QCResult<Vec<u8>> {
    zstd::bulk::decompress(data, capacity)
        .map_err(|e| QCError::DecompressionFailed(format!("Zstd decompression failed: {}", e)))
}

/// High-level function to compress a single chunk into a `(BlockHeader, Vec<u8>)` pair.
///
/// If compression produces a payload larger than the original uncompressed data,
/// this automatically falls back to `Algorithm::Store` to ensure no file expansion.
pub fn compress_block(
    raw_data: &[u8],
    requested_algo: Algorithm,
    requested_prep: Preprocessor,
    profile: Profile,
) -> QCResult<(BlockHeader, Vec<u8>)> {
    let original_size = raw_data.len() as u32;
    let crc32 = compute_crc32(raw_data);

    if requested_algo == Algorithm::Store || raw_data.is_empty() {
        let header = BlockHeader {
            algorithm: Algorithm::Store,
            preprocessor: Preprocessor::None,
            original_size,
            compressed_size: original_size,
            crc32,
        };
        return Ok((header, raw_data.to_vec()));
    }

    // Step 1: Preprocess
    let preprocessed = apply_preprocessor(raw_data, requested_prep);

    // Step 2: Compress
    let compressed_result = match requested_algo {
        Algorithm::Store => unreachable!(),
        Algorithm::Lzma2 => compress_lzma2(&preprocessed, profile.lzma2_level()),
        Algorithm::Zstd => compress_zstd(&preprocessed, profile.zstd_level()),
    };

    let compressed_payload = compressed_result?;

    // Check if compression was beneficial
    if compressed_payload.len() >= raw_data.len() {
        // Fallback to store
        let header = BlockHeader {
            algorithm: Algorithm::Store,
            preprocessor: Preprocessor::None,
            original_size,
            compressed_size: original_size,
            crc32,
        };
        Ok((header, raw_data.to_vec()))
    } else {
        let header = BlockHeader {
            algorithm: requested_algo,
            preprocessor: requested_prep,
            original_size,
            compressed_size: compressed_payload.len() as u32,
            crc32,
        };
        Ok((header, compressed_payload))
    }
}

/// High-level function to decompress a block payload and verify integrity.
pub fn decompress_block(
    header: &BlockHeader,
    payload: &[u8],
    block_index: u64,
) -> QCResult<Vec<u8>> {
    let uncompressed_stage = match header.algorithm {
        Algorithm::Store => payload.to_vec(),
        Algorithm::Lzma2 => decompress_lzma2(payload)?,
        Algorithm::Zstd => decompress_zstd(payload, header.original_size as usize)?,
    };

    // Step 2: Reverse preprocessor
    let raw_data = undo_preprocessor(&uncompressed_stage, header.preprocessor);

    // Step 3: Verify CRC32
    verify_crc32(&raw_data, header.crc32, block_index)?;

    Ok(raw_data)
}
