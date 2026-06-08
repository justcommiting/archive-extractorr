use anyhow::{Context, Result};
use brotli::Decompressor as BrotliDecoder;
use flate2::read::GzDecoder;
use lz4_flex::frame::FrameDecoder as Lz4Decoder;

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tar::Archive;
use unrar::Archive as RarArchive;
use xz2::read::XzDecoder;
use zip::ZipArchive;
use zstd::stream::read::Decoder as ZstdDecoder;

/// Copy data from reader to writer using a caller-supplied buffer,
/// replacing the default 8 KB buffer used by `io::copy`.
fn copy_buffered<R: Read + ?Sized, W: Write>(
    reader: &mut R,
    writer: &mut W,
    buf: &mut [u8],
) -> io::Result<u64> {
    let mut total = 0;
    loop {
        let n = reader.read(buf)?;
        if n == 0 {
            return Ok(total);
        }
        writer.write_all(&buf[..n])?;
        total += n as u64;
    }
}

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
                // Archive entries with `C:\foo` or `\\.\` prefixes have those
                // components stripped to enforce relativity to `dest`. The
                // `starts_with(dest)` check below catches actual traversal.
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

/// Type-safe sentinel for 7z cancellation, avoiding string-based error matching.
struct ExtractionCancelled;

impl std::fmt::Display for ExtractionCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Extraction cancelled")
    }
}

impl std::fmt::Debug for ExtractionCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ExtractionCancelled")
    }
}

impl std::error::Error for ExtractionCancelled {}

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

    let file =
        File::open(ctx.path).with_context(|| format!("Failed to open {} file", format_label))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("Failed to read {} file metadata", format_label))?;
    ctx.total.store(metadata.len() as usize, Ordering::Relaxed);

    let progress_reader = ProgressReader {
        inner: file,
        progress: Arc::clone(&ctx.progress),
    };
    let mut decoder = make_decoder(progress_reader)
        .with_context(|| format!("Failed to create {} decoder", format_label))?;

    // Derive output filename by stripping a known extension
    let raw_name = ctx
        .path
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
    let mut outfile = BufWriter::with_capacity(
        64 * 1024,
        File::create(&out_path).context("Failed to create output file")?,
    );

    let mut buf = vec![0u8; 256 * 1024];
    copy_buffered(&mut decoder, &mut outfile, &mut buf)
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
    #[expect(dead_code, reason = "Reserved for future CLI display or progress reporting")]
    pub compressed_size: u64,
    pub path: std::path::PathBuf,
}

/// Shared context for extraction operations, eliminating repeated
/// parameter lists across all extraction functions.
///
/// # Progress semantics
/// - For single-file formats (Gzip, Bzip2, Xz, Zstd, Brotli, Lz4): `progress` and `total`
///   represent **byte counts** (compressed bytes read from disk / total file size).
/// - For multi-file archives (Zip, Tar, Rar, 7z): `progress` and `total` represent
///   **file counts** (files extracted / total files).
///   The UI branches on `ArchiveFormat::is_single_file()` to format accordingly.
pub struct ExtractionContext<'a> {
    pub path: &'a Path,
    pub dest: &'a Path,
    pub progress: Arc<AtomicUsize>,
    pub total: Arc<AtomicUsize>,
    pub cancel_flag: Arc<AtomicBool>,
    pub password: Option<&'a str>,
    /// If `Some`, only extract entries whose name (as reported by `ArchiveEntry.name`)
    /// matches one of these values. Paths are matched exactly (not substrings).
    /// When `None`, all entries are extracted.
    pub files: Option<Vec<String>>,
}

impl ExtractionContext<'_> {
    pub fn should_extract(&self, name: &str) -> bool {
        match &self.files {
            Some(files) => files.iter().any(|f| f == name),
            None => true,
        }
    }
}

