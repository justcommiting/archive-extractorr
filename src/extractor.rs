use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tar::Archive;
use unrar::Archive as RarArchive;
use xz2::read::XzDecoder;
use zip::ZipArchive;

/// Archive entry information
#[derive(Clone, Debug)]
pub struct ArchiveEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    #[allow(dead_code)]
    pub compressed_size: u64,
    pub path: std::path::PathBuf,
}

/// Check if a ZIP archive is password protected
pub fn is_zip_encrypted(path: &Path) -> bool {
    if let Ok(file) = File::open(path) {
        if let Ok(mut archive) = ZipArchive::new(file) {
            for i in 0..archive.len() {
                // Try to get the file - if it fails with UnsupportedArchive, it's encrypted
                match archive.by_index(i) {
                    Ok(_) => continue,
                    Err(zip::result::ZipError::UnsupportedArchive(_)) => return true,
                    Err(_) => continue,
                }
            }
        }
    }
    false
}

/// Check if a RAR archive is password protected
pub fn is_rar_encrypted(path: &Path) -> bool {
    use unrar::Archive as RarArchive;
    let path_str = path.to_string_lossy().to_string();
    let archive = RarArchive::new(&path_str);
    if let Ok(mut list_archive) = archive.open_for_listing() {
        while let Some(result) = list_archive.by_ref().next() {
            if let Ok(_entry) = result {
                // Check if any entry is encrypted
                // unrar doesn't directly expose encryption status in list mode
                // so we'll need to try extraction and see if it fails
            }
        }
    }
    // For RAR, we'll detect encryption during extraction
    false
}

/// Supported archive formats
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    Gzip,
    Bzip2,
    Xz,
    Rar,
    Unknown,
}

impl ArchiveFormat {
    pub fn from_extension(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "zip" => ArchiveFormat::Zip,
            "tar" => ArchiveFormat::Tar,
            "gz" | "gzip" => ArchiveFormat::Gzip,
            "bz2" | "bzip2" => ArchiveFormat::Bzip2,
            "xz" | "lzma" => ArchiveFormat::Xz,
            "rar" => ArchiveFormat::Rar,
            _ => ArchiveFormat::Unknown,
        }
    }

    pub fn from_magic_bytes(data: &[u8]) -> Option<ArchiveFormat> {
        // ZIP: 50 4B 03 04
        if data.len() >= 4 && data[0..4] == [0x50, 0x4B, 0x03, 0x04] {
            return Some(ArchiveFormat::Zip);
        }
        // RAR: 52 61 72 21 1A 07 00 (v1.5) or 52 61 72 21 1A 07 01 00 (v2.0)
        if data.len() >= 7 && data[0..7] == [0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00] {
            return Some(ArchiveFormat::Rar);
        }
        if data.len() >= 8 && data[0..8] == [0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00] {
            return Some(ArchiveFormat::Rar);
        }
        // GZIP: 1F 8B
        if data.len() >= 2 && data[0..2] == [0x1F, 0x8B] {
            return Some(ArchiveFormat::Gzip);
        }
        // BZIP2: 42 5A 68
        if data.len() >= 3 && data[0..3] == [0x42, 0x5A, 0x68] {
            return Some(ArchiveFormat::Bzip2);
        }
        // XZ: FD 37 7A 58 5A 00
        if data.len() >= 6 && data[0..6] == [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00] {
            return Some(ArchiveFormat::Xz);
        }
        // TAR: ustar at offset 257
        if data.len() >= 262 && data[257..262] == [0x75, 0x73, 0x74, 0x61, 0x72] {
            return Some(ArchiveFormat::Tar);
        }
        None
    }

    pub fn detect(path: &Path) -> Option<ArchiveFormat> {
        // Try magic bytes first
        if let Ok(mut file) = File::open(path) {
            let mut buffer = [0u8; 512];
            if file.read_exact(&mut buffer).is_ok() {
                if let Some(format) = Self::from_magic_bytes(&buffer) {
                    return Some(format);
                }
            }
        }
        // Fall back to extension
        let format = Self::from_extension(path);
        if format != ArchiveFormat::Unknown {
            Some(format)
        } else {
            None
        }
    }
}

