use crate::extractor::{self, ArchiveFormat};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;

/// Archive Extractor CLI - Extract archives from the command line
#[derive(Parser)]
#[command(name = "archive-extractor")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Paths to the archive files
    #[arg(value_name = "ARCHIVES")]
    pub archives: Vec<PathBuf>,

    /// Extract archives (default action)
    #[arg(short = 'x', long = "extract")]
    pub extract: bool,

    /// List contents of archives
    #[arg(short = 'l', long = "list")]
    pub list: bool,

    /// Show information about archives
    #[arg(short = 'i', long = "info")]
    pub info: bool,

    /// Destination directory (only works when extracting a single archive)
    #[arg(short, long, value_name = "DIR")]
    pub output: Option<PathBuf>,

    /// Enable verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Password for encrypted archives
    #[arg(short, long)]
    pub password: Option<String>,
}

pub fn run(cli: Cli) -> anyhow::Result<()> {
    if cli.archives.is_empty() {
        anyhow::bail!("No archive files specified. Use --help for usage.");
    }

    let action_list = cli.list;
    let action_info = cli.info;
    let action_extract = cli.extract || (!action_list && !action_info);

    if action_list {
        for archive in &cli.archives {
            run_list(archive, cli.verbose)?;
        }
    } else if action_info {
        for archive in &cli.archives {
            run_info(archive, cli.verbose)?;
        }
    } else if action_extract {
        use rayon::prelude::*;
        cli.archives.par_iter().try_for_each(|archive| {
            run_extract(
                archive,
                cli.output.as_deref(),
                cli.password.as_deref(),
                cli.verbose,
            )
        })?;
    }

    Ok(())
}

fn run_extract(
    archive: &Path,
    output: Option<&Path>,
    password: Option<&str>,
    verbose: bool,
) -> anyhow::Result<()> {
    if !archive.exists() {
        anyhow::bail!("Archive not found: {}", archive.display());
    }

    let format = ArchiveFormat::detect(archive).unwrap_or(ArchiveFormat::Unknown);
    if format == ArchiveFormat::Unknown {
        anyhow::bail!("Unknown archive format: {}", archive.display());
    }

    // Determine destination
    let dest: PathBuf = if let Some(out) = output {
        out.to_path_buf()
    } else if format.is_single_file() {
        archive
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else if let Some(stem) = archive.file_stem() {
        archive
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(stem)
    } else {
        PathBuf::from("./output")
    };

    if verbose {
        println!(
            "Extracting: {} ({})",
            archive.display(),
            format_name(format)
        );
        println!("Destination: {}", dest.display());
    }

    let progress = Arc::new(AtomicUsize::new(0));
    let total = Arc::new(AtomicUsize::new(0));
    let cancel = Arc::new(AtomicBool::new(false));

    let progress_clone = Arc::clone(&progress);
    let total_clone = Arc::clone(&total);

    let result = extractor::extract_archive(
        archive,
        &dest,
        progress_clone,
        total_clone,
        cancel,
        password,
    );

    match result {
        Ok(count) => {
            if verbose {
                println!("Extracted {} files to {}", count, dest.display());
            } else {
                println!("Extracted {} files from {}", count, archive.display());
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn run_list(archive: &Path, verbose: bool) -> anyhow::Result<()> {
    if !archive.exists() {
        anyhow::bail!("Archive not found: {}", archive.display());
    }

    let entries = extractor::list_archive(archive)?;

    if verbose {
        println!("Archive: {}", archive.display());
        println!("Total entries: {}", entries.len());
        println!();
        println!("{:<10} {:<10} Name", "Type", "Size");
        println!("{}", "-".repeat(60));
    }

    let mut total_size: u64 = 0;
    for entry in &entries {
        if verbose {
            let type_str = if entry.is_dir { "DIR" } else { "FILE" };
            let size_str = format_size(entry.size);
            println!("{:<10} {:<10} {}", type_str, size_str, entry.path.display());
        } else {
            println!("{}", entry.path.display());
        }
        if !entry.is_dir {
            total_size += entry.size;
        }
    }

    if verbose {
        println!("{}", "-".repeat(60));
        println!(
            "Total: {} files, {}",
            entries.iter().filter(|e| !e.is_dir).count(),
            format_size(total_size)
        );
    }

    Ok(())
}

fn run_info(archive: &Path, verbose: bool) -> anyhow::Result<()> {
    if !archive.exists() {
        anyhow::bail!("Archive not found: {}", archive.display());
    }

    let format = ArchiveFormat::detect(archive).unwrap_or(ArchiveFormat::Unknown);
    let entries = extractor::list_archive(archive)?;

    let total_files = entries.iter().filter(|e| !e.is_dir).count();
    let total_dirs = entries.iter().filter(|e| e.is_dir).count();
    let total_size: u64 = entries.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();

    println!("Archive: {}", archive.display());
    println!("Format: {}", format_name(format));
    println!(
        "Entries: {} ({} files, {} directories)",
        entries.len(),
        total_files,
        total_dirs
    );
    println!("Total size: {}", format_size(total_size));

    // Check for encryption
    if format == ArchiveFormat::Zip && extractor::is_zip_encrypted(archive) {
        println!("Encrypted: Yes");
    } else {
        println!("Encrypted: No");
    }

    if verbose && !entries.is_empty() {
        println!("\nContents:");
        for entry in &entries {
            let size_str = format_size(entry.size);
            let prefix = if entry.is_dir { "D" } else { " " };
            println!("  [{}] {} ({})", prefix, entry.path.display(), size_str);
        }
    }

    Ok(())
}

fn format_name(format: ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Zip => "ZIP",
        ArchiveFormat::Tar => "TAR",
        ArchiveFormat::Gzip => "GZIP",
        ArchiveFormat::Bzip2 => "BZIP2",
        ArchiveFormat::Xz => "XZ",
        ArchiveFormat::SevenZip => "7z",
        ArchiveFormat::Zstd => "ZSTD",
        ArchiveFormat::Brotli => "BROTLI",
        ArchiveFormat::Lz4 => "LZ4",
        ArchiveFormat::Rar => "RAR",
        ArchiveFormat::Unknown => "Unknown",
    }
}

fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    match size {
        s if s < KB => format!("{} B", s),
        s if s < MB => format!("{:.1} KB", s as f64 / KB as f64),
        s if s < GB => format!("{:.1} MB", s as f64 / MB as f64),
        s if s < TB => format!("{:.1} GB", s as f64 / GB as f64),
        s => format!("{:.1} TB", s as f64 / TB as f64),
    }
}
