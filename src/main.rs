//! QuantumCompress CLI Application
//!
//! Powerful command-line interface for the QuantumCompress engine.

use clap::{Parser, Subcommand};
use console::style;
use quantum_compress::archive::{create_archive, extract_archive, list_archive, test_archive};
use quantum_compress::error::QCResult;
use quantum_compress::format::Profile;
use std::path::PathBuf;
use std::process;
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "qc",
    author = "QuantumCompress Team",
    version = "0.1.0",
    about = "Ultra-powerful adaptive hybrid file compressor — maximum ratio & zero errors",
    long_about = "QuantumCompress (QC) is an adaptive multi-algorithm compression engine combining LZMA2, Zstandard, entropy analysis, and hardware-accelerated 4-layer data integrity verification."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compress files or directories into a .qcx archive
    #[command(alias = "c", alias = "a")]
    Compress {
        /// Files or directories to compress
        #[arg(required = true)]
        inputs: Vec<PathBuf>,

        /// Output .qcx archive path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Compression profile: turbo, fast, normal, ultra, quantum
        #[arg(short, long, default_value = "normal")]
        profile: Profile,

        /// Number of parallel threads (default: all CPU cores)
        #[arg(short, long)]
        threads: Option<usize>,

        /// Verify archive integrity immediately after creation
        #[arg(long, default_value_t = true)]
        verify: bool,
    },

    /// Extract an archive into a target directory
    #[command(alias = "x", alias = "d")]
    Extract {
        /// Path to .qcx archive
        #[arg(required = true)]
        archive: PathBuf,

        /// Target extraction directory (default: current directory)
        #[arg(short, long, default_value = ".")]
        output: PathBuf,
    },

    /// List the contents of a .qcx archive
    #[command(alias = "l")]
    List {
        /// Path to .qcx archive
        #[arg(required = true)]
        archive: PathBuf,
    },

    /// Test the integrity of a .qcx archive without extracting
    #[command(alias = "t")]
    Test {
        /// Path to .qcx archive
        #[arg(required = true)]
        archive: PathBuf,
    },

    /// Benchmark compression profiles on a given file or directory
    #[command(alias = "b")]
    Bench {
        /// Target file or directory to benchmark
        #[arg(required = true)]
        input: PathBuf,
    },

    /// Launch the QuantumCompress desktop GUI
    #[command(alias = "g")]
    Gui,
}

fn main() {
    // If double-clicked or run without arguments, try launching GUI
    if std::env::args().len() == 1 {
        if let Ok(current_exe) = std::env::current_exe() {
            let dir = current_exe.parent().unwrap_or_else(|| std::path::Path::new("."));
            let gui_exe = dir.join("qc-gui.exe");
            if gui_exe.exists() {
                if std::process::Command::new(&gui_exe).spawn().is_ok() {
                    return;
                }
            }
        }
    }

    let cli = Cli::parse();

    let result = run_command(cli);
    if let Err(e) = result {
        eprintln!("{} {}", style("Error:").bold().red(), e);
        process::exit(1);
    }
}

fn run_command(cli: Cli) -> QCResult<()> {
    match cli.command {
        Commands::Compress {
            inputs,
            output,
            profile,
            threads,
            verify,
        } => {
            if let Some(t) = threads {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(t)
                    .build_global()
                    .ok();
            }

            let output_path = match output {
                Some(p) => p,
                None => {
                    let first = &inputs[0];
                    let file_stem = first.file_stem().unwrap_or_default().to_string_lossy();
                    PathBuf::from(format!("{}.qcx", file_stem))
                }
            };

            let start = Instant::now();
            create_archive(&inputs, &output_path, profile, verify)?;
            let elapsed = start.elapsed();
            println!(
                "{} Operation completed in {:.2}s",
                style("⚡").bold().yellow(),
                elapsed.as_secs_f64()
            );
            Ok(())
        }

        Commands::Extract { archive, output } => {
            let start = Instant::now();
            extract_archive(&archive, &output)?;
            let elapsed = start.elapsed();
            println!(
                "{} Extraction completed in {:.2}s",
                style("⚡").bold().yellow(),
                elapsed.as_secs_f64()
            );
            Ok(())
        }

        Commands::List { archive } => list_archive(&archive),

        Commands::Test { archive } => {
            let start = Instant::now();
            test_archive(&archive)?;
            let elapsed = start.elapsed();
            println!(
                "{} Integrity test completed in {:.2}s",
                style("⚡").bold().yellow(),
                elapsed.as_secs_f64()
            );
            Ok(())
        }

        Commands::Bench { input } => run_bench(&input),

        Commands::Gui => {
            let current_exe = std::env::current_exe()?;
            let dir = current_exe.parent().unwrap_or_else(|| std::path::Path::new("."));
            let gui_exe = dir.join("qc-gui.exe");
            if gui_exe.exists() {
                println!("{} Meluncurkan QuantumCompress Desktop GUI...", style("🚀").bold());
                std::process::Command::new(&gui_exe).spawn()?;
            } else {
                std::process::Command::new("qc-gui").spawn()?;
            }
            Ok(())
        }
    }
}

fn run_bench(input: &PathBuf) -> QCResult<()> {
    println!(
        "{} Running benchmark on {}",
        style("🔬 Benchmark:").bold().cyan(),
        style(input.display()).bold().green()
    );

    let profiles = [
        Profile::Turbo,
        Profile::Fast,
        Profile::Normal,
        Profile::Ultra,
    ];

    println!(
        "{}",
        style("================================================================================")
            .dim()
    );
    println!(
        "{:<10}  {:<15}  {:<15}  {}",
        style("Profile").bold(),
        style("Time").bold(),
        style("Archive Size").bold(),
        style("Description").bold()
    );
    println!(
        "{}",
        style("--------------------------------------------------------------------------------")
            .dim()
    );

    let temp_dir = std::env::temp_dir();

    for profile in profiles {
        let temp_archive = temp_dir.join(format!("bench_{:?}.qcx", profile));
        let start = Instant::now();

        create_archive(&[input], &temp_archive, profile, false)?;
        let elapsed = start.elapsed();

        let archive_size = std::fs::metadata(&temp_archive)
            .map(|m| m.len())
            .unwrap_or(0);

        let _ = std::fs::remove_file(&temp_archive);

        println!(
            "{:<10}  {:<15}  {:<15}  {}",
            style(profile).bold().yellow(),
            format!("{:.2}s", elapsed.as_secs_f64()),
            quantum_compress::progress::format_bytes(archive_size),
            profile.description()
        );
    }

    println!(
        "{}",
        style("================================================================================")
            .dim()
    );
    Ok(())
}
