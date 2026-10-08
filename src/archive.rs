//! QuantumCompress Archive Creation, Extraction, Testing, and Listing
//!
//! Provides the core streaming archive engine capable of processing multi-gigabyte files
//! with bounded memory usage, Rayon parallel block compression, and multi-layer verification.

use crate::analyzer::analyze_block;
use crate::compress::{compress_block, decompress_block};
use crate::error::{QCError, QCResult};
use crate::format::{
    ArchiveMetadata, BlockHeader, FileEntry, FORMAT_VERSION, MAGIC, Profile,
};
use crate::integrity::{
    compute_file_blake3, compute_file_sha256_range, to_hex,
};
use crate::progress::{create_bytes_progress_bar, create_spinner, format_bytes};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use console::style;
use rayon::prelude::*;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

/// Represents an uncompressed chunk of data with its file and block indices.
struct RawChunk {
    file_index: usize,
    block_index: u64,
    data: Vec<u8>,
}

/// Represents a compressed chunk ready to be written to disk.
struct CompressedChunk {
    _file_index: usize,
    _block_index: u64,
    header: BlockHeader,
    payload: Vec<u8>,
}

/// Compress files or directories into a `.qcx` archive.
pub fn create_archive<P: AsRef<Path>, Q: AsRef<Path>>(
    inputs: &[P],
    output_path: Q,
    profile: Profile,
    verify_after: bool,
) -> QCResult<()> {
    let output_path = output_path.as_ref();
    println!(
        "{} Creating archive {} with profile {}",
        style("[1/4]").bold().cyan(),
        style(output_path.display()).bold().green(),
        style(profile).bold().yellow()
    );

    let spinner = create_spinner("Scanning files and computing signatures...");

    // 1. Scan all input files & directories
    let mut files_to_compress: Vec<(PathBuf, String, bool, u64, [u8; 32], i64)> = Vec::new();
    let mut total_original_size: u64 = 0;
    let mut total_files: u64 = 0;
    let mut total_dirs: u64 = 0;

    for input in inputs {
        let input_path = input.as_ref();
        if !input_path.exists() {
            return Err(QCError::FileNotFound(input_path.to_path_buf()));
        }

        let base_dir = if input_path.is_dir() {
            input_path.parent().unwrap_or(input_path)
        } else {
            input_path.parent().unwrap_or(input_path)
        };

        for entry in WalkDir::new(input_path).follow_links(false) {
            let entry = entry.map_err(|e| QCError::Io(e.into()))?;
            let path = entry.path().to_path_buf();
            let metadata = fs::symlink_metadata(&path)?;

            let relative_path = path
                .strip_prefix(base_dir)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            let modified = metadata
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            if metadata.is_dir() {
                total_dirs += 1;
                files_to_compress.push((path, relative_path, true, 0, [0u8; 32], modified));
            } else if metadata.is_file() {
                let size = metadata.len();
                total_files += 1;
                total_original_size += size;
                // Compute BLAKE3 streaming hash
                let hash = compute_file_blake3(&path)?;
                files_to_compress.push((path, relative_path, false, size, hash, modified));
            }
        }
    }

    spinner.finish_and_clear();

    if files_to_compress.is_empty() {
        return Err(QCError::EmptyArchive);
    }

    println!(
        "{} Found {} files, {} directories ({})",
        style("✓").bold().green(),
        style(total_files).bold(),
        style(total_dirs).bold(),
        style(format_bytes(total_original_size)).bold().cyan()
    );

    // 2. Pre-calculate block layout
    let block_size = profile.block_size();
    let mut file_entries = Vec::new();
    let mut current_block_index: u64 = 0;

    for (_full_path, rel_path, is_dir, size, hash, modified) in &files_to_compress {
        if *is_dir {
            file_entries.push(FileEntry {
                path: rel_path.clone(),
                original_size: 0,
                blake3_hash: [0u8; 32],
                block_start: 0,
                block_count: 0,
                is_directory: true,
                modified_timestamp: *modified,
            });
        } else {
            let blocks_for_file = if *size == 0 {
                0
            } else {
                ((*size as usize + block_size - 1) / block_size) as u64
            };

            file_entries.push(FileEntry {
                path: rel_path.clone(),
                original_size: *size,
                blake3_hash: *hash,
                block_start: current_block_index,
                block_count: blocks_for_file,
                is_directory: false,
                modified_timestamp: *modified,
            });

            current_block_index += blocks_for_file;
        }
    }

    let total_blocks = current_block_index;
    let archive_metadata = ArchiveMetadata {
        version: FORMAT_VERSION,
        profile,
        total_files,
        total_dirs,
        total_original_size,
        block_count: total_blocks,
        block_size: block_size as u64,
        files: file_entries,
    };

    // Serialize and compress metadata header with Zstandard level 19
    let meta_serialized = bincode::serialize(&archive_metadata)?;
    let meta_compressed = zstd::bulk::compress(&meta_serialized, 19)
        .map_err(|e| QCError::CompressionFailed(format!("Metadata compression failed: {}", e)))?;

    // 3. Open output archive and write container header
    let out_file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(output_path)?;
    let mut writer = BufWriter::with_capacity(1024 * 1024, out_file);

    // Header layout:
    // [MAGIC: 4 bytes] [VERSION: 2 bytes] [META_COMPRESSED_LEN: 8 bytes] [META_DATA: N bytes]
    writer.write_all(&MAGIC)?;
    writer.write_u16::<LittleEndian>(FORMAT_VERSION)?;
    writer.write_u64::<LittleEndian>(meta_compressed.len() as u64)?;
    writer.write_all(&meta_compressed)?;

    println!(
        "{} Compressing {} blocks with multi-threading (Rayon)...",
        style("[2/4]").bold().cyan(),
        style(total_blocks).bold()
    );

    let progress_bar = create_bytes_progress_bar(total_original_size, "Compressing");

    // 4. Stream and compress blocks in batches to keep RAM usage bounded
    let concurrency = (num_cpus::get() * 2).max(4);
    let mut chunk_batch: Vec<RawChunk> = Vec::with_capacity(concurrency);
    let mut block_idx_counter: u64 = 0;

    for (file_idx, (full_path, _rel_path, is_dir, size, _hash, _modified)) in
        files_to_compress.iter().enumerate()
    {
        if *is_dir || *size == 0 {
            continue;
        }

        let mut file = BufReader::new(File::open(full_path)?);

        loop {
            let mut buffer = vec![0u8; block_size];
            let mut read_bytes = 0;

            while read_bytes < block_size {
                let n = file.read(&mut buffer[read_bytes..])?;
                if n == 0 {
                    break;
                }
                read_bytes += n;
            }

            if read_bytes == 0 {
                break;
            }

            buffer.truncate(read_bytes);
            chunk_batch.push(RawChunk {
                file_index: file_idx,
                block_index: block_idx_counter,
                data: buffer,
            });
            block_idx_counter += 1;

            if chunk_batch.len() >= concurrency {
                process_and_write_batch(&chunk_batch, &mut writer, profile, &progress_bar)?;
                chunk_batch.clear();
            }
        }
    }

    // Process remaining batch
    if !chunk_batch.is_empty() {
        process_and_write_batch(&chunk_batch, &mut writer, profile, &progress_bar)?;
        chunk_batch.clear();
    }

    progress_bar.finish_and_clear();
    writer.flush()?;

    // 5. Compute global SHA-256 for archive tamper detection
    let mut file_to_hash = File::open(output_path)?;
    let archive_len_before_hash = file_to_hash.metadata()?.len();
    let archive_sha256 =
        compute_file_sha256_range(&mut file_to_hash, 0, archive_len_before_hash)?;

    // Append 32-byte archive SHA-256 trailer
    let mut append_writer = OpenOptions::new().append(true).open(output_path)?;
    append_writer.write_all(&archive_sha256)?;
    append_writer.flush()?;

    let final_archive_size = output_path.metadata()?.len();
    let ratio = if total_original_size > 0 {
        (final_archive_size as f64 / total_original_size as f64) * 100.0
    } else {
        100.0
    };

    println!(
        "{} Compression complete! Original: {} ➔ Compressed: {} ({:.2}%)",
        style("✓").bold().green(),
        style(format_bytes(total_original_size)).bold(),
        style(format_bytes(final_archive_size)).bold().cyan(),
        style(ratio).bold().yellow()
    );

    // 6. Verify-after-compress (optional but highly recommended)
    if verify_after {
        println!(
            "{} Verifying archive integrity (Round-Trip Bit-Exact Check)...",
            style("[3/4]").bold().cyan()
        );
        test_archive(output_path)?;
        println!(
            "{} Verification passed: 100% bit-exact integrity guaranteed!",
            style("✓").bold().green()
        );
    }

    Ok(())
}

