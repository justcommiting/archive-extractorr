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
        s if s < TB => format!("{:.1} GB", s as f64 / GB as f64),
        s => format!("{:.1} TB", s as f64 / TB as f64),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_name() {
        assert_eq!(format_name(ArchiveFormat::Zip), "ZIP Archive");
        assert_eq!(format_name(ArchiveFormat::Tar), "TAR Archive");
        assert_eq!(format_name(ArchiveFormat::Gzip), "GZIP Compressed");
        assert_eq!(format_name(ArchiveFormat::Bzip2), "BZIP2 Compressed");
        assert_eq!(format_name(ArchiveFormat::Xz), "XZ Compressed");
        assert_eq!(format_name(ArchiveFormat::Rar), "RAR Archive");
        assert_eq!(format_name(ArchiveFormat::Unknown), "Unknown Format");
    }

    #[test]
    fn test_format_icon() {
        assert_eq!(format_icon(ArchiveFormat::Zip), "📦");
        assert_eq!(format_icon(ArchiveFormat::Rar), "📦");
        assert_eq!(format_icon(ArchiveFormat::Tar), "📋");
        assert_eq!(format_icon(ArchiveFormat::Unknown), "❓");
    }

    #[test]
    fn test_file_icon_directories() {
        let dir_entry = ArchiveEntry {
            name: "folder".to_string(),
            is_dir: true,
            size: 0,
            compressed_size: 0,
            path: Path::new("folder").to_path_buf(),
        };
        assert_eq!(file_icon(&dir_entry), "📁");
    }

    #[test]
    fn test_file_icon_extensions() {
        let txt_entry = ArchiveEntry {
            name: "file.txt".to_string(),
            is_dir: false,
            size: 0,
            compressed_size: 0,
            path: Path::new("file.txt").to_path_buf(),
        };
        assert_eq!(file_icon(&txt_entry), "📄");

        let pdf_entry = ArchiveEntry {
            name: "document.pdf".to_string(),
            is_dir: false,
            size: 0,
            compressed_size: 0,
            path: Path::new("document.pdf").to_path_buf(),
        };
        assert_eq!(file_icon(&pdf_entry), "📕");

        let jpg_entry = ArchiveEntry {
            name: "photo.jpg".to_string(),
            is_dir: false,
            size: 0,
            compressed_size: 0,
            path: Path::new("photo.jpg").to_path_buf(),
        };
        assert_eq!(file_icon(&jpg_entry), "🖼️");

        let mp3_entry = ArchiveEntry {
            name: "song.mp3".to_string(),
            is_dir: false,
            size: 0,
            compressed_size: 0,
            path: Path::new("song.mp3").to_path_buf(),
        };
        assert_eq!(file_icon(&mp3_entry), "🎵");

        let mp4_entry = ArchiveEntry {
            name: "video.mp4".to_string(),
            is_dir: false,
            size: 0,
            compressed_size: 0,
            path: Path::new("video.mp4").to_path_buf(),
        };
        assert_eq!(file_icon(&mp4_entry), "🎬");

        let rs_entry = ArchiveEntry {
            name: "code.rs".to_string(),
            is_dir: false,
            size: 0,
            compressed_size: 0,
            path: Path::new("code.rs").to_path_buf(),
        };
        assert_eq!(file_icon(&rs_entry), "⚙️");
    }

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1), "1 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn test_format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1048575), "1024.0 KB");
    }

    #[test]
    fn test_format_size_megabytes() {
        assert_eq!(format_size(1048576), "1.0 MB");
        assert_eq!(format_size(1572864), "1.5 MB");
        assert_eq!(format_size(1073741823), "1024.0 MB");
    }

    #[test]
    fn test_format_size_gigabytes() {
        assert_eq!(format_size(1073741824), "1.0 GB");
        assert_eq!(format_size(1610612736), "1.5 GB");
        assert_eq!(format_size(1099511627775), "1024.0 GB");
    }

    #[test]
    fn test_format_size_terabytes() {
        assert_eq!(format_size(1099511627776), "1.0 TB");
        assert_eq!(format_size(1649267441664), "1.5 TB");
    }

    #[test]
    fn test_supported_extensions() {
        let extensions = supported_extensions();
        assert!(extensions.contains(&"zip"));
        assert!(extensions.contains(&"tar"));
        assert!(extensions.contains(&"gz"));
        assert!(extensions.contains(&"rar"));
    }
}
