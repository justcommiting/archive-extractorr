use crate::extractor::{ArchiveEntry, ArchiveFormat};
use std::path::Path;

/// Get a human-readable format name
pub fn format_name(format: ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Zip => "ZIP Archive",
        ArchiveFormat::Tar => "TAR Archive",
        ArchiveFormat::Gzip => "GZIP Compressed",
        ArchiveFormat::Bzip2 => "BZIP2 Compressed",
        ArchiveFormat::Xz => "XZ Compressed",
        ArchiveFormat::Rar => "RAR Archive",
        ArchiveFormat::Unknown => "Unknown Format",
    }
}

/// Get format icon/emoji
pub fn format_icon(format: ArchiveFormat) -> &'static str {
    match format {
        ArchiveFormat::Zip => "📦",
        ArchiveFormat::Tar => "📋",
        ArchiveFormat::Gzip => "🗜️",
        ArchiveFormat::Bzip2 => "🗜️",
        ArchiveFormat::Xz => "🗜️",
        ArchiveFormat::Rar => "📦",
        ArchiveFormat::Unknown => "❓",
    }
}

/// Get file icon based on entry type and extension
pub fn file_icon(entry: &ArchiveEntry) -> &'static str {
    if entry.is_dir {
        return "📁";
    }

    let ext = entry
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "txt" | "md" | "rst" | "readme" => "📄",
        "pdf" => "📕",
        "doc" | "docx" => "📘",
        "xls" | "xlsx" | "csv" => "📊",
        "ppt" | "pptx" => "📊",
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "svg" | "webp" => "🖼️",
        "mp3" | "wav" | "flac" | "aac" | "ogg" => "🎵",
        "mp4" | "avi" | "mkv" | "mov" | "webm" => "🎬",
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => "📦",
        "exe" | "msi" | "app" | "dmg" => "⚙️",
        "sh" | "bash" | "zsh" | "fish" => "🔧",
        "py" => "🐍",
        "js" | "ts" | "jsx" | "tsx" => "📜",
        "rs" | "go" | "c" | "cpp" | "h" | "hpp" => "⚙️",
        "html" | "css" | "scss" => "🌐",
        "json" | "xml" | "yaml" | "yml" | "toml" => "⚙️",
        _ => "📄",
    }
}

/// Format file size for display
pub fn format_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    match size {
        s if s < KB => format!("{} B", s),
        s if s < MB => format!("{:.1} KB", s as f64 / KB as f64),
        s if s < GB => format!("{:.1} MB", s as f64 / MB as f64),
        s if s < GB => format!("{:.1} GB", s as f64 / GB as f64),
        s => format!("{:.1} TB", s as f64 / TB as f64),
    }
}

/// Filter entries by search text (unused, kept for future use)
#[allow(dead_code)]
pub fn filter_entries<'a>(entries: &'a [ArchiveEntry], search: &str) -> Vec<&'a ArchiveEntry> {
    if search.is_empty() {
        return entries.iter().collect();
    }

    let search_lower = search.to_lowercase();
    entries
        .iter()
        .filter(|e| e.name.to_lowercase().contains(&search_lower))
        .collect()
}

/// Calculate total size of entries (unused, kept for future use)
#[allow(dead_code)]
pub fn total_size(entries: &[ArchiveEntry]) -> u64 {
    entries.iter().map(|e| e.size).sum()
}

/// Check if a path is a supported archive
pub fn is_supported_archive(path: &Path) -> bool {
    ArchiveFormat::detect(path).is_some()
}

/// Get supported extensions
pub fn supported_extensions() -> &'static [&'static str] {
    &[
        "zip", "tar", "gz", "gzip", "bz2", "bzip2", "xz", "lzma", "rar",
    ]
}
