//! QuantumCompress User Interface & Progress Reporting
//!
//! Provides styled CLI progress indicators, spinners, and data size formatters.

use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

/// Create a sleek progress bar for tracking processed byte counts with speed and ETA.
pub fn create_bytes_progress_bar(total_bytes: u64, msg: &'static str) -> ProgressBar {
    let pb = ProgressBar::new(total_bytes);
    let style = ProgressStyle::default_bar()
        .template("{prefix:.bold.cyan} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta}) {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("█▓▒░ ");

    pb.set_style(style);
    pb.set_prefix(msg);
    pb
}

/// Create a fast spinner for indeterminate tasks (e.g. scanning files or computing global hashes).
pub fn create_spinner(msg: &'static str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    let style = ProgressStyle::default_spinner()
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
        .template("{prefix:.bold.green} {spinner:.cyan} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner());

    pb.set_style(style);
    pb.set_prefix(msg);
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Format byte counts into human-readable strings (e.g. "14.52 MB", "2.10 GB").
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
