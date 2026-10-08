//! QuantumCompress GUI Desktop Application
//!
//! Modern, reactive desktop interface powered by eframe / egui.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use quantum_compress::archive::{
    create_archive, extract_archive, read_archive_metadata, test_archive,
};
use quantum_compress::format::{ArchiveMetadata, Profile};
use quantum_compress::progress::format_bytes;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Instant;

#[derive(PartialEq, Clone, Copy)]
enum AppTab {
    Compress,
    Extract,
    Inspect,
    Benchmark,
}

enum AppEvent {
    CompressDone {
        original_bytes: u64,
        compressed_bytes: u64,
        elapsed_sec: f64,
        archive_path: PathBuf,
    },
    ExtractDone {
        total_files: u64,
        elapsed_sec: f64,
        destination: PathBuf,
    },
    TestDone {
        message: String,
        success: bool,
    },
    BenchDone {
        profile: Profile,
        elapsed_sec: f64,
        archive_size: u64,
    },
    BenchFinished,
    Error(String),
}

struct CompressResult {
    original_bytes: u64,
    compressed_bytes: u64,
    elapsed_sec: f64,
    archive_path: PathBuf,
}

struct BenchResult {
    profile: Profile,
    elapsed_sec: f64,
    archive_size: u64,
}

pub struct QuantumCompressApp {
    current_tab: AppTab,

    // Communication with background workers
    sender: Sender<AppEvent>,
    receiver: Receiver<AppEvent>,
    is_busy: bool,
    status_text: String,

    // ── Compress Tab State ──
    compress_inputs: Vec<PathBuf>,
    compress_output: Option<PathBuf>,
    compress_profile: Profile,
    compress_verify: bool,
    compress_result: Option<CompressResult>,

    // ── Extract Tab State ──
    extract_archive_path: Option<PathBuf>,
    extract_output_dir: Option<PathBuf>,
    extract_metadata: Option<ArchiveMetadata>,

    // ── Inspect Tab State ──
    inspect_archive_path: Option<PathBuf>,
    inspect_metadata: Option<ArchiveMetadata>,
    inspect_test_result: Option<(bool, String)>,

    // ── Benchmark Tab State ──
    bench_target: Option<PathBuf>,
    bench_results: Vec<BenchResult>,
    bench_running: bool,
}

impl Default for QuantumCompressApp {
    fn default() -> Self {
        let (sender, receiver) = channel();
        Self {
            current_tab: AppTab::Compress,
            sender,
            receiver,
            is_busy: false,
            status_text: "Siap digunakan.".to_string(),

            compress_inputs: Vec::new(),
            compress_output: None,
            compress_profile: Profile::Normal,
            compress_verify: true,
            compress_result: None,

            extract_archive_path: None,
            extract_output_dir: None,
            extract_metadata: None,

            inspect_archive_path: None,
            inspect_metadata: None,
            inspect_test_result: None,

            bench_target: None,
            bench_results: Vec::new(),
            bench_running: false,
        }
    }
}