/// Check if a ZIP archive is password protected
/// Only checks the first 10 entries to avoid O(n) scan of large archives.
pub fn is_zip_encrypted(path: &Path) -> bool {
    if let Ok(file) = File::open(path) {
        if let Ok(mut archive) = ZipArchive::new(file) {
            let limit = archive.len().min(10);
            for i in 0..limit {
                match archive.by_index(i) {
                    Ok(_) => continue,
                    Err(zip::result::ZipError::UnsupportedArchive(
                        zip::result::ZipError::PASSWORD_REQUIRED,
                    )) => return true,
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
                Err(_) => return true,
            }
        }
        false
    } else {
        false
    }
}

/// Check if a 7z archive is password protected
pub fn is_sevenzip_encrypted(path: &Path) -> bool {
    use sevenz_rust::{Password, SevenZReader};
    match SevenZReader::open(path, Password::default()) {
        Err(sevenz_rust::Error::PasswordRequired)
        | Err(sevenz_rust::Error::MaybeBadPassword(_)) => true,
        Ok(mut reader) => {
            // Check if file data is encrypted by trying to read from the first entry
            let res = reader.for_each_entries(|_entry, r| {
                let mut buf = [0u8; 1];
                let _ = r.read(&mut buf).map_err(sevenz_rust::Error::io)?;
                Ok(false) // stop after first entry
            });
            matches!(
                res,
                Err(sevenz_rust::Error::PasswordRequired)
                    | Err(sevenz_rust::Error::MaybeBadPassword(_))
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

/// Tracks created directories during extraction, using interior mutability so it
/// works in both single-threaded and parallel (rayon) contexts.
struct DirTracker {
    created: Mutex<HashSet<PathBuf>>,
}

impl DirTracker {
    fn new() -> Self {
        Self {
            created: Mutex::new(HashSet::new()),
        }
    }

    fn ensure_parent(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            let mut created = self.created.lock().unwrap();
            if !created.contains(parent) {
                fs::create_dir_all(parent)?;
                created.insert(parent.to_path_buf());
            }
        }
        Ok(())
    }

    fn ensure_dir(&self, path: &Path) -> Result<()> {
        let mut created = self.created.lock().unwrap();
        if !created.contains(path) {
            fs::create_dir_all(path)?;
            created.insert(path.to_path_buf());
        }
        Ok(())
    }
}

/// Compute an adaptive chunk size for parallel ZIP extraction.
/// Targets roughly one chunk per CPU to balance load with minimal overhead.
fn zip_chunk_size(total_files: usize) -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (total_files / cpus).clamp(64, 2048)
}

/// Extract a range of entries from a ZIP archive. Used by both the
/// single-threaded fast path and the parallel chunked path.
fn extract_zip_entries(
    bytes: &[u8],
    ctx: &ExtractionContext,
    indices: &[usize],
    created_dirs: &DirTracker,
) -> Result<()> {
    let mut archive =
        ZipArchive::new(std::io::Cursor::new(bytes)).context("Invalid ZIP archive")?;
    let mut buf = vec![0u8; 256 * 1024];

    for &i in indices {
        if ctx.cancel_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Extraction cancelled");
        }

        let mut entry = if let Some(pwd) = ctx.password {
            match archive.by_index_decrypt(i, pwd.as_bytes()) {
                Ok(result) => result.map_err(|_| anyhow::anyhow!("Invalid password"))?,
                Err(e) => return Err(e).context("Failed to decrypt entry"),
            }
        } else {
            match archive.by_index(i) {
                Ok(e) => e,
                Err(zip::result::ZipError::UnsupportedArchive(_)) => {
                    anyhow::bail!(
                        "Archive contains encrypted entries, please provide a password"
                    );
                }
                Err(e) => return Err(e).context("Failed to read entry"),
            }
        };

        let entry_path = entry.enclosed_name().ok_or_else(|| {
            anyhow::anyhow!("Entry path escapes archive directory: {}", entry.name())
        })?;
        let out_path = sanitize_target_path(ctx.dest, entry_path)?;

        if entry.name().ends_with('/') {
            created_dirs.ensure_dir(&out_path)?;
        } else {
            created_dirs.ensure_parent(&out_path)?;

            let mut outfile = BufWriter::with_capacity(
                64 * 1024,
                File::create(&out_path).context("Failed to create output file")?,
            );
            copy_buffered(&mut entry, &mut outfile, &mut buf).context("Failed to extract file")?;
            outfile.flush().context("Failed to flush output file")?;
        }

        ctx.progress.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

/// Extract a ZIP archive
///
/// Reads the file into memory once and shares the buffer across parallel workers
/// via [`Arc`], avoiding N+1 file opens (one per entry + one for counting).
/// Uses a single-threaded fast path for small archives to avoid chunking
/// overhead and rayon dispatch latency.
pub fn extract_zip(ctx: &ExtractionContext) -> Result<usize> {
    use rayon::prelude::*;

    let file = File::open(ctx.path).context("Failed to open ZIP file")?;
    let mmap = unsafe { memmap2::Mmap::map(&file).context("Failed to mmap ZIP file")? };
    let data: Arc<memmap2::Mmap> = Arc::new(mmap);

    let total_files = {
        let bytes: &[u8] = &data;
        let archive =
            ZipArchive::new(std::io::Cursor::new(bytes)).context("Invalid ZIP archive")?;
        archive.len()
    };
    ctx.total.store(total_files, Ordering::Relaxed);

    let created_dirs = DirTracker::new();
    let bytes: &[u8] = &data;

    // Build index list, filtering by name if ctx.files is set
    let indices: Vec<usize> = match &ctx.files {
        Some(files) => {
            let mut archive =
                ZipArchive::new(std::io::Cursor::new(bytes)).context("Invalid ZIP archive")?;
            let mut matched = Vec::new();
            for i in 0..archive.len() {
                if let Ok(entry) = archive.by_index(i) {
                    let name = entry.name().to_string();
                    if files.iter().any(|f| f == &name) {
                        matched.push(i);
                    }
                }
            }
            matched
        }
        None => (0..total_files).collect(),
    };

    if indices.is_empty() && ctx.files.is_some() {
        anyhow::bail!("No matching files found in archive");
    }

    if indices.len() < 64 {
        extract_zip_entries(bytes, ctx, &indices, &created_dirs)?;
    } else {
        let chunk_size = zip_chunk_size(indices.len());
        let chunks: Vec<&[usize]> = indices.chunks(chunk_size).collect();

        chunks.into_par_iter().try_for_each(|chunk| {
            if ctx.cancel_flag.load(Ordering::Relaxed) {
                return Err(anyhow::anyhow!("Extraction cancelled"));
            }
            extract_zip_entries(bytes, ctx, chunk, &created_dirs)
        })?;
    }

    Ok(indices.len())
}

/// Extract a TAR archive
pub fn extract_tar(ctx: &ExtractionContext) -> Result<usize> {
    let file = File::open(ctx.path).context("Failed to open TAR file")?;
    let mut archive = Archive::new(BufReader::new(file));
    let mut extracted = 0;
    let created_dirs = DirTracker::new();

    for entry in archive.entries().context("Failed to read entries")? {
        if ctx.cancel_flag.load(Ordering::Relaxed) {
            anyhow::bail!("Extraction cancelled");
        }

        let mut entry = entry.context("Failed to read entry")?;
        let entry_name = entry
            .path()
            .context("Failed to read entry path")?
            .to_string_lossy()
            .to_string();

        if !ctx.should_extract(&entry_name) {
            continue;
        }

        let entry_path = Path::new(&entry_name);
        let out_path = sanitize_target_path(ctx.dest, entry_path)?;

        created_dirs.ensure_parent(&out_path)?;

        entry.unpack(&out_path).context("Failed to extract entry")?;
        extracted += 1;
        ctx.progress.fetch_add(1, Ordering::Relaxed);
        ctx.total.store(extracted, Ordering::Relaxed);
    }

    if extracted == 0 && ctx.files.is_some() {
        anyhow::bail!("No matching files found in archive");
    }

    Ok(extracted)
}

/// Extract a GZIP file
pub fn extract_gzip(ctx: &ExtractionContext) -> Result<usize> {
    extract_single_file(ctx, &[".gz", ".gzip"], "GZIP", |r| Ok(GzDecoder::new(r)))
}

/// Extract a BZIP2 file
pub fn extract_bzip2(ctx: &ExtractionContext) -> Result<usize> {
    extract_single_file(ctx, &[".bz2"], "BZIP2", |r| {
        Ok(bzip2::read::BzDecoder::new(r))
    })
}

/// Extract an XZ file
pub fn extract_xz(ctx: &ExtractionContext) -> Result<usize> {
    extract_single_file(ctx, &[".xz"], "XZ", |r| Ok(XzDecoder::new(r)))
}

/// Extract a 7z archive
///
/// Cancellation is detected via [`ExtractionCancelled`] sentinel error, compared
/// by string representation. This was validated against `sevenz_rust` 0.6 and
/// depends on `Error::other()` not wrapping the message.
pub fn extract_sevenzip(ctx: &ExtractionContext) -> Result<usize> {
    use sevenz_rust::{Password, SevenZReader};

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
    let created_dirs = DirTracker::new();
    let mut buf = vec![0u8; 256 * 1024];

    let res = reader.for_each_entries(|entry, file_reader| {
        if ctx.cancel_flag.load(Ordering::Relaxed) {
            return Err(sevenz_rust::Error::other(ExtractionCancelled.to_string()));
        }

        if !ctx.should_extract(&entry.name) {
            extracted += 1;
            ctx.progress.fetch_add(1, Ordering::Relaxed);
            return Ok(true);
        }

        let entry_path = Path::new(&entry.name);
        let out_path = sanitize_target_path(ctx.dest, entry_path)
            .map_err(|e| sevenz_rust::Error::other(e.to_string()))?;

        if entry.is_directory() {
            created_dirs
                .ensure_dir(&out_path)
                .map_err(|e| sevenz_rust::Error::other(format!("{:#}", e)))?;
        } else {
            created_dirs
                .ensure_parent(&out_path)
                .map_err(|e| sevenz_rust::Error::other(format!("{:#}", e)))?;

            let mut outfile = BufWriter::with_capacity(
                64 * 1024,
                File::create(&out_path).map_err(sevenz_rust::Error::io)?,
            );
            copy_buffered(file_reader, &mut outfile, &mut buf).map_err(sevenz_rust::Error::io)?;
            outfile.flush().map_err(sevenz_rust::Error::io)?;
        }

        extracted += 1;
        ctx.progress.fetch_add(1, Ordering::Relaxed);
        Ok(true)
    });

    match res {
        Ok(_) => Ok(extracted),
        Err(e) => {
            if e.to_string() == ExtractionCancelled.to_string() {
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

    // Open for processing/extraction (with password if provided).
    // Progress total is set post-hoc based on extracted count since
    // we skip a separate listing pass to avoid double I/O.
    let archive = if let Some(pwd) = ctx.password {
        RarArchive::with_password(&path_str, pwd)
    } else {
        RarArchive::new(&path_str)
    };

    let mut process_archive = archive
        .open_for_processing()
        .context("Failed to open RAR for processing")?;

    let mut extracted = 0;
    let created_dirs = DirTracker::new();

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
        let entry_name = header.filename.to_string_lossy().to_string();

        if !ctx.should_extract(&entry_name) {
            process_archive = archive_with_header.skip().context("Failed to skip entry")?;
            continue;
        }

        let entry_path = header.filename.clone();
        let out_path = sanitize_target_path(ctx.dest, &entry_path)?;

        if header.is_directory() {
            created_dirs.ensure_dir(&out_path)?;
        } else {
            created_dirs.ensure_parent(&out_path)?;

            // Extract the file to the sanitized path instead of using
            // extract_with_base (which uses the raw entry filename and
            // would bypass path traversal protection)
            process_archive = archive_with_header
                .extract_to(&out_path)
                .context("Failed to extract file")?;
            extracted += 1;
            ctx.progress.fetch_add(1, Ordering::Relaxed);
            ctx.total.store(extracted, Ordering::Relaxed);
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
                let is_dir = header.entry_type() == tar::EntryType::Directory;
                entries.push(ArchiveEntry {
                    name: path.to_string_lossy().to_string(),
                    is_dir,
                    size: header.size().unwrap_or(0),
                    compressed_size: 0,
                    path,
                });
            }
            Ok(entries)
        }
        ArchiveFormat::Gzip
        | ArchiveFormat::Bzip2
        | ArchiveFormat::Xz
        | ArchiveFormat::Zstd
        | ArchiveFormat::Brotli
        | ArchiveFormat::Lz4 => {
            let file = File::open(path)?;
            let metadata = file.metadata()?;
            Ok(vec![ArchiveEntry {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                is_dir: false,
                size: 0,
                compressed_size: metadata.len(),
                path: path.to_path_buf(),
            }])
        }
        ArchiveFormat::SevenZip => {
            use sevenz_rust::{Password, SevenZReader};
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
    use std::io::Write;
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
    fn test_gzip_extraction() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test.tar.gz");
        let dest_dir = temp_dir.path().join("output");
        fs::create_dir_all(&dest_dir).unwrap();

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"Hello, gzip!").unwrap();
        let compressed = encoder.finish().unwrap();
        std::fs::write(&archive_path, compressed).unwrap();

        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));

        let ctx = ExtractionContext {
            path: &archive_path,
            dest: &dest_dir,
            progress,
            total,
            cancel_flag: cancel,
            password: None,
            files: None,
        };

        let result = extract_gzip(&ctx);
        assert!(result.is_ok());

        let output_path = dest_dir.join("test.tar");
        assert!(output_path.exists());
        let content = std::fs::read_to_string(output_path).unwrap();
        assert_eq!(content, "Hello, gzip!");
    }

    #[test]
    fn test_zip_extract_small_archive() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test.zip");
        let dest_dir = temp_dir.path().join("output");

        let file = File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("hello.txt", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"Hello, world!").unwrap();
        zip.finish().unwrap();

        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));

        let ctx = ExtractionContext {
            path: &archive_path,
            dest: &dest_dir,
            progress,
            total,
            cancel_flag: cancel,
            password: None,
            files: None,
        };

        let result = extract_zip(&ctx);
        assert!(result.is_ok());
        assert!(dest_dir.join("hello.txt").exists());
        let content = std::fs::read_to_string(dest_dir.join("hello.txt")).unwrap();
        assert_eq!(content, "Hello, world!");
    }

    #[test]
    fn test_zip_extract_with_subdirs() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test.zip");
        let dest_dir = temp_dir.path().join("output");

        let file = File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.add_directory("subdir/", zip::write::FileOptions::default())
            .unwrap();
        zip.start_file("subdir/file.txt", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"nested").unwrap();
        zip.finish().unwrap();

        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));

        let ctx = ExtractionContext {
            path: &archive_path,
            dest: &dest_dir,
            progress,
            total,
            cancel_flag: cancel,
            password: None,
            files: None,
        };

        let result = extract_zip(&ctx);
        assert!(result.is_ok());
        assert!(dest_dir.join("subdir/file.txt").exists());
        let content = std::fs::read_to_string(dest_dir.join("subdir/file.txt")).unwrap();
        assert_eq!(content, "nested");
    }

    #[test]
    fn test_tar_extract_small_archive() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test.tar");
        let dest_dir = temp_dir.path().join("output");

        let file = File::create(&archive_path).unwrap();
        let mut tar = tar::Builder::new(file);
        let mut header = tar::Header::new_gnu();
        header.set_size(5);
        header.set_entry_type(tar::EntryType::Regular);
        tar.append_data(&mut header, "data.txt", &b"hello"[..])
            .unwrap();
        tar.finish().unwrap();

        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));

        let ctx = ExtractionContext {
            path: &archive_path,
            dest: &dest_dir,
            progress,
            total,
            cancel_flag: cancel,
            password: None,
            files: None,
        };

        let result = extract_tar(&ctx);
        assert!(result.is_ok());
        assert!(dest_dir.join("data.txt").exists());
        let content = std::fs::read_to_string(dest_dir.join("data.txt")).unwrap();
        assert_eq!(content, "hello");
    }

    #[test]
    fn test_path_traversal_rejected_zip() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("traversal.zip");
        let dest_dir = temp_dir.path().join("output");

        let file = File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("../escape.txt", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"pwned").unwrap();
        zip.finish().unwrap();

        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));

        let ctx = ExtractionContext {
            path: &archive_path,
            dest: &dest_dir,
            progress,
            total,
            cancel_flag: cancel,
            password: None,
            files: None,
        };

        let result = extract_zip(&ctx);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("traversal") || err.contains("escapes"));
    }

    #[test]
    fn test_sanitize_target_path_rejects_traversal() {
        let dest = Path::new("/safe/dir");
        assert!(sanitize_target_path(dest, Path::new("../evil")).is_err());
        assert!(sanitize_target_path(dest, Path::new("a/../../evil")).is_err());
        assert!(sanitize_target_path(dest, Path::new("/absolute/path")).is_ok());
    }

    #[test]
    fn test_list_archive_zip() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test.zip");

        let file = File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("file_a.txt", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"aaa").unwrap();
        zip.start_file("file_b.txt", zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(b"bbb").unwrap();
        zip.finish().unwrap();

        let entries = list_archive(&archive_path).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.name == "file_a.txt"));
        assert!(entries.iter().any(|e| e.name == "file_b.txt"));
    }

    #[test]
    fn test_extract_cancellation_during_zip() {
        let temp_dir = TempDir::new().unwrap();
        let archive_path = temp_dir.path().join("test.zip");
        let dest_dir = temp_dir.path().join("output");

        let file = File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        for i in 0..10 {
            zip.start_file(
                format!("file_{}.txt", i),
                zip::write::FileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"data").unwrap();
        }
        zip.finish().unwrap();

        let progress = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(true));

        let ctx = ExtractionContext {
            path: &archive_path,
            dest: &dest_dir,
            progress,
            total,
            cancel_flag: cancel,
            password: None,
            files: None,
        };

        let result = extract_zip(&ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cancelled"));
    }
}
