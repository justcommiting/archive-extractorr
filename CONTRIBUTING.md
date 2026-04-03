# Contributing to Archive Extractor

Thank you for your interest in contributing to Archive Extractor! This document provides guidelines and instructions for contributing.

## Getting Started

1. **Fork the repository** and clone your fork
2. **Install Rust** (latest stable version recommended)
3. **Build the project**:
   ```bash
   cargo build
   ```
4. **Run the application**:
   ```bash
   cargo run
   ```

## Development Guidelines

### Code Style
- We use `rustfmt` for consistent formatting
- Run `cargo fmt` before committing
- We use `clippy` for linting
- Run `cargo clippy` to check for issues

### Testing
- Run all tests: `cargo test`
- Add tests for new functionality
- Ensure existing tests pass before submitting a PR

### Commit Messages
- Use clear, descriptive commit messages
- Prefix with the area of change (e.g., "ui: ", "extractor: ", "ci: ")
- Example: `ui: Add keyboard shortcuts for common actions`

## Pull Request Process

1. Create a feature branch from `main`
2. Make your changes and ensure tests pass
3. Update documentation if needed
4. Submit a pull request with a clear description
5. Respond to review feedback

## Reporting Issues

### Bug Reports
Include:
- Steps to reproduce the issue
- Expected behavior
- Actual behavior
- Archive Extractor version
- Operating system and version
- Sample archive file (if applicable)

### Feature Requests
Include:
- Clear description of the desired functionality
- Use case or problem it solves
- Any relevant examples or mockups

## Architecture Overview

- `src/main.rs` - Application entry point
- `src/app.rs` - Main application state and UI logic
- `src/extractor.rs` - Archive extraction engine
- `src/formats.rs` - Format detection and utilities
- `src/ui/` - UI components and themes

## Supported Formats

- ZIP (.zip) - including password-protected
- TAR (.tar)
- GZIP (.gz)
- BZIP2 (.bz2)
- XZ (.xz)
- RAR (.rar)

## Questions?

Feel free to open an issue for questions or discussions about contributing.