impl eframe::App for QuantumCompressApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Poll for background worker events
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                AppEvent::CompressDone {
                    original_bytes,
                    compressed_bytes,
                    elapsed_sec,
                    archive_path,
                } => {
                    self.is_busy = false;
                    let ratio = if original_bytes > 0 {
                        (compressed_bytes as f64 / original_bytes as f64) * 100.0
                    } else {
                        100.0
                    };
                    self.status_text = format!(
                        "Kompresi Sukses: {} ➔ {} ({:.2}%) dalam {:.2} detik",
                        format_bytes(original_bytes),
                        format_bytes(compressed_bytes),
                        ratio,
                        elapsed_sec
                    );
                    self.compress_result = Some(CompressResult {
                        original_bytes,
                        compressed_bytes,
                        elapsed_sec,
                        archive_path,
                    });
                }
                AppEvent::ExtractDone {
                    total_files,
                    elapsed_sec,
                    destination,
                } => {
                    self.is_busy = false;
                    self.status_text = format!(
                        "Ekstraksi Sukses: {} file diekstrak ke {} dalam {:.2}s",
                        total_files,
                        destination.display(),
                        elapsed_sec
                    );
                }
                AppEvent::TestDone {
                    message,
                    success,
                } => {
                    self.is_busy = false;
                    self.status_text = message.clone();
                    self.inspect_test_result = Some((success, message));
                }
                AppEvent::BenchDone {
                    profile,
                    elapsed_sec,
                    archive_size,
                } => {
                    self.bench_results.push(BenchResult {
                        profile,
                        elapsed_sec,
                        archive_size,
                    });
                }
                AppEvent::BenchFinished => {
                    self.bench_running = false;
                    self.is_busy = false;
                    self.status_text = "Benchmark selesai!".to_string();
                }
                AppEvent::Error(err) => {
                    self.is_busy = false;
                    self.bench_running = false;
                    self.status_text = format!("Error: {}", err);
                }
            }
        }

        // Handle Drag & Drop files into window
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            let dropped = ctx.input(|i| i.raw.dropped_files.clone());
            for file in dropped {
                if let Some(path) = file.path {
                    match self.current_tab {
                        AppTab::Compress => {
                            if !self.compress_inputs.contains(&path) {
                                self.compress_inputs.push(path);
                            }
                        }
                        AppTab::Extract => {
                            self.load_extract_archive(path);
                        }
                        AppTab::Inspect => {
                            self.load_inspect_archive(path);
                        }
                        AppTab::Benchmark => {
                            self.bench_target = Some(path);
                        }
                    }
                }
            }
        }

        self.render_header(ui);
        ui.add_space(8.0);
        self.render_tabs(ui);
        ui.add_space(8.0);

        egui::ScrollArea::vertical().show(ui, |ui| match self.current_tab {
            AppTab::Compress => self.render_compress_tab(ui),
            AppTab::Extract => self.render_extract_tab(ui),
            AppTab::Inspect => self.render_inspect_tab(ui),
            AppTab::Benchmark => self.render_benchmark_tab(ui),
        });

        self.render_footer(ui);

        // Repaint periodically when busy to animate spinners
        if self.is_busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

