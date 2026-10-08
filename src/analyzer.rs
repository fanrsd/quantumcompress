//! QuantumCompress Data Analyzer & Adaptive Algorithm Selector
//!
//! Analyzes input data blocks using Shannon entropy, known file magic bytes,
//! and delta variance to dynamically choose the optimal compression algorithm
//! and preprocessing filter.

use crate::format::{Algorithm, Preprocessor, Profile};

/// Calculate Shannon entropy in bits per byte (range: 0.0 to 8.0).
///
/// An entropy near 8.0 indicates high randomness or existing compression/encryption.
/// Low entropy (< 5.0) indicates highly repetitive structured or text data.
pub fn calculate_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    let mut counts = [0u64; 256];
    for &b in data {
        counts[b as usize] += 1;
    }

    let len_f = data.len() as f64;
    let mut entropy = 0.0f64;

    for &count in &counts {
        if count > 0 {
            let p = count as f64 / len_f;
            entropy -= p * p.log2();
        }
    }

    entropy
}

/// Detect whether the initial bytes of a file/block match known compressed signatures.
pub fn is_known_compressed_format(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }

    // JPEG: FF D8 FF
    if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return true;
    }
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if data.len() >= 8 && &data[0..8] == b"\x89PNG\r\n\x1a\n" {
        return true;
    }
    // ZIP / JAR / DOCX / XLSX / APK: PK\x03\x04
    if &data[0..4] == b"PK\x03\x04" {
        return true;
    }
    // GZIP: 1F 8B
    if data[0] == 0x1F && data[1] == 0x8B {
        return true;
    }
    // 7z: 7z\xBC\xAF\x27\x1C
    if data.len() >= 6 && &data[0..6] == b"7z\xbc\xaf\x27\x1c" {
        return true;
    }
    // XZ: FD 37 7A 58 5A 00
    if data.len() >= 6 && &data[0..6] == b"\xfd7zXZ\x00" {
        return true;
    }
    // Zstandard: 28 B5 2F FD
    if &data[0..4] == [0x28, 0xb5, 0x2f, 0xfd] {
        return true;
    }
    // MP4 / MOV: check for 'ftyp'
    if data.len() >= 12 && &data[4..8] == b"ftyp" {
        return true;
    }
    // MP3: ID3
    if data.len() >= 3 && &data[0..3] == b"ID3" {
        return true;
    }

    false
}

/// Evaluates if delta preprocessing (storing differences between consecutive bytes)
/// significantly lowers the variance of the data. Useful for raw audio, uncompressed bitmaps, and sensor data.
pub fn is_delta_beneficial(data: &[u8]) -> bool {
    if data.len() < 256 {
        return false;
    }

    // Sample up to first 8KB to avoid overhead
    let sample_len = std::cmp::min(data.len(), 8192);
    let sample = &data[..sample_len];

    // Compute raw variance
    let raw_mean: f64 = sample.iter().map(|&x| x as f64).sum::<f64>() / sample_len as f64;
    let raw_variance: f64 = sample
        .iter()
        .map(|&x| {
            let diff = x as f64 - raw_mean;
            diff * diff
        })
        .sum::<f64>()
        / sample_len as f64;

    // Compute delta variance
    let mut delta_sum = 0.0f64;
    let mut prev = sample[0];
    for &curr in &sample[1..] {
        let delta = curr.wrapping_sub(prev) as i8 as f64;
        delta_sum += delta * delta;
        prev = curr;
    }
    let delta_variance = delta_sum / (sample_len - 1) as f64;

    // If delta reduces variance by at least 35%, it is strongly beneficial
    delta_variance < raw_variance * 0.65
}

/// Smart decision engine: selects the optimal compression algorithm and preprocessing filter
/// for a specific chunk of data based on user profile and data characteristics.
pub fn analyze_block(data: &[u8], profile: Profile) -> (Algorithm, Preprocessor) {
    if data.is_empty() {
        return (Algorithm::Store, Preprocessor::None);
    }

    let entropy = calculate_entropy(data);

    // If data is essentially pure noise or already heavily compressed (> 7.92 bits/byte)
    // and profile is not Quantum (which tries everything anyway), store verbatim to prevent expansion.
    if entropy > 7.92 && profile != Profile::Quantum {
        if is_known_compressed_format(data) {
            return (Algorithm::Store, Preprocessor::None);
        }
    }

    // Check preprocessor suitability
    let preprocessor = if is_delta_beneficial(data) {
        Preprocessor::Delta
    } else {
        Preprocessor::None
    };

    // Algorithm selection based on entropy and profile
    let algorithm = match profile {
        Profile::Turbo => Algorithm::Zstd,
        Profile::Fast => Algorithm::Zstd,
        Profile::Normal => {
            // For normal profile, use Zstd for high entropy (faster) and LZMA2 for lower entropy (better ratio)
            if entropy > 7.2 {
                Algorithm::Zstd
            } else {
                Algorithm::Lzma2
            }
        }
        Profile::Ultra => Algorithm::Lzma2,
        Profile::Quantum => Algorithm::Lzma2,
    };

    (algorithm, preprocessor)
}
