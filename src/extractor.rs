use anyhow::{Context, Result};
use brotli::Decompressor as BrotliDecoder;
use flate2::read::GzDecoder;
use lz4_flex::frame::FrameDecoder as Lz4Decoder;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tar::Archive;
use unrar::Archive as RarArchive;
use xz2::read::XzDecoder;
use zip::ZipArchive;
use zstd::stream::read::Decoder as ZstdDecoder;

/// Helper to sanitize and validate target paths to prevent directory traversal
fn sanitize_target_path(dest: &Path, entry_path: &Path) -> Result<PathBuf> {
    let mut safe_path = PathBuf::new();
    for component in entry_path.components() {
        match component {
            std::path::Component::Normal(c) => safe_path.push(c),
            std::path::Component::ParentDir => {
                anyhow::bail!("Directory traversal attempt detected: {:?}", entry_path);
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                // Skip absolute prefixes to enforce relativity to dest
            }
            std::path::Component::CurDir => {}
        }
    }
    let full_path = dest.join(safe_path);
    if full_path.starts_with(dest) {
        Ok(full_path)
    } else {
        anyhow::bail!("Path escapes destination directory: {:?}", entry_path);
    }
}

/// Generic Read wrapper that tracks the number of bytes read for progress reporting.
struct ProgressReader<R> {
    inner: R,
    progress: Arc<AtomicUsize>,
}

impl<R: Read> Read for ProgressReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes = self.inner.read(buf)?;
        self.progress.fetch_add(bytes, Ordering::Relaxed);
        Ok(bytes)
    }
}

/// Generic single-file decompressor — handles the shared boilerplate for all
/// single-file compression formats (gzip, bzip2, xz, zstd, brotli, lz4).
///
/// Opens the file, wraps it in a [`ProgressReader`], constructs a format-specific
/// decoder via `make_decoder`, derives the output name by stripping known
/// extensions, and streams the decompressed data to the output file.
fn extract_single_file<D: Read>(
    ctx: &ExtractionContext,
    extensions: &[&str],
    format_label: &str,
    make_decoder: impl FnOnce(ProgressReader<File>) -> Result<D>,
) -> Result<usize> {
    if ctx.cancel_flag.load(Ordering::Relaxed) {
        anyhow::bail!("Extraction cancelled");
    }

    let file = File::open(ctx.path)
        .with_context(|| format!("Failed to open {} file", format_label))?;
    let metadata = file.metadata()
        .with_context(|| format!("Failed to read {} file metadata", format_label))?;
    ctx.total.store(metadata.len() as usize, Ordering::Relaxed);

    let progress_reader = ProgressReader {
        inner: file,
        progress: Arc::clone(&ctx.progress),
    };
    let mut decoder = make_decoder(progress_reader)
        .with_context(|| format!("Failed to create {} decoder", format_label))?;

    // Derive output filename by stripping a known extension
    let raw_name = ctx.path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut output_name = raw_name.clone();
    for ext in extensions {
        if output_name.ends_with(ext) {
            output_name = output_name[..output_name.len() - ext.len()].to_string();
            break;
        }
    }
    if output_name == raw_name {
        // No extension matched — the file may be misnamed
        anyhow::bail!(
            "File '{}' does not have expected extension {:?}",
            ctx.path.display(),
            extensions
        );
    }
    if output_name.is_empty() {
        output_name = "output".to_string();
    }

    let out_path = ctx.dest.join(&output_name);
    let mut outfile = BufWriter::new(
        File::create(&out_path).context("Failed to create output file")?,
    );

    io::copy(&mut decoder, &mut outfile)
        .with_context(|| format!("Failed to decompress {} file", format_label))?;
    outfile.flush().context("Failed to flush output file")?;

    Ok(1)
}

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

/// Shared context for extraction operations, eliminating repeated
/// parameter lists across all extraction functions.
pub struct ExtractionContext<'a> {
    pub path: &'a Path,
    pub dest: &'a Path,
    pub progress: Arc<AtomicUsize>,
    pub total: Arc<AtomicUsize>,
    pub cancel_flag: Arc<AtomicBool>,
    pub password: Option<&'a str>,
}

