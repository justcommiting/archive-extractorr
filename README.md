# Archive Extractor

A cross-platform archive extractor with a modern, beautiful GUI built with Rust and egui.

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)
![Tests](https://github.com/justcommiting/archive-extractorr/workflows/Release/badge.svg)

## Features

- **Modern GUI**: Clean, dark-themed interface built with egui
- **Cross-Platform**: Works on Windows, macOS, and Linux
- **Drag & Drop**: Simply drag archive files onto the window
- **Multiple Formats**: Support for popular archive formats:
  - ZIP (.zip)
  - TAR (.tar)
  - GZIP (.gz)
  - BZIP2 (.bz2)
  - XZ (.xz)
  - RAR (.rar)
- **Real-time Progress**: Visual progress bar during extraction
- **File Preview**: Browse archive contents before extracting
- **Search**: Quick search through archive contents
- **Theme Toggle**: Switch between dark and light themes

## Installation

### From Source

```bash
# Clone or navigate to the project directory
cd archive-extractor

# Build in release mode
cargo build --release

# Run the application
cargo run --release
```

### Pre-built Binaries

Pre-built binaries are available for:
- **Windows**: `archive-extractor.exe`
- **macOS**: `archive-extractor.app`
- **Linux**: `archive-extractor` (AppImage coming soon)

## Usage

1. **Open an Archive**: Click "Open Archive" or drag and drop an archive file
2. **Browse Contents**: View the files inside the archive
3. **Select Destination**: Choose where to extract files
4. **Extract**: Click the "Extract" button to extract all files

### Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+O` / `Cmd+O` | Open archive |
| `Ctrl+D` / `Cmd+D` | Select destination folder |
| `Ctrl+E` / `Cmd+E` | Extract |
| `Ctrl+Q` / `Cmd+Q` | Quit |
| `Escape` | Cancel extraction |

## Building for Production

### Windows

```bash
cargo build --release --target x86_64-pc-windows-msvc
```

### macOS

```bash
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin  # Apple Silicon
```

### Linux

```bash
cargo build --release --target x86_64-unknown-linux-gnu
```

## Project Structure

```
archive-extractor/
├── Cargo.toml          # Project dependencies
├── src/
│   ├── main.rs         # Application entry point
│   ├── app.rs          # Main application logic
│   ├── extractor.rs    # Archive extraction engine
│   ├── formats.rs      # Format detection and utilities
│   └── ui/
│       ├── mod.rs      # UI module exports
│       ├── theme.rs    # Theme configuration
│       └── widgets.rs  # Custom UI widgets
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

This project is licensed under the MIT License - see the LICENSE file for details.

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## Security

See [SECURITY.md](SECURITY.md) for reporting security vulnerabilities.

## Acknowledgments

- [egui](https://github.com/emilk/egui) - The immediate mode GUI framework
- [rfd](https://github.com/PolyMeilex/rfd) - Rust File Dialog
