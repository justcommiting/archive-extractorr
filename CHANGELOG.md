# Changelog

All notable changes to Archive Extractor will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Keyboard shortcuts (Ctrl+O, Ctrl+D, Ctrl+E, Ctrl+Q, Escape)
- Per-file progress tracking during extraction
- Error logging for individual file extraction failures
- Unit tests for extractor and formats modules
- CI linting with Clippy and rustfmt

### Changed
- Continue extraction on individual file errors instead of failing completely
- macOS builds now use universal binary (Intel + Apple Silicon)

### Fixed
- GB/TB size formatting bug in `format_size()`

## [0.1.0] - 2026-04-03

### Added
- Initial release
- Support for ZIP, TAR, GZIP, BZIP2, XZ, and RAR formats
- Password-protected ZIP archive support with auto-detection
- Modern dark-themed GUI built with egui
- Drag-and-drop archive loading
- File preview with search functionality
- Real-time extraction progress
- Cross-platform support (Windows, macOS, Linux)
- Light/dark theme toggle
