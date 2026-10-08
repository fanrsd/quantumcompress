//! # QuantumCompress
//!
//! Ultra-powerful adaptive hybrid file compression engine.
//!
//! ## Features
//!
//! - **Multi-algorithm**: LZMA2 (best ratio) + Zstandard (best speed) + Store
//! - **Adaptive selection**: entropy-based algorithm routing per block
//! - **Parallel compression**: rayon-powered multi-threaded block processing
//! - **4-layer integrity**: CRC32 per-block → BLAKE3 per-file → SHA-256 archive
//! - **5 profiles**: Turbo → Fast → Normal → Ultra → Quantum
//! - **Verify-after-compress**: optional automatic round-trip verification

pub mod error;
pub mod format;
pub mod integrity;
pub mod compress;
pub mod analyzer;
pub mod progress;
pub mod archive;
