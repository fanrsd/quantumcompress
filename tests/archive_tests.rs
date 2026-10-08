use quantum_compress::analyzer::calculate_entropy;
use quantum_compress::archive::{create_archive, extract_archive, list_archive, test_archive};
use quantum_compress::compress::{
    apply_preprocessor, compress_block, decompress_block, undo_preprocessor,
};
use quantum_compress::format::{Algorithm, Preprocessor, Profile};
use quantum_compress::integrity::compute_blake3;
use std::fs::{self, File};
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_shannon_entropy() {
    let uniform = vec![b'A'; 1000];
    let entropy_uniform = calculate_entropy(&uniform);
    assert!(entropy_uniform < 0.01, "Uniform data should have ~0 entropy");

    let mut random = Vec::with_capacity(256 * 10);
    for _ in 0..10 {
        for b in 0..=255u8 {
            random.push(b);
        }
    }
    let entropy_random = calculate_entropy(&random);
    assert!(
        (entropy_random - 8.0).abs() < 0.05,
        "Uniform distribution across 256 bytes should have ~8.0 entropy"
    );
}

#[test]
fn test_delta_roundtrip() {
    let original: Vec<u8> = (0..5000).map(|i| (i % 256) as u8).collect();
    let delta = apply_preprocessor(&original, Preprocessor::Delta);
    let recovered = undo_preprocessor(&delta, Preprocessor::Delta);
    assert_eq!(original, recovered, "Delta transform must round-trip bit-identically");
}

#[test]
fn test_block_compression_all_algorithms() {
    let sample_text = b"QuantumCompress ultra-powerful engine. ".repeat(200);

    for algo in [Algorithm::Store, Algorithm::Zstd, Algorithm::Lzma2] {
        let (header, payload) = compress_block(
            &sample_text,
            algo,
            Preprocessor::None,
            Profile::Normal,
        )
        .expect("Compression should succeed");

        let recovered = decompress_block(&header, &payload, 0)
            .expect("Decompression and CRC32 verification should succeed");

        assert_eq!(sample_text.to_vec(), recovered);
    }
}

#[test]
fn test_archive_create_extract_verify() {
    let temp_dir = tempdir().expect("Failed to create tempdir");
    let input_dir = temp_dir.path().join("input");
    fs::create_dir_all(&input_dir).unwrap();

    // Create test files
    let file1_path = input_dir.join("file1.txt");
    let mut file1 = File::create(&file1_path).unwrap();
    let text_data = "This is a repeated string for compression testing. ".repeat(1000);
    file1.write_all(text_data.as_bytes()).unwrap();

    let sub_dir = input_dir.join("nested");
    fs::create_dir_all(&sub_dir).unwrap();
    let file2_path = sub_dir.join("binary.bin");
    let mut file2 = File::create(&file2_path).unwrap();
    let bin_data: Vec<u8> = (0..10000).map(|i| (i * 7 % 256) as u8).collect();
    file2.write_all(&bin_data).unwrap();

    let archive_path = temp_dir.path().join("test_archive.qcx");

    // 1. Create archive with verification
    create_archive(&[&input_dir], &archive_path, Profile::Normal, true)
        .expect("Archive creation must succeed");

    assert!(archive_path.exists());
    assert!(archive_path.metadata().unwrap().len() > 0);

    // 2. Test archive integrity
    test_archive(&archive_path).expect("Archive integrity test must pass");

    // 3. List archive
    list_archive(&archive_path).expect("List archive must succeed");

    // 4. Extract archive
    let extract_dir = temp_dir.path().join("extracted");
    extract_archive(&archive_path, &extract_dir).expect("Extraction must succeed");

    // 5. Verify extracted file contents and BLAKE3 hashes
    let extracted_file1 = extract_dir.join("input/file1.txt");
    let extracted_file2 = extract_dir.join("input/nested/binary.bin");

    assert!(extracted_file1.exists(), "Extracted file1 must exist");
    assert!(extracted_file2.exists(), "Extracted file2 must exist");

    let extracted_text = fs::read(&extracted_file1).unwrap();
    let extracted_bin = fs::read(&extracted_file2).unwrap();

    assert_eq!(text_data.as_bytes(), &extracted_text[..]);
    assert_eq!(&bin_data[..], &extracted_bin[..]);

    assert_eq!(compute_blake3(text_data.as_bytes()), compute_blake3(&extracted_text));
    assert_eq!(compute_blake3(&bin_data), compute_blake3(&extracted_bin));
}
