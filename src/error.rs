//! QuantumCompress Error Types
//!
//! All errors across the crate are funneled through [`QCError`] for consistent handling.

use std::path::PathBuf;
use thiserror::Error;

/// Unified error type for all QuantumCompress operations.
#[derive(Error, Debug)]
pub enum QCError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid archive: {0}")]
    InvalidArchive(String),

    #[error("Compression failed: {0}")]
    CompressionFailed(String),

    #[error("Decompression failed: {0}")]
    DecompressionFailed(String),

    #[error("Integrity check failed for '{path}': expected {expected}, got {actual}")]
    IntegrityError {
        path: String,
        expected: String,
        actual: String,
    },

    #[error("Block CRC32 mismatch at block {block_index}: expected 0x{expected:08X}, got 0x{actual:08X}")]
    BlockCrcMismatch {
        block_index: u64,
        expected: u32,
        actual: u32,
    },

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Archive is empty — no files to compress")]
    EmptyArchive,

    #[error("{0}")]
    Other(String),
}

/// Convenience type alias used throughout the crate.
pub type QCResult<T> = Result<T, QCError>;

// ── Conversions from third-party errors ──────────────────────────────

impl From<bincode::Error> for QCError {
    fn from(e: bincode::Error) -> Self {
        QCError::SerializationError(e.to_string())
    }
}