/// Extract a ZIP archive
pub fn extract_zip(
    path: &Path,
    dest: &Path,
    _progress: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel_flag: Arc<AtomicBool>,
    password: Option<&str>,
) -> Result<usize> {
    let file = File::open(path).context("Failed to open ZIP file")?;
    let mut archive = ZipArchive::new(file).context("Invalid ZIP archive")?;
    let total_files = archive.len();
    total.store(total_files, Ordering::Relaxed);

    let mut extracted = 0;

    for i in 0..archive.len() {
        if cancel_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Extraction cancelled");
        }

        // Try to get the entry, using password if needed
        let mut entry = if let Some(pwd) = password {
            // Use password for all entries - by_index_decrypt handles both encrypted and plain
            match archive.by_index_decrypt(i, pwd.as_bytes()) {
                Ok(result) => result.map_err(|_| anyhow::anyhow!("Invalid password"))?,
                Err(e) => return Err(e).context("Failed to decrypt entry"),
            }
        } else {
            // No password - try regular access
            match archive.by_index(i) {
                Ok(e) => e,
                Err(zip::result::ZipError::UnsupportedArchive(_)) => {
                    anyhow::bail!("Archive contains encrypted entries, please provide a password");
                }
                Err(e) => return Err(e).context("Failed to read entry"),
            }
        };

        let entry_path = entry.enclosed_name().unwrap_or_else(|| Path::new(entry.name()));
        let out_path = dest.join(entry_path);

        if entry.name().ends_with('/') {
            fs::create_dir_all(&out_path).context("Failed to create directory")?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).context("Failed to create parent directory")?;
            }
            let mut outfile = File::create(&out_path).context("Failed to create output file")?;
            io::copy(&mut entry, &mut outfile).context("Failed to extract file")?;
        }

        extracted += 1;
    }

    Ok(extracted)
}

/// Extract a TAR archive
pub fn extract_tar(
    path: &Path,
    dest: &Path,
    _progress: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel_flag: Arc<AtomicBool>,
    _password: Option<&str>,
) -> Result<usize> {
    let file = File::open(path).context("Failed to open TAR file")?;
    let mut archive = Archive::new(file);

    // Count entries first
    let entries: Vec<_> = archive
        .entries()
        .context("Failed to read archive entries")?
        .filter_map(|e| e.ok())
        .collect();
    total.store(entries.len(), Ordering::Relaxed);

    let mut extracted = 0;

    for entry in archive.entries().context("Failed to read entries")? {
        if cancel_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Extraction cancelled");
        }

        let mut entry = entry.context("Failed to read entry")?;
        entry.unpack_in(dest).context("Failed to extract entry")?;
        extracted += 1;
    }

    Ok(extracted)
}

/// Extract a GZIP file
pub fn extract_gzip(
    path: &Path,
    dest: &Path,
    _progress: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel_flag: Arc<AtomicBool>,
    _password: Option<&str>,
) -> Result<usize> {
    if cancel_flag.load(Ordering::Relaxed) {
        anyhow::bail!("Extraction cancelled");
    }

    let file = File::open(path).context("Failed to open GZIP file")?;
    let mut decoder = GzDecoder::new(file);

    let mut output_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    if output_name.ends_with(".gz") {
        output_name = output_name[..output_name.len() - 3].to_string();
    }
    if output_name.is_empty() {
        output_name = "output".to_string();
    }

    let out_path = dest.join(&output_name);
    let mut outfile = File::create(&out_path).context("Failed to create output file")?;

    let mut buffer = Vec::new();
    decoder.read_to_end(&mut buffer).context("Failed to decompress")?;
    outfile.write_all(&buffer).context("Failed to write output")?;

    total.store(1, Ordering::Relaxed);

    Ok(1)
}

