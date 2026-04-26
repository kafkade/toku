# ADR-001: Core Language — Rust

**Status**: Accepted
**Date**: 2026-04-26
**Decision**: Use Rust as the core language for all shared libraries and the CLI.

## Context

Toku is a multi-platform personal book manager. The core library must compile to native
binaries (Linux, macOS, Windows), WASM (web), and C FFI (iOS/macOS via Swift). The CLI
is the primary interface and must be fast, cross-platform, and distributable via
`cargo install` and pre-built binaries.

## Decision

**Rust** is the core language for all shared crates and the CLI binary.

## Rationale

- **CLI ecosystem**: clap v4 is the industry standard for Rust CLI tools — auto-generated
  completions, man pages, and help text.
- **Cross-compilation**: Native binaries for Linux/macOS/Windows from a single codebase.
  WASM via `wasm-pack`. C FFI via `cbindgen` or UniFFI for mobile.
- **SQLite**: `rusqlite` is mature and well-maintained, with FTS5 support.
- **Performance**: Sub-200ms CLI startup, <100ms search on 10k books.
- **Community**: Rust attracts contributors who value correctness and clean architecture.
- **Developer confirmed**: The maintainer's primary language is Rust.

## Alternatives Considered

| Language | Rejected Because |
|----------|-----------------|
| Go | Weaker FFI story, no WASM for shared-core, GC pauses |
| TypeScript/Node | Runtime dependency, cold start latency, poor FFI |
| Python | Too slow for CLI, no native cross-compilation, poor FFI |
| Swift | Apple-only for CLI, poor Windows/Linux story |

## Consequences

- Higher contribution barrier than Go or Python — mitigated by good documentation,
  clear module boundaries, and "good first issue" labels.
- Longer compile times — mitigated by workspace caching and incremental compilation.