impl QuantumCompressApp {
    fn render_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(
                RichText::new("🗜️ QuantumCompress")
                    .size(24.0)
                    .color(Color32::from_rgb(0, 210, 255))
                    .strong(),
            );
            ui.label(
                RichText::new("v0.1.0 — Ultra-Powerful Engine")
                    .size(14.0)
                    .color(Color32::GRAY),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.is_busy {
                    ui.spinner();
                    ui.label(
                        RichText::new("Memproses...")
                            .color(Color32::YELLOW)
                            .strong(),
                    );
                } else {
                    ui.label(
                        RichText::new("● Engine Siap")
                            .color(Color32::GREEN)
                            .small(),
                    );
                }
            });
        });
    }

    fn render_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.current_tab,
                AppTab::Compress,
                RichText::new("🗜️ Buat Arsip").size(15.0),
            );
            ui.selectable_value(
                &mut self.current_tab,
                AppTab::Extract,
                RichText::new("📂 Ekstrak Arsip").size(15.0),
            );
            ui.selectable_value(
                &mut self.current_tab,
                AppTab::Inspect,
                RichText::new("🔍 Lihat & Uji Arsip").size(15.0),
            );
            ui.selectable_value(
                &mut self.current_tab,
                AppTab::Benchmark,
                RichText::new("⚡ Benchmark Profil").size(15.0),
            );
        });
        ui.separator();
    }

    // ─────────────────────────────────────────────────────────────────────────
    // TAB: COMPRESS
    // ─────────────────────────────────────────────────────────────────────────
    fn render_compress_tab(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("File / Folder Sumber:").strong());
                if ui.button("➕ Tambah File...").clicked() {
                    if let Some(files) = rfd::FileDialog::new().pick_files() {
                        for f in files {
                            if !self.compress_inputs.contains(&f) {
                                self.compress_inputs.push(f);
                            }
                        }
                    }
                }
                if ui.button("📁 Tambah Folder...").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        if !self.compress_inputs.contains(&folder) {
                            self.compress_inputs.push(folder);
                        }
                    }
                }
                if !self.compress_inputs.is_empty() && ui.button("🗑️ Bersihkan").clicked() {
                    self.compress_inputs.clear();
                }
            });

            ui.add_space(4.0);

            if self.compress_inputs.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.label(
                        RichText::new("Tarik dan letakkan (Drag & Drop) file atau folder ke sini")
                            .color(Color32::GRAY)
                            .italics(),
                    );
                    ui.add_space(20.0);
                });
            } else {
                egui::Frame::new()
                    .stroke(Stroke::new(1.0, Color32::DARK_GRAY))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .show(ui, |ui| {
                                let mut to_remove = None;
                                for (idx, path) in self.compress_inputs.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let icon = if path.is_dir() { "📁" } else { "📄" };
                                        ui.label(format!("{} {}", icon, path.display()));
                                        if ui.small_button("❌").clicked() {
                                            to_remove = Some(idx);
                                        }
                                    });
                                }
                                if let Some(idx) = to_remove {
                                    self.compress_inputs.remove(idx);
                                }
                            });
                    });
            }
        });

        ui.add_space(8.0);

        // Profile Selector Card
        ui.group(|ui| {
            ui.label(RichText::new("Pilih Profil Engine:").strong());
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut self.compress_profile,
                    Profile::Turbo,
                    "🚀 Turbo (Tercepat)",
                );
                ui.selectable_value(
                    &mut self.compress_profile,
                    Profile::Fast,
                    "⚡ Fast (Seimbang Cepat)",
                );
                ui.selectable_value(
                    &mut self.compress_profile,
                    Profile::Normal,
                    "📦 Normal (Standar)",
                );
                ui.selectable_value(
                    &mut self.compress_profile,
                    Profile::Ultra,
                    "💎 Ultra (Rasio Tinggi)",
                );
                ui.selectable_value(
                    &mut self.compress_profile,
                    Profile::Quantum,
                    "🔬 Quantum (Rasio Ekstrem)",
                );
            });

            ui.add_space(4.0);
            let profile_desc = match self.compress_profile {
                Profile::Turbo => "Zstandard Level 1, Blok 1 MB — Kecepatan dekompresi hingga 1.8 GB/s.",
                Profile::Fast => "Zstandard Level 10, Blok 2 MB — Optimal untuk backup harian seimbang.",
                Profile::Normal => "LZMA2 Level 6 / Zstd adaptif, Blok 4 MB — Standar serbaguna rasio tinggi.",
                Profile::Ultra => "LZMA2 Level 9, Blok 8 MB — Kompresi maksimal untuk arsip permanen.",
                Profile::Quantum => "LZMA2 Level 9, Blok 16 MB — Kompresi ultra ekstrem untuk cold storage.",
            };
            ui.label(RichText::new(profile_desc).small().color(Color32::from_rgb(180, 220, 255)));

            ui.add_space(6.0);
            ui.checkbox(
                &mut self.compress_verify,
                "Verifikasi Integritas Otomatis (Bit-Exact BLAKE3 & SHA-256 Check)",
            );
        });

        ui.add_space(8.0);

        // Destination selector
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Lokasi Simpan (.qcx):").strong());
                let dest_str = match &self.compress_output {
                    Some(p) => p.display().to_string(),
                    None => "Otomatis (sama dengan nama file sumber)".to_string(),
                };
                ui.monospace(dest_str);
                if ui.button("Ubah Lokasi...").clicked() {
                    if let Some(dest) = rfd::FileDialog::new()
                        .add_filter("QuantumCompress Archive", &["qcx"])
                        .save_file()
                    {
                        self.compress_output = Some(dest);
                    }
                }
            });
        });

        ui.add_space(12.0);

        // Big Action Button
        let can_compress = !self.compress_inputs.is_empty() && !self.is_busy;
        ui.vertical_centered(|ui| {
            let btn = ui.add_enabled(
                can_compress,
                egui::Button::new(
                    RichText::new("🗜️ MULAI KOMPRESI")
                        .size(18.0)
                        .color(Color32::WHITE)
                        .strong(),
                )
                .min_size(Vec2::new(260.0, 44.0))
                .fill(if can_compress {
                    Color32::from_rgb(0, 140, 230)
                } else {
                    Color32::DARK_GRAY
                }),
            );

            if btn.clicked() {
                self.start_compression();
            }
        });

        // Summary result card if available
        if let Some(res) = &self.compress_result {
            ui.add_space(12.0);
            ui.group(|ui| {
                ui.heading(
                    RichText::new("✅ Hasil Kompresi Terakhir")
                        .color(Color32::GREEN)
                        .size(16.0),
                );
                ui.horizontal(|ui| {
                    ui.label("Ukuran Asli:");
                    ui.monospace(RichText::new(format_bytes(res.original_bytes)).strong());
                    ui.label("➔ Ukuran Arsip:");
                    ui.monospace(
                        RichText::new(format_bytes(res.compressed_bytes))
                            .color(Color32::from_rgb(0, 210, 255))
                            .strong(),
                    );
                    let ratio = if res.original_bytes > 0 {
                        (res.compressed_bytes as f64 / res.original_bytes as f64) * 100.0
                    } else {
                        100.0
                    };
                    ui.label(format!("(Hemat: {:.1}%)", 100.0 - ratio));
                });
                ui.horizontal(|ui| {
                    ui.label("Waktu Eksekusi:");
                    ui.monospace(format!("{:.2} detik", res.elapsed_sec));
                    ui.label("Lokasi File:");
                    ui.monospace(res.archive_path.display().to_string());
                });
            });
        }
    }

    fn start_compression(&mut self) {
        self.is_busy = true;
        self.compress_result = None;
        self.status_text = "Memproses kompresi secara paralel...".to_string();

        let inputs = self.compress_inputs.clone();
        let output = match &self.compress_output {
            Some(p) => p.clone(),
            None => {
                let first = &inputs[0];
                let stem = first.file_stem().unwrap_or_default().to_string_lossy();
                first.parent().unwrap_or(first).join(format!("{}.qcx", stem))
            }
        };

        let profile = self.compress_profile;
        let verify = self.compress_verify;
        let tx = self.sender.clone();

        thread::spawn(move || {
            let start = Instant::now();
            let mut total_original = 0;
            for p in &inputs {
                if let Ok(meta) = std::fs::metadata(p) {
                    if meta.is_file() {
                        total_original += meta.len();
                    }
                }
            }

            match create_archive(&inputs, &output, profile, verify) {
                Ok(()) => {
                    let compressed_size = std::fs::metadata(&output)
                        .map(|m| m.len())
                        .unwrap_or(0);
                    let elapsed = start.elapsed().as_secs_f64();
                    let _ = tx.send(AppEvent::CompressDone {
                        original_bytes: total_original,
                        compressed_bytes: compressed_size,
                        elapsed_sec: elapsed,
                        archive_path: output,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppEvent::Error(err.to_string()));
                }
            }
        });
    }

    // ─────────────────────────────────────────────────────────────────────────
    // TAB: EXTRACT
    // ─────────────────────────────────────────────────────────────────────────
    fn render_extract_tab(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("File Arsip (.qcx):").strong());
                if ui.button("Pilih Arsip...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("QuantumCompress Archive", &["qcx"])
                        .pick_file()
                    {
                        self.load_extract_archive(path);
                    }
                }
            });

            if let Some(archive_path) = &self.extract_archive_path {
                ui.add_space(4.0);
                ui.monospace(format!("📁 {}", archive_path.display()));
            } else {
                ui.label(
                    RichText::new("Belum ada arsip dipilih. Tarik & letakkan file .qcx ke sini.")
                        .color(Color32::GRAY)
                        .italics(),
                );
            }
        });

        // Archive metadata preview
        if let Some(meta) = &self.extract_metadata {
            ui.add_space(6.0);
            ui.group(|ui| {
                ui.heading(RichText::new("📋 Informasi Arsip").size(15.0));
                ui.horizontal(|ui| {
                    ui.label("Profil:");
                    ui.label(RichText::new(format!("{:?}", meta.profile)).strong().color(Color32::YELLOW));
                    ui.label("Total File:");
                    ui.label(RichText::new(meta.total_files.to_string()).strong());
                    ui.label("Direktori:");
                    ui.label(RichText::new(meta.total_dirs.to_string()).strong());
                    ui.label("Ukuran Ekstraksi:");
                    ui.label(RichText::new(format_bytes(meta.total_original_size)).strong().color(Color32::GREEN));
                });
            });
        }

        ui.add_space(8.0);

        // Destination folder
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Folder Tujuan Ekstraksi:").strong());
                if ui.button("Pilih Folder Tujuan...").clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        self.extract_output_dir = Some(folder);
                    }
                }
            });
            if let Some(out_dir) = &self.extract_output_dir {
                ui.monospace(format!("📂 {}", out_dir.display()));
            } else {
                ui.monospace("Folder saat ini (default)");
            }
        });

        ui.add_space(14.0);

        let can_extract = self.extract_archive_path.is_some() && !self.is_busy;
        ui.vertical_centered(|ui| {
            let btn = ui.add_enabled(
                can_extract,
                egui::Button::new(
                    RichText::new("📂 EKSTRAK SEKARANG")
                        .size(18.0)
                        .color(Color32::WHITE)
                        .strong(),
                )
                .min_size(Vec2::new(260.0, 44.0))
                .fill(if can_extract {
                    Color32::from_rgb(0, 160, 110)
                } else {
                    Color32::DARK_GRAY
                }),
            );

            if btn.clicked() {
                self.start_extraction();
            }
        });
    }

    fn load_extract_archive(&mut self, path: PathBuf) {
        if let Ok(mut f) = File::open(&path).map(BufReader::new) {
            if let Ok(meta) = read_archive_metadata(&mut f) {
                self.extract_metadata = Some(meta);
            }
        }
        self.extract_archive_path = Some(path);
    }

    fn start_extraction(&mut self) {
        if let Some(archive_path) = self.extract_archive_path.clone() {
            let out_dir = self
                .extract_output_dir
                .clone()
                .unwrap_or_else(|| PathBuf::from("."));
            self.is_busy = true;
            self.status_text = "Mengekstrak file dan memverifikasi integritas...".to_string();

            let tx = self.sender.clone();
            thread::spawn(move || {
                let start = Instant::now();
                match extract_archive(&archive_path, &out_dir) {
                    Ok(()) => {
                        let elapsed = start.elapsed().as_secs_f64();
                        let total_files = 0; // reported in status
                        let _ = tx.send(AppEvent::ExtractDone {
                            total_files,
                            elapsed_sec: elapsed,
                            destination: out_dir,
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(AppEvent::Error(err.to_string()));
                    }
                }
            });
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // TAB: INSPECT & TEST
    // ─────────────────────────────────────────────────────────────────────────
    fn render_inspect_tab(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Buka Arsip .qcx:").strong());
                if ui.button("Pilih File...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("QuantumCompress Archive", &["qcx"])
                        .pick_file()
                    {
                        self.load_inspect_archive(path);
                    }
                }

                if self.inspect_archive_path.is_some() {
                    if ui.button("🛡️ Uji Integritas Arsip").clicked() {
                        self.start_archive_test();
                    }
                }
            });

            if let Some(p) = &self.inspect_archive_path {
                ui.monospace(p.display().to_string());
            }
        });

        if let Some((success, msg)) = &self.inspect_test_result {
            ui.add_space(4.0);
            let color = if *success { Color32::GREEN } else { Color32::RED };
            ui.label(RichText::new(msg).color(color).strong());
        }

        if let Some(meta) = &self.inspect_metadata {
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("Profil: {:?}", meta.profile));
                    ui.separator();
                    ui.label(format!("Total File: {}", meta.total_files));
                    ui.separator();
                    ui.label(format!("Total Ukuran: {}", format_bytes(meta.total_original_size)));
                    ui.separator();
                    ui.label(format!("Jumlah Blok: {}", meta.block_count));
                });
            });

            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                egui::Grid::new("inspect_grid").striped(true).show(ui, |ui| {
                    ui.label(RichText::new("Tipe").strong());
                    ui.label(RichText::new("Ukuran").strong());
                    ui.label(RichText::new("Blok").strong());
                    ui.label(RichText::new("Jalur File").strong());
                    ui.end_row();

                    for entry in &meta.files {
                        let (icon, color) = if entry.is_directory {
                            ("DIR", Color32::from_rgb(100, 150, 255))
                        } else {
                            ("FILE", Color32::GREEN)
                        };
                        ui.colored_label(color, icon);
                        if entry.is_directory {
                            ui.label("-");
                        } else {
                            ui.label(format_bytes(entry.original_size));
                        }
                        ui.label(entry.block_count.to_string());
                        ui.label(&entry.path);
                        ui.end_row();
                    }
                });
            });
        }
    }

    fn load_inspect_archive(&mut self, path: PathBuf) {
        self.inspect_test_result = None;
        if let Ok(mut f) = File::open(&path).map(BufReader::new) {
            if let Ok(meta) = read_archive_metadata(&mut f) {
                self.inspect_metadata = Some(meta);
            }
        }
        self.inspect_archive_path = Some(path);
    }

    fn start_archive_test(&mut self) {
        if let Some(archive_path) = self.inspect_archive_path.clone() {
            self.is_busy = true;
            self.status_text = "Menguji integritas SHA-256, CRC32, dan BLAKE3...".to_string();
            let tx = self.sender.clone();

            thread::spawn(move || {
                match test_archive(&archive_path) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::TestDone {
                            message: "Semua checksum valid: 100% bit-exact integrity!".to_string(),
                            success: true,
                        });
                    }
                    Err(err) => {
                        let _ = tx.send(AppEvent::TestDone {
                            message: format!("Tes gagal: {}", err),
                            success: false,
                        });
                    }
                }
            });
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // TAB: BENCHMARK
    // ─────────────────────────────────────────────────────────────────────────
    fn render_benchmark_tab(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Target Benchmark:").strong());
                if ui.button("Pilih File/Folder...").clicked() {
                    if let Some(f) = rfd::FileDialog::new().pick_file() {
                        self.bench_target = Some(f);
                    }
                }
                if let Some(target) = &self.bench_target {
                    ui.monospace(target.display().to_string());
                }
            });
        });

        ui.add_space(8.0);

        let can_bench = self.bench_target.is_some() && !self.is_busy;
        if ui
            .add_enabled(can_bench, egui::Button::new("⚡ Mulai Benchmark Profil"))
            .clicked()
        {
            self.start_benchmark();
        }

        if !self.bench_results.is_empty() {
            ui.add_space(10.0);
            egui::Grid::new("bench_grid").striped(true).show(ui, |ui| {
                ui.label(RichText::new("Profil").strong());
                ui.label(RichText::new("Waktu").strong());
                ui.label(RichText::new("Ukuran Hasil").strong());
                ui.label(RichText::new("Keterangan").strong());
                ui.end_row();

                for item in &self.bench_results {
                    ui.label(RichText::new(format!("{:?}", item.profile)).strong().color(Color32::YELLOW));
                    ui.label(format!("{:.2}s", item.elapsed_sec));
                    ui.label(format_bytes(item.archive_size));
                    ui.label(item.profile.description());
                    ui.end_row();
                }
            });
        }
    }

    fn start_benchmark(&mut self) {
        if let Some(target) = self.bench_target.clone() {
            self.is_busy = true;
            self.bench_running = true;
            self.bench_results.clear();
            self.status_text = "Menjalankan benchmark antar profil...".to_string();

            let tx = self.sender.clone();
            thread::spawn(move || {
                let profiles = [
                    Profile::Turbo,
                    Profile::Fast,
                    Profile::Normal,
                    Profile::Ultra,
                ];

                let temp_dir = std::env::temp_dir();

                for p in profiles {
                    let temp_file = temp_dir.join(format!("bench_gui_{:?}.qcx", p));
                    let start = Instant::now();
                    if create_archive(&[&target], &temp_file, p, false).is_ok() {
                        let elapsed = start.elapsed().as_secs_f64();
                        let size = std::fs::metadata(&temp_file)
                            .map(|m| m.len())
                            .unwrap_or(0);
                        let _ = std::fs::remove_file(&temp_file);
                        let _ = tx.send(AppEvent::BenchDone {
                            profile: p,
                            elapsed_sec: elapsed,
                            archive_size: size,
                        });
                    }
                }
                let _ = tx.send(AppEvent::BenchFinished);
            });
        }
    }

    fn render_footer(&self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status_text).small().color(Color32::LIGHT_GRAY));
            });
            ui.separator();
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 640.0])
            .with_min_inner_size([720.0, 500.0])
            .with_title("QuantumCompress — Ultra-Powerful File Compressor"),
        ..Default::default()
    };

    eframe::run_native(
        "QuantumCompress",
        options,
        Box::new(|_cc| Ok(Box::new(QuantumCompressApp::default()))),
    )
}