/// Extract a BZIP2 file
pub fn extract_bzip2(
    path: &Path,
    dest: &Path,
    _progress: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel_flag: Arc<AtomicBool>,
    _password: Option<&str>,
) -> Result<usize> {
    if cancel_flag.load(Ordering::Relaxed) {
        anyhow::bail!("Extraction cancelled");
    }

    let file = File::open(path).context("Failed to open BZIP2 file")?;
    let mut decoder = bzip2::read::BzDecoder::new(file);

    let mut output_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    if output_name.ends_with(".bz2") {
        output_name = output_name[..output_name.len() - 4].to_string();
    }
    if output_name.is_empty() {
        output_name = "output".to_string();
    }

    let out_path = dest.join(&output_name);
    let mut outfile = File::create(&out_path).context("Failed to create output file")?;

    let mut buffer = Vec::new();
    decoder.read_to_end(&mut buffer).context("Failed to decompress")?;
    outfile.write_all(&buffer).context("Failed to write output")?;

    total.store(1, Ordering::Relaxed);

    Ok(1)
}

/// Extract an XZ file
pub fn extract_xz(
    path: &Path,
    dest: &Path,
    _progress: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel_flag: Arc<AtomicBool>,
    _password: Option<&str>,
) -> Result<usize> {
    if cancel_flag.load(Ordering::Relaxed) {
        anyhow::bail!("Extraction cancelled");
    }

    let file = File::open(path).context("Failed to open XZ file")?;
    let mut decoder = XzDecoder::new(file);

    let mut output_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    if output_name.ends_with(".xz") {
        output_name = output_name[..output_name.len() - 3].to_string();
    }
    if output_name.is_empty() {
        output_name = "output".to_string();
    }

    let out_path = dest.join(&output_name);
    let mut outfile = File::create(&out_path).context("Failed to create output file")?;

    let mut buffer = Vec::new();
    decoder.read_to_end(&mut buffer).context("Failed to decompress")?;
    outfile.write_all(&buffer).context("Failed to write output")?;

    total.store(1, Ordering::Relaxed);

    Ok(1)
}

/// Extract a RAR archive
pub fn extract_rar(
    path: &Path,
    dest: &Path,
    progress: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel_flag: Arc<AtomicBool>,
    password: Option<&str>,
) -> Result<usize> {
    let path_str = path.to_string_lossy().to_string();

    // Open for listing first to count entries
    let archive = RarArchive::new(&path_str);
    let mut list_archive = archive.open_for_listing().context("Failed to open RAR archive")?;
    let entries: Vec<_> = list_archive.by_ref().filter_map(|e| e.ok()).collect();
    total.store(entries.len(), Ordering::Relaxed);

    // Now open for processing/extraction (with password if provided)
    let archive = if let Some(pwd) = password {
        RarArchive::with_password(&path_str, pwd)
    } else {
        RarArchive::new(&path_str)
    };

    let mut process_archive = archive.open_for_processing().context("Failed to open RAR for processing")?;

    let mut extracted = 0;

    loop {
        if cancel_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Extraction cancelled");
        }

        // Read next header - returns OpenArchive<Process, CursorBeforeFile>
        let archive_with_header = match process_archive.read_header() {
            Ok(Some(h)) => h,
            Ok(None) => break, // End of archive
            Err(e) => return Err(e).context("Failed to read RAR header"),
        };

        let header = archive_with_header.entry();
        let entry_path = header.filename.clone();
        let out_path = dest.join(&entry_path);

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).context("Failed to create directory")?;
        }

        if header.is_directory() {
            fs::create_dir_all(&out_path).ok();
        } else {
            // Extract the file using extract_with_base
            process_archive = archive_with_header
                .extract_with_base(dest)
                .context("Failed to extract file")?;
            extracted += 1;
            progress.fetch_add(1, Ordering::Relaxed);
            continue;
        }

        // Skip directory entries
        process_archive = archive_with_header.skip().context("Failed to skip entry")?;
    }

    Ok(extracted)
}