/// Check if a ZIP archive is password protected
pub fn is_zip_encrypted(path: &Path) -> bool {
    if let Ok(file) = File::open(path) {
        if let Ok(mut archive) = ZipArchive::new(file) {
            for i in 0..archive.len() {
                match archive.by_index(i) {
                    Ok(_) => continue,
                    Err(zip::result::ZipError::UnsupportedArchive(zip::result::ZipError::PASSWORD_REQUIRED)) => return true,
                    Err(_) => continue,
                }
            }
        }
    }
    false
}

/// Check if a RAR archive is password protected
pub fn is_rar_encrypted(path: &Path) -> bool {
    let path_str = path.to_string_lossy().to_string();
    if let Ok(mut archive) = RarArchive::new(&path_str).open_for_listing() {
        for entry in archive.by_ref() {
            match entry {
                Ok(entry) => {
                    if entry.is_encrypted() {
                        return true;
                    }
                }
                // Listing failures on protected archives are frequently surfaced here.
                Err(_) => return true,
            }
        }
        false
    } else {
        // If opening for listing fails, it might be encrypted (RAR returns error
        // when listing an encrypted archive without password)
        true
    }
}

/// Check if a 7z archive is password protected
pub fn is_sevenzip_encrypted(path: &Path) -> bool {
    use sevenz_rust::{SevenZReader, Password};
    match SevenZReader::open(path, Password::default()) {
        Err(sevenz_rust::Error::PasswordRequired) | Err(sevenz_rust::Error::MaybeBadPassword(_)) => true,
        Ok(mut reader) => {
            // Check if file data is encrypted by trying to read from the first entry
            let res = reader.for_each_entries(|_entry, r| {
                let mut buf = [0u8; 1];
                let _ = r.read(&mut buf).map_err(sevenz_rust::Error::io)?;
                Ok(false) // stop after first entry
            });
            matches!(
                res,
                Err(sevenz_rust::Error::PasswordRequired) | Err(sevenz_rust::Error::MaybeBadPassword(_))
            )
        }
        _ => false,
    }
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
    SevenZip,
    Zstd,
    Brotli,
    Lz4,
    Unknown,
}

impl ArchiveFormat {
    pub fn is_single_file(self) -> bool {
        matches!(
            self,
            ArchiveFormat::Gzip
                | ArchiveFormat::Bzip2
                | ArchiveFormat::Xz
                | ArchiveFormat::Zstd
                | ArchiveFormat::Brotli
                | ArchiveFormat::Lz4
        )
    }

