# Archive Extractor

A cross-platform archive extractor with a modern, human-friendly GUI built with Rust and egui.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)
![Tests](https://github.com/justcommitting/archive-extractorr/workflows/Release/badge.svg)

## Features

- **Human-friendly UI**: Contextual greeting, natural language messages, ETA, and animations
- **Cross-Platform**: Works on Windows, macOS, and Linux
- **Drag & Drop**: Animated drop zone with visual feedback
- **Multiple Formats**: Support for popular archive formats:
  - ZIP (.zip), TAR (.tar), GZIP (.gz), BZIP2 (.bz2)
  - XZ (.xz), RAR (.rar), 7z (.7z), Zstandard (.zst)
  - Brotli (.br), LZ4 (.lz4)
- **Real-time Progress**: Progress bar with ETA and speed display
- **Password-protected Archives**: Gentle password prompt with helpful hints
- **File Preview**: Browse archive contents before extracting
- **Search**: Quick search through archive contents
- **Sorting**: Sort by name, size, or type with helpful tooltips
- **Cancel Confirmation**: Prevents accidental cancellation during extraction
- **Theme Toggle**: Switch between dark and light themes
- **Keyboard Shortcuts**: Ctrl+O (open), Ctrl+D (destination), Ctrl+E (extract)

## Installation

### From Source

```bash
cd archive-extractor
cargo build --release
cargo run --release
```

### Pre-built Binaries

Pre-built binaries are available for:
- **Windows**: `archive-extractor.exe`
- **macOS**: `archive-extractor.app`
- **Linux**: `archive-extractor`

## Usage

### GUI Mode

Run without arguments to launch the graphical interface:

```bash
./target/release/archive-extractor
```

1. **Drop or Open**: Drag an archive onto the animated drop zone, or click "Browse Files"
2. **Browse Contents**: View files inside the archive with size details
3. **Choose Destination**: Select where to extract files
4. **Extract**: Click "Extract" and watch the progress with ETA
5. **Done!** Open the destination folder directly or extract another archive

### CLI Mode

```bash
# Extract an archive
archive-extractor extract archive.zip -o /path/to/destination

# List archive contents
archive-extractor list archive.zip

# Show archive information
archive-extractor info archive.zip

# Extract with password for encrypted archives
archive-extractor extract protected.zip -p mypassword

# Verbose output
archive-extractor extract archive.zip -v
```

### Keyboard Shortcuts (GUI)

| Shortcut | Action |
|----------|--------|
| `Ctrl+O` / `Cmd+O` | Open archive |
| `Ctrl+D` / `Cmd+D` | Select destination folder |
| `Ctrl+E` / `Cmd+E` | Extract |
| `Ctrl+Q` / `Cmd+Q` | Quit |
| `Escape` | Cancel extraction (with confirmation) |

## Project Structure

```
archive-extractor/
├── Cargo.toml          # Project dependencies
├── src/
│   ├── main.rs         # Application entry point (GUI + CLI)
│   ├── app.rs          # Main application logic and UI
│   ├── cli.rs          # Command-line interface
│   ├── extractor.rs    # Archive extraction engine
│   ├── formats.rs      # Format detection and utilities
│   └── ui/
│       ├── mod.rs      # UI module exports
│       └── theme.rs    # Theme configuration (dark/light)
├── assets/
│   └── icon.png        # Application icon
└── README.md
```

## Dependencies

- **egui/eframe**: Immediate mode GUI framework
- **zip**: ZIP archive support
- **tar**: TAR archive support
- **flate2**: GZIP compression
- **bzip2**: BZIP2 compression
- **xz2**: XZ compression
- **rfd**: Native file dialogs

## License

This project is licensed under the MIT License.

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## Security

See [SECURITY.md](SECURITY.md) for reporting security vulnerabilities.