/// Helper function to parallel-compress a batch of chunks and write them sequentially to the archive.
fn process_and_write_batch<W: Write>(
    chunks: &[RawChunk],
    writer: &mut W,
    profile: Profile,
    progress_bar: &indicatif::ProgressBar,
) -> QCResult<()> {
    // Rayon parallel compression
    let compressed_results: Result<Vec<CompressedChunk>, QCError> = chunks
        .par_iter()
        .map(|chunk| {
            let (algo, prep) = analyze_block(&chunk.data, profile);
            let (header, payload) = compress_block(&chunk.data, algo, prep, profile)?;
            Ok(CompressedChunk {
                _file_index: chunk.file_index,
                _block_index: chunk.block_index,
                header,
                payload,
            })
        })
        .collect();

    let compressed_chunks = compressed_results?;

    // Sequential write of block headers and payloads
    for chunk in compressed_chunks {
        let header_bytes = bincode::serialize(&chunk.header)?;
        writer.write_u32::<LittleEndian>(header_bytes.len() as u32)?;
        writer.write_all(&header_bytes)?;
        writer.write_all(&chunk.payload)?;
        progress_bar.inc(chunk.header.original_size as u64);
    }

    Ok(())
}

/// Read archive metadata from an open reader.
pub fn read_archive_metadata<R: Read + Seek>(reader: &mut R) -> QCResult<ArchiveMetadata> {
    reader.seek(SeekFrom::Start(0))?;

    // Verify magic
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(QCError::InvalidArchive(
            "File signature mismatch. Not a valid QCX archive.".to_string(),
        ));
    }

    // Verify version
    let version = reader.read_u16::<LittleEndian>()?;
    if version > FORMAT_VERSION {
        return Err(QCError::InvalidArchive(format!(
            "Archive version {} is newer than supported version {}.",
            version, FORMAT_VERSION
        )));
    }

    // Read compressed metadata length
    let meta_len = reader.read_u64::<LittleEndian>()? as usize;
    let mut meta_compressed = vec![0u8; meta_len];
    reader.read_exact(&mut meta_compressed)?;

    // Decompress metadata with Zstandard
    let meta_decompressed = zstd::decode_all(&meta_compressed[..])
        .map_err(|e| QCError::InvalidArchive(format!("Corrupt archive metadata: {}", e)))?;

    let metadata: ArchiveMetadata = bincode::deserialize(&meta_decompressed)?;
    Ok(metadata)
}