    pub fn from_extension(path: &Path) -> Self {
        match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
            "zip" => ArchiveFormat::Zip,
            "tar" => ArchiveFormat::Tar,
            "gz" | "gzip" => ArchiveFormat::Gzip,
            "bz2" | "bzip2" => ArchiveFormat::Bzip2,
            "xz" | "lzma" => ArchiveFormat::Xz,
            "rar" => ArchiveFormat::Rar,
            "7z" | "7zip" => ArchiveFormat::SevenZip,
            "zst" | "zstd" => ArchiveFormat::Zstd,
            "br" | "brotli" => ArchiveFormat::Brotli,
            "lz4" => ArchiveFormat::Lz4,
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
        // 7z: 37 7A BC AF 27 1C
        if data.len() >= 6 && data[0..6] == [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] {
            return Some(ArchiveFormat::SevenZip);
        }
        // Zstandard: 28 B5 2F FD
        if data.len() >= 4 && data[0..4] == [0x28, 0xB5, 0x2F, 0xFD] {
            return Some(ArchiveFormat::Zstd);
        }
        // LZ4 frame: 04 22 4D 18
        if data.len() >= 4 && data[0..4] == [0x04, 0x22, 0x4D, 0x18] {
            return Some(ArchiveFormat::Lz4);
        }
        // LZ4 legacy: 02 21 4C 18
        if data.len() >= 4 && data[0..4] == [0x02, 0x21, 0x4C, 0x18] {
            return Some(ArchiveFormat::Lz4);
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
///
/// Reads the file into memory once and shares the buffer across parallel workers
/// via [`Arc`], avoiding N+1 file opens (one per entry + one for counting).
pub fn extract_zip(ctx: &ExtractionContext) -> Result<usize> {
    use rayon::prelude::*;

    // Read the ZIP file into memory once, then share the buffer via Arc.
    // Each parallel worker creates a cheap Cursor over the shared data
    // instead of reopening the file for every entry.
    let data = Arc::new(std::fs::read(ctx.path).context("Failed to read ZIP file")?);

    let total_files = {
        let bytes: &[u8] = &*data;
        let archive = ZipArchive::new(std::io::Cursor::new(bytes))
            .context("Invalid ZIP archive")?;
        archive.len()
    };
    ctx.total.store(total_files, Ordering::Relaxed);

    let created_dirs = std::sync::Mutex::new(std::collections::HashSet::new());

    (0..total_files).into_par_iter().try_for_each(|i| {
        if ctx.cancel_flag.load(Ordering::Relaxed) {
            return Err(anyhow::anyhow!("Extraction cancelled"));
        }

        // Create archive from the shared in-memory buffer — no file open needed
        let bytes: &[u8] = &**data;
        let mut archive = ZipArchive::new(std::io::Cursor::new(bytes))
            .context("Invalid ZIP archive")?;

        let mut entry = if let Some(pwd) = ctx.password {
            match archive.by_index_decrypt(i, pwd.as_bytes()) {
                Ok(result) => result.map_err(|_| anyhow::anyhow!("Invalid password"))?,
                Err(e) => return Err(e).context("Failed to decrypt entry"),
            }
        } else {
            match archive.by_index(i) {
                Ok(e) => e,
                Err(zip::result::ZipError::UnsupportedArchive(_)) => {
                    anyhow::bail!("Archive contains encrypted entries, please provide a password");
                }
                Err(e) => return Err(e).context("Failed to read entry"),
            }
        };

        let entry_path = entry
            .enclosed_name()
            .ok_or_else(|| anyhow::anyhow!("Entry path escapes archive directory: {}", entry.name()))?;
        let out_path = sanitize_target_path(ctx.dest, entry_path)?;

        if entry.name().ends_with('/') {
            let mut created_dirs_lock = created_dirs.lock().unwrap();
            if !created_dirs_lock.contains(&out_path) {
                fs::create_dir_all(&out_path).context("Failed to create directory")?;
                created_dirs_lock.insert(out_path.clone());
            }
        } else {
            if let Some(parent) = out_path.parent() {
                let mut created_dirs_lock = created_dirs.lock().unwrap();
                if !created_dirs_lock.contains(parent) {
                    fs::create_dir_all(parent).context("Failed to create parent directory")?;
                    created_dirs_lock.insert(parent.to_path_buf());
                }
            }

            let mut outfile = BufWriter::new(
                File::create(&out_path).context("Failed to create output file")?
            );
            io::copy(&mut entry, &mut outfile).context("Failed to extract file")?;
            outfile.flush().context("Failed to flush output file")?;
        }

        ctx.progress.fetch_add(1, Ordering::Relaxed);
        Ok(())
    })?;

    Ok(total_files)
}

/// Extract a TAR archive
pub fn extract_tar(ctx: &ExtractionContext) -> Result<usize> {
    let file = File::open(ctx.path).context("Failed to open TAR file")?;
    let mut archive = Archive::new(BufReader::new(file));
    let mut extracted = 0;
    let mut created_dirs = std::collections::HashSet::new();

    for entry in archive.entries().context("Failed to read entries")? {
        if ctx.cancel_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Extraction cancelled");
        }

        let mut entry = entry.context("Failed to read entry")?;
        let entry_path = entry.path().context("Failed to read entry path")?.to_path_buf();
        let out_path = sanitize_target_path(ctx.dest, &entry_path)?;

        if let Some(parent) = out_path.parent() {
            if !created_dirs.contains(parent) {
                fs::create_dir_all(parent).context("Failed to create directory")?;
                created_dirs.insert(parent.to_path_buf());
            }
        }

        entry.unpack(&out_path).context("Failed to extract entry")?;
        extracted += 1;
        ctx.progress.fetch_add(1, Ordering::Relaxed);
    }

    ctx.total.store(extracted, Ordering::Relaxed);
    Ok(extracted)
}

/// Extract a GZIP file
pub fn extract_gzip(ctx: &ExtractionContext) -> Result<usize> {
    extract_single_file(ctx, &[".gz", ".gzip"], "GZIP", |r| Ok(GzDecoder::new(r)))
}

/// Extract a BZIP2 file
pub fn extract_bzip2(ctx: &ExtractionContext) -> Result<usize> {
    extract_single_file(ctx, &[".bz2"], "BZIP2", |r| Ok(bzip2::read::BzDecoder::new(r)))
}

/// Extract an XZ file
pub fn extract_xz(ctx: &ExtractionContext) -> Result<usize> {
    extract_single_file(ctx, &[".xz"], "XZ", |r| Ok(XzDecoder::new(r)))
}

/// Extract a 7z archive
pub fn extract_sevenzip(ctx: &ExtractionContext) -> Result<usize> {
    use sevenz_rust::{SevenZReader, Password};

    if ctx.cancel_flag.load(Ordering::Relaxed) {
        anyhow::bail!("Extraction cancelled");
    }

    let p = match ctx.password {
        Some(pwd) => Password::from(pwd),
        None => Password::default(),
    };

    let mut reader = SevenZReader::open(ctx.path, p).context("Failed to open 7z archive")?;
    let total_entries = reader.archive().files.len();
    ctx.total.store(total_entries, Ordering::Relaxed);

    let mut extracted = 0;
    let mut created_dirs = std::collections::HashSet::new();

    let res = reader.for_each_entries(|entry, file_reader| {
        if ctx.cancel_flag.load(Ordering::Relaxed) {
            return Err(sevenz_rust::Error::other("Extraction cancelled"));
        }

        let entry_path = Path::new(&entry.name);
        let out_path = sanitize_target_path(ctx.dest, entry_path)
            .map_err(|e| sevenz_rust::Error::other(e.to_string()))?;

        if entry.is_directory() {
            if !created_dirs.contains(&out_path) {
                fs::create_dir_all(&out_path).map_err(sevenz_rust::Error::io)?;
                created_dirs.insert(out_path.clone());
            }
        } else {
            if let Some(parent) = out_path.parent() {
                if !created_dirs.contains(parent) {
                    fs::create_dir_all(parent).map_err(sevenz_rust::Error::io)?;
                    created_dirs.insert(parent.to_path_buf());
                }
            }

            let mut outfile = BufWriter::new(
                File::create(&out_path).map_err(sevenz_rust::Error::io)?
            );
            std::io::copy(file_reader, &mut outfile).map_err(sevenz_rust::Error::io)?;
            outfile.flush().map_err(sevenz_rust::Error::io)?;
        }

        extracted += 1;
        ctx.progress.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    });

    match res {
        Ok(_) => Ok(extracted),
        Err(e) => {
            if e.to_string().contains("cancelled") {
                anyhow::bail!("Extraction cancelled");
            }
            Err(anyhow::anyhow!("Failed to extract 7z archive: {}", e))
        }
    }
}

/// Extract a Zstandard file
pub fn extract_zstd(ctx: &ExtractionContext) -> Result<usize> {
    extract_single_file(ctx, &[".zstd", ".zst"], "Zstandard", |r| {
        ZstdDecoder::new(r).context("Failed to create Zstandard decoder")
    })
}

/// Extract a Brotli file
pub fn extract_brotli(ctx: &ExtractionContext) -> Result<usize> {
    extract_single_file(ctx, &[".br"], "Brotli", |r| Ok(BrotliDecoder::new(r, 4096)))
}

/// Extract an LZ4 file
pub fn extract_lz4(ctx: &ExtractionContext) -> Result<usize> {
    extract_single_file(ctx, &[".lz4"], "LZ4", |r| Ok(Lz4Decoder::new(r)))
}

/// Extract a RAR archive
pub fn extract_rar(ctx: &ExtractionContext) -> Result<usize> {
    let path_str = ctx.path.to_string_lossy().to_string();

    // Open for listing first to count entries
    let archive = RarArchive::new(&path_str);
    let mut list_archive = archive
        .open_for_listing()
        .context("Failed to open RAR archive")?;
    let entries: Vec<_> = list_archive.by_ref().filter_map(|e| e.ok()).collect();
    ctx.total.store(entries.len(), Ordering::Relaxed);

    // Now open for processing/extraction (with password if provided)
    let archive = if let Some(pwd) = ctx.password {
        RarArchive::with_password(&path_str, pwd)
    } else {
        RarArchive::new(&path_str)
    };

    let mut process_archive = archive
        .open_for_processing()
        .context("Failed to open RAR for processing")?;

    let mut extracted = 0;
    let mut created_dirs = std::collections::HashSet::new();

    loop {
        if ctx.cancel_flag.load(Ordering::Relaxed) {
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
        let out_path = sanitize_target_path(ctx.dest, &entry_path)?;

        if let Some(parent) = out_path.parent() {
            if !created_dirs.contains(parent) {
                fs::create_dir_all(parent).context("Failed to create directory")?;
                created_dirs.insert(parent.to_path_buf());
            }
        }

        if header.is_directory() {
            if !created_dirs.contains(&out_path) {
                fs::create_dir_all(&out_path).ok();
                created_dirs.insert(out_path.clone());
            }
        } else {
            // Extract the file to the sanitized path instead of using
            // extract_with_base (which uses the raw entry filename and
            // would bypass path traversal protection)
            process_archive = archive_with_header
                .extract_to(&out_path)
                .context("Failed to extract file")?;
            extracted += 1;
            ctx.progress.fetch_add(1, Ordering::Relaxed);
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
                    path: entry
                        .enclosed_name()
                        .unwrap_or_else(|| Path::new(entry.name()))
                        .to_path_buf(),
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
        ArchiveFormat::Gzip | ArchiveFormat::Bzip2 | ArchiveFormat::Xz | ArchiveFormat::Zstd | ArchiveFormat::Brotli | ArchiveFormat::Lz4 => {
            let file = File::open(path)?;
            let metadata = file.metadata()?;
            Ok(vec![ArchiveEntry {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                is_dir: false,
                size: metadata.len(),
                compressed_size: metadata.len(),
                path: path.to_path_buf(),
            }])
        }
        ArchiveFormat::SevenZip => {
            use sevenz_rust::{SevenZReader, Password};
            let reader = SevenZReader::open(path, Password::default())
                .map_err(|e| anyhow::anyhow!("Failed to list 7z archive: {}", e))?;
            let archive = reader.archive();
            let mut entries = Vec::new();
            for entry in &archive.files {
                let name = entry.name.to_string();
                entries.push(ArchiveEntry {
                    name: name.clone(),
                    is_dir: entry.is_directory(),
                    size: entry.size,
                    compressed_size: entry.compressed_size,
                    path: std::path::Path::new(&name).to_path_buf(),
                });
            }
            Ok(entries)
        }
        ArchiveFormat::Rar => {
            let path_str = path.to_string_lossy().to_string();
            let archive = RarArchive::new(&path_str);
            let mut list_archive = archive
                .open_for_listing()
                .context("Failed to open RAR archive")?;
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

/// Main extraction function — dispatches to the format-specific extractor,
/// all of which receive the shared extraction context.
pub fn extract_archive(ctx: &ExtractionContext) -> Result<usize> {
    let format = ArchiveFormat::detect(ctx.path).context("Unknown archive format")?;

    // Create destination directory
    fs::create_dir_all(ctx.dest).context("Failed to create destination directory")?;

    match format {
        ArchiveFormat::Zip => extract_zip(ctx),
        ArchiveFormat::Tar => extract_tar(ctx),
        ArchiveFormat::Gzip => extract_gzip(ctx),
        ArchiveFormat::Bzip2 => extract_bzip2(ctx),
        ArchiveFormat::Xz => extract_xz(ctx),
        ArchiveFormat::SevenZip => extract_sevenzip(ctx),
        ArchiveFormat::Zstd => extract_zstd(ctx),
        ArchiveFormat::Brotli => extract_brotli(ctx),
        ArchiveFormat::Lz4 => extract_lz4(ctx),
        ArchiveFormat::Rar => extract_rar(ctx),
        ArchiveFormat::Unknown => anyhow::bail!("Unknown archive format"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_archive_format_from_extension() {
        assert_eq!(
            ArchiveFormat::from_extension(Path::new("test.zip")),
            ArchiveFormat::Zip
        );
        assert_eq!(
            ArchiveFormat::from_extension(Path::new("test.tar")),
            ArchiveFormat::Tar
        );
        assert_eq!(
            ArchiveFormat::from_extension(Path::new("test.gz")),
            ArchiveFormat::Gzip
        );
        assert_eq!(
            ArchiveFormat::from_extension(Path::new("test.gzip")),
            ArchiveFormat::Gzip
        );
        assert_eq!(
            ArchiveFormat::from_extension(Path::new("test.bz2")),
            ArchiveFormat::Bzip2
        );
        assert_eq!(
            ArchiveFormat::from_extension(Path::new("test.xz")),
            ArchiveFormat::Xz
        );
        assert_eq!(
            ArchiveFormat::from_extension(Path::new("test.rar")),
            ArchiveFormat::Rar
        );
        assert_eq!(
            ArchiveFormat::from_extension(Path::new("test.unknown")),
            ArchiveFormat::Unknown
        );
    }

    #[test]
    fn test_archive_format_from_magic_bytes_zip() {
        // ZIP magic bytes: 50 4B 03 04
        let data = vec![0x50, 0x4B, 0x03, 0x04, 0x00, 0x00];
        assert_eq!(
            ArchiveFormat::from_magic_bytes(&data),
            Some(ArchiveFormat::Zip)
        );
    }

    #[test]
    fn test_archive_format_from_magic_bytes_gzip() {
        // GZIP magic bytes: 1F 8B
        let data = vec![0x1F, 0x8B, 0x08, 0x00];
        assert_eq!(
            ArchiveFormat::from_magic_bytes(&data),
            Some(ArchiveFormat::Gzip)
        );
    }

    #[test]
    fn test_archive_format_from_magic_bytes_bzip2() {
        // BZIP2 magic bytes: 42 5A 68
        let data = vec![0x42, 0x5A, 0x68, 0x39];
        assert_eq!(
            ArchiveFormat::from_magic_bytes(&data),
            Some(ArchiveFormat::Bzip2)
        );
    }

    #[test]
    fn test_archive_format_from_magic_bytes_xz() {
        // XZ magic bytes: FD 37 7A 58 5A 00
        let data = vec![0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00];
        assert_eq!(
            ArchiveFormat::from_magic_bytes(&data),
            Some(ArchiveFormat::Xz)
        );
    }

    #[test]
    fn test_archive_format_from_magic_bytes_rar_v15() {
        // RAR v1.5 magic bytes: 52 61 72 21 1A 07 00
        let data = vec![0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00];
        assert_eq!(
            ArchiveFormat::from_magic_bytes(&data),
            Some(ArchiveFormat::Rar)
        );
    }

    #[test]
    fn test_archive_format_from_magic_bytes_rar_v20() {
        // RAR v2.0 magic bytes: 52 61 72 21 1A 07 01 00
        let data = vec![0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00];
        assert_eq!(
            ArchiveFormat::from_magic_bytes(&data),
            Some(ArchiveFormat::Rar)
        );
    }

    #[test]
    fn test_archive_format_from_magic_bytes_empty() {
        let data: Vec<u8> = vec![];
        assert_eq!(ArchiveFormat::from_magic_bytes(&data), None);
    }

    #[test]
    fn test_archive_format_from_magic_bytes_unknown() {
        let data = vec![0x00, 0x01, 0x02, 0x03];
        assert_eq!(ArchiveFormat::from_magic_bytes(&data), None);
    }

    #[test]
    fn test_sanitize_target_path() {
        let dest = Path::new("/target/dir");
        
        // Safe relative paths
        assert_eq!(
            sanitize_target_path(dest, Path::new("file.txt")).unwrap(),
            dest.join("file.txt")
        );
        assert_eq!(
            sanitize_target_path(dest, Path::new("sub/dir/file.txt")).unwrap(),
            dest.join("sub/dir/file.txt")
        );

        // Path containing CurDir components
        assert_eq!(
            sanitize_target_path(dest, Path::new("./file.txt")).unwrap(),
            dest.join("file.txt")
        );

        // Absolute paths should be converted to relative to dest
        assert_eq!(
            sanitize_target_path(dest, Path::new("/file.txt")).unwrap(),
            dest.join("file.txt")
        );

        // Directory traversal using ParentDir components should fail
        assert!(sanitize_target_path(dest, Path::new("../file.txt")).is_err());
        assert!(sanitize_target_path(dest, Path::new("sub/../../file.txt")).is_err());
    }

    #[test]
    fn test_create_temp_dir_for_extraction() {
        let temp_dir = TempDir::new().unwrap();
        let dest = temp_dir.path().join("output");
        fs::create_dir_all(&dest).unwrap();
        assert!(dest.exists());
    }
}