/// List entries in an archive without extracting
pub fn list_archive(path: &Path) -> Result<Vec<ArchiveEntry>> {
    let format = ArchiveFormat::detect(path).context("Unknown archive format")?;

    match format {
        ArchiveFormat::Zip => {
            let file = File::open(path)?;
            let mut archive = ZipArchive::new(file)?;
            let mut entries = Vec::new();

            for i in 0..archive.len() {
                let entry = archive.by_index(i)?;
                entries.push(ArchiveEntry {
                    name: entry.name().to_string(),
                    is_dir: entry.name().ends_with('/'),
                    size: entry.size(),
                    compressed_size: entry.compressed_size(),
                    path: entry.enclosed_name().unwrap_or_else(|| Path::new(entry.name())).to_path_buf(),
                });
            }
            Ok(entries)
        }
        ArchiveFormat::Tar => {
            let file = File::open(path)?;
            let mut archive = Archive::new(file);
            let mut entries = Vec::new();

            for entry in archive.entries()? {
                let entry = entry?;
                let path = entry.path()?.to_path_buf();
                let header = entry.header();
                entries.push(ArchiveEntry {
                    name: path.to_string_lossy().to_string(),
                    is_dir: entry.path()?.ends_with("/"),
                    size: header.size().unwrap_or(0),
                    compressed_size: 0,
                    path,
                });
            }
            Ok(entries)
        }
        ArchiveFormat::Gzip | ArchiveFormat::Bzip2 | ArchiveFormat::Xz => {
            let file = File::open(path)?;
            let metadata = file.metadata()?;
            Ok(vec![ArchiveEntry {
                name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
                is_dir: false,
                size: metadata.len(),
                compressed_size: metadata.len(),
                path: path.to_path_buf(),
            }])
        }
        ArchiveFormat::Rar => {
            let path_str = path.to_string_lossy().to_string();
            let archive = RarArchive::new(&path_str);
            let mut list_archive = archive.open_for_listing().context("Failed to open RAR archive")?;
            let mut entries = Vec::new();

            for entry_result in list_archive.by_ref() {
                let entry = entry_result?;
                entries.push(ArchiveEntry {
                    name: entry.filename.to_string_lossy().to_string(),
                    is_dir: entry.is_directory(),
                    size: entry.unpacked_size,
                    compressed_size: entry.unpacked_size, // unrar doesn't provide packed_size in list mode
                    path: entry.filename,
                });
            }
            Ok(entries)
        }
        ArchiveFormat::Unknown => {
            anyhow::bail!("Unknown archive format");
        }
    }
}

/// Main extraction function
pub fn extract_archive(
    path: &Path,
    dest: &Path,
    progress: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    cancel_flag: Arc<AtomicBool>,
    password: Option<&str>,
) -> Result<usize> {
    let format = ArchiveFormat::detect(path).context("Unknown archive format")?;

    // Create destination directory
    fs::create_dir_all(dest).context("Failed to create destination directory")?;

    match format {
        ArchiveFormat::Zip => extract_zip(path, dest, progress, total, cancel_flag, password),
        ArchiveFormat::Tar => extract_tar(path, dest, progress, total, cancel_flag, password),
        ArchiveFormat::Gzip => extract_gzip(path, dest, progress, total, cancel_flag, password),
        ArchiveFormat::Bzip2 => extract_bzip2(path, dest, progress, total, cancel_flag, password),
        ArchiveFormat::Xz => extract_xz(path, dest, progress, total, cancel_flag, password),
        ArchiveFormat::Rar => extract_rar(path, dest, progress, total, cancel_flag, password),
        ArchiveFormat::Unknown => anyhow::bail!("Unknown archive format"),
    }
}