/// Extract all files from a `.qcx` archive into a target directory.
pub fn extract_archive<P: AsRef<Path>, Q: AsRef<Path>>(
    archive_path: P,
    output_dir: Q,
) -> QCResult<()> {
    let archive_path = archive_path.as_ref();
    let output_dir = output_dir.as_ref();

    println!(
        "{} Extracting {} ➔ {}",
        style("[1/3]").bold().cyan(),
        style(archive_path.display()).bold().green(),
        style(output_dir.display()).bold().yellow()
    );

    let mut file = BufReader::new(File::open(archive_path)?);
    let metadata = read_archive_metadata(&mut file)?;

    println!(
        "{} Archive contains {} files, {} dirs (Original: {})",
        style("✓").bold().green(),
        style(metadata.total_files).bold(),
        style(metadata.total_dirs).bold(),
        style(format_bytes(metadata.total_original_size)).bold().cyan()
    );

    // Create target directory if it doesn't exist
    fs::create_dir_all(output_dir)?;

    // 1. Recreate directory structure
    for entry in &metadata.files {
        if entry.is_directory {
            let target_dir = output_dir.join(&entry.path);
            fs::create_dir_all(&target_dir)?;
        }
    }

    let progress_bar = create_bytes_progress_bar(metadata.total_original_size, "Extracting");

    // 2. Stream blocks and reconstruct files
    let mut current_block_idx: u64 = 0;

    for entry in &metadata.files {
        if entry.is_directory {
            continue;
        }

        let target_path = output_dir.join(&entry.path);
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut out_file = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&target_path)?,
        );

        let mut hasher = blake3::Hasher::new();

        for _ in 0..entry.block_count {
            let header_len = file.read_u32::<LittleEndian>()? as usize;
            let mut header_bytes = vec![0u8; header_len];
            file.read_exact(&mut header_bytes)?;
            let header: BlockHeader = bincode::deserialize(&header_bytes)?;

            let mut payload = vec![0u8; header.compressed_size as usize];
            file.read_exact(&mut payload)?;

            // Decompress and verify CRC32
            let raw_data = decompress_block(&header, &payload, current_block_idx)?;
            current_block_idx += 1;

            out_file.write_all(&raw_data)?;
            hasher.update(&raw_data);
            progress_bar.inc(raw_data.len() as u64);
        }

        out_file.flush()?;

        // Verify bit-exact BLAKE3 file integrity
        if entry.original_size > 0 {
            let computed_hash = *hasher.finalize().as_bytes();
            if computed_hash != entry.blake3_hash {
                return Err(QCError::IntegrityError {
                    path: entry.path.clone(),
                    expected: to_hex(&entry.blake3_hash),
                    actual: to_hex(&computed_hash),
                });
            }
        }
    }

    progress_bar.finish_and_clear();
    println!(
        "{} Extraction complete: all {} files extracted with 100% bit-exact integrity!",
        style("✓").bold().green(),
        style(metadata.total_files).bold()
    );

    Ok(())
}

