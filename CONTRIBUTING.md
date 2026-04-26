# Contributing to Toku

Thank you for your interest in contributing to Toku!

## Code of Conduct

Be respectful, constructive, and inclusive. We're building a tool for readers — let's treat each other well.

## How to Contribute

### Reporting Bugs

- Open an issue using the bug report template
- Include steps to reproduce, expected behavior, and actual behavior
- Include platform (Linux/macOS/Windows) and version information

### Suggesting Features

- Open an issue using the feature request template
- Describe the use case and how it aligns with Toku's local-first, no-social principles
- Note: features that require a server for core functionality will not be accepted

### Adding an Importer

- Importers are a great way to contribute! Check the `importer` label for requested sources
- Each importer lives in `toku-import/` and implements the `ImportEngine` trait
- Importers must support: `--dry-run`, idempotent re-import via source IDs, and provenance tracking
- Include test fixtures (real or anonymized export files) in `tests/fixtures/`

### Pull Requests

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes
4. Ensure all checks pass: `cargo fmt --check && cargo clippy --workspace -- -D warnings && cargo test --workspace`
5. Submit a pull request with a clear description

### Security Vulnerabilities

If you discover a security vulnerability, **do not open a public issue**. Instead, please use GitHub's private vulnerability reporting feature.

## Development Setup

```sh
# Clone the repo
git clone https://github.com/kafkade/toku.git
cd toku

# Build all crates
cargo build --workspace

# Run tests
cargo test --workspace

# Run the CLI
cargo run -p toku-cli -- --help
```

## Architecture

See `.github/copilot-instructions.md` for the full architecture overview and `docs/adr/` for Architecture Decision Records.

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
