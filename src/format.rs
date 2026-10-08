//! QCX Archive Format Definitions
//!
//! ## Binary layout
//!
//! ```text
//! ┌──────────────────────────────────────────┐
//! │ MAGIC: 4 bytes  "QCX\x01"               │
//! │ VERSION: u16 LE                          │
//! │ META_COMPRESSED_SIZE: u64 LE             │
//! │ METADATA: zstd-compressed bincode blob   │
//! │ BLOCK[0]: BlockHeader + compressed data  │
//! │ BLOCK[1]: BlockHeader + compressed data  │
//! │ …                                        │
//! │ BLOCK[N]: BlockHeader + compressed data  │
//! │ ARCHIVE_SHA256: 32 bytes                 │
//! └──────────────────────────────────────────┘
//! ```

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

// ── Magic & version ──────────────────────────────────────────────────

/// First 4 bytes of every `.qcx` file.
pub const MAGIC: [u8; 4] = [b'Q', b'C', b'X', 0x01];

/// Current format version (bump on breaking changes).
pub const FORMAT_VERSION: u16 = 1;

// ── Enums ────────────────────────────────────────────────────────────

/// Compression algorithm used for a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Algorithm {
    /// No compression — stored verbatim.
    Store,
    /// LZMA2 via liblzma (best ratio for general data).
    Lzma2,
    /// Zstandard (excellent speed-to-ratio tradeoff).
    Zstd,
}

impl std::fmt::Display for Algorithm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Algorithm::Store => write!(f, "Store"),
            Algorithm::Lzma2 => write!(f, "LZMA2"),
            Algorithm::Zstd  => write!(f, "Zstd"),
        }
    }
}

/// Optional preprocessor applied *before* compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preprocessor {
    None,
    Delta,
}

impl std::fmt::Display for Preprocessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Preprocessor::None  => write!(f, "None"),
            Preprocessor::Delta => write!(f, "Delta"),
        }
    }
}

/// Compression profile — presets that bundle algorithm, level, and block size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
pub enum Profile {
    /// 🚀 Zstd level 1, 1 MB blocks — maximum speed.
    Turbo,
    /// ⚡ Zstd level 10, 2 MB blocks — balanced speed.
    Fast,
    /// 📦 LZMA2 level 6, 4 MB blocks — balanced ratio.
    Normal,
    /// 💎 LZMA2 level 9, 8 MB blocks — maximum compression.
    Ultra,
    /// 🔬 LZMA2 level 9, 16 MB blocks — extreme compression.
    Quantum,
}

impl Profile {
    /// Block size in bytes for this profile.
    pub fn block_size(&self) -> usize {
        match self {
            Profile::Turbo   => 1 << 20,  //  1 MB
            Profile::Fast    => 2 << 20,  //  2 MB
            Profile::Normal  => 4 << 20,  //  4 MB
            Profile::Ultra   => 8 << 20,  //  8 MB
            Profile::Quantum => 16 << 20, // 16 MB
        }
    }

    /// Primary algorithm for this profile.
    pub fn default_algorithm(&self) -> Algorithm {
        match self {
            Profile::Turbo | Profile::Fast => Algorithm::Zstd,
            _ => Algorithm::Lzma2,
        }
    }

    /// LZMA2 preset level (0–9).
    pub fn lzma2_level(&self) -> u32 {
        match self {
            Profile::Turbo   => 1,
            Profile::Fast    => 3,
            Profile::Normal  => 6,
            Profile::Ultra   => 9,
            Profile::Quantum => 9,
        }
    }

    /// Zstd compression level (1–22).
    pub fn zstd_level(&self) -> i32 {
        match self {
            Profile::Turbo   => 1,
            Profile::Fast    => 10,
            Profile::Normal  => 15,
            Profile::Ultra   => 19,
            Profile::Quantum => 22,
        }
    }

    /// Human-readable one-liner.
    pub fn description(&self) -> &'static str {
        match self {
            Profile::Turbo   => "Turbo  — Zstd fastest, minimal ratio",
            Profile::Fast    => "Fast   — Zstd balanced, good ratio",
            Profile::Normal  => "Normal — LZMA2 balanced",
            Profile::Ultra   => "Ultra  — LZMA2 maximum compression",
            Profile::Quantum => "Quantum— LZMA2 extreme + large blocks",
        }
    }
}

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", match self {
            Profile::Turbo   => "Turbo",
            Profile::Fast    => "Fast",
            Profile::Normal  => "Normal",
            Profile::Ultra   => "Ultra",
            Profile::Quantum => "Quantum",
        })
    }
}

// ── Structs ──────────────────────────────────────────────────────────

/// Top-level metadata stored (compressed) at the front of the archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveMetadata {
    pub version: u16,
    pub profile: Profile,
    pub total_files: u64,
    pub total_dirs: u64,
    pub total_original_size: u64,
    pub block_count: u64,
    pub block_size: u64,
    pub files: Vec<FileEntry>,
}

/// One logical file (or directory) inside the archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative path inside the archive (forward-slash separated).
    pub path: String,
    /// Original uncompressed size in bytes (0 for directories).
    pub original_size: u64,
    /// BLAKE3 hash of the original file content.
    pub blake3_hash: [u8; 32],
    /// Index of the first block that belongs to this file.
    pub block_start: u64,
    /// Number of consecutive blocks that belong to this file.
    pub block_count: u64,
    /// `true` when this entry represents a directory (no block data).
    pub is_directory: bool,
    /// Last-modified timestamp (seconds since UNIX epoch).
    pub modified_timestamp: i64,
}

/// Fixed-size header written directly before each compressed block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockHeader {
    /// Algorithm used to compress this block.
    pub algorithm: Algorithm,
    /// Preprocessor applied before compression.
    pub preprocessor: Preprocessor,
    /// Size of the uncompressed block data.
    pub original_size: u32,
    /// Size of the compressed block data that follows this header.
    pub compressed_size: u32,
    /// CRC32 of the *uncompressed* data (fast integrity check).
    pub crc32: u32,
}