/// Test archive integrity by verifying block CRC32, file BLAKE3, and archive SHA-256 without extracting.
pub fn test_archive<P: AsRef<Path>>(archive_path: P) -> QCResult<()> {
    let archive_path = archive_path.as_ref();
    let mut file = BufReader::new(File::open(archive_path)?);

    // 1. Verify archive-level SHA-256
    let total_len = file.get_ref().metadata()?.len();
    if total_len < 32 {
        return Err(QCError::InvalidArchive("Archive file is truncated".to_string()));
    }

    let payload_len = total_len - 32;
    let expected_archive_sha256 = {
        let mut seekable = File::open(archive_path)?;
        seekable.seek(SeekFrom::Start(payload_len))?;
        let mut hash = [0u8; 32];
        seekable.read_exact(&mut hash)?;
        hash
    };

    let actual_archive_sha256 =
        compute_file_sha256_range(&mut File::open(archive_path)?, 0, payload_len)?;

    if actual_archive_sha256 != expected_archive_sha256 {
        return Err(QCError::IntegrityError {
            path: "Archive Trailer SHA-256".to_string(),
            expected: to_hex(&expected_archive_sha256),
            actual: to_hex(&actual_archive_sha256),
        });
    }

    // 2. Read metadata and verify each block and file
    let metadata = read_archive_metadata(&mut file)?;
    let progress_bar = create_bytes_progress_bar(metadata.total_original_size, "Testing");

    let mut current_block_idx: u64 = 0;

    for entry in &metadata.files {
        if entry.is_directory {
            continue;
        }

        let mut hasher = blake3::Hasher::new();

        for _ in 0..entry.block_count {
            let header_len = file.read_u32::<LittleEndian>()? as usize;
            let mut header_bytes = vec![0u8; header_len];
            file.read_exact(&mut header_bytes)?;
            let header: BlockHeader = bincode::deserialize(&header_bytes)?;

            let mut payload = vec![0u8; header.compressed_size as usize];
            file.read_exact(&mut payload)?;

            // Decompress and verify CRC32
            let raw_data = decompress_block(&header, &payload, current_block_idx)?;
            current_block_idx += 1;

            hasher.update(&raw_data);
            progress_bar.inc(raw_data.len() as u64);
        }

        // Verify BLAKE3
        if entry.original_size > 0 {
            let actual_hash = *hasher.finalize().as_bytes();
            if actual_hash != entry.blake3_hash {
                return Err(QCError::IntegrityError {
                    path: entry.path.clone(),
                    expected: to_hex(&entry.blake3_hash),
                    actual: to_hex(&actual_hash),
                });
            }
        }
    }

    progress_bar.finish_and_clear();
    println!(
        "{} Archive test successful: SHA-256, CRC32, and all BLAKE3 signatures are valid!",
        style("✓").bold().green()
    );

    Ok(())
}

/// Print formatted table listing all files and directories in the archive.
pub fn list_archive<P: AsRef<Path>>(archive_path: P) -> QCResult<()> {
    let mut file = BufReader::new(File::open(archive_path)?);
    let metadata = read_archive_metadata(&mut file)?;

    println!(
        "{}",
        style("================================================================================")
            .dim()
    );
    println!(
        "{}  |  Profile: {}  |  Blocks: {} ({} each)",
        style("QuantumCompress Archive (.qcx)").bold().cyan(),
        style(metadata.profile).bold().yellow(),
        style(metadata.block_count).bold(),
        style(format_bytes(metadata.block_size)).bold()
    );
    println!(
        "{}",
        style("================================================================================")
            .dim()
    );
    println!(
        "{:<10}  {:<12}  {:<10}  {}",
        style("Type").bold(),
        style("Size").bold(),
        style("Blocks").bold(),
        style("Path").bold()
    );
    println!(
        "{}",
        style("--------------------------------------------------------------------------------")
            .dim()
    );

    for entry in &metadata.files {
        let entry_type = if entry.is_directory {
            style("DIR").blue().to_string()
        } else {
            style("FILE").green().to_string()
        };

        let size_str = if entry.is_directory {
            "-".to_string()
        } else {
            format_bytes(entry.original_size)
        };

        println!(
            "{:<10}  {:<12}  {:<10}  {}",
            entry_type,
            size_str,
            entry.block_count,
            entry.path
        );
    }

    println!(
        "{}",
        style("================================================================================")
            .dim()
    );
    println!(
        "Total: {} files, {} directories, {}",
        style(metadata.total_files).bold(),
        style(metadata.total_dirs).bold(),
        style(format_bytes(metadata.total_original_size)).bold().cyan()
    );

    Ok(())
}
