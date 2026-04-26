# ADR-003: CLI Design — clap v4 with Subcommands

**Status**: Accepted
**Date**: 2026-04-26
**Decision**: Use clap v4 (derive macros) for CLI parsing with a subcommand-based
structure. Support table, JSON, and CSV output formats.

## Context

The CLI is Toku's primary interface. It must be pleasant to use, scriptable, and
well-documented. It is a first-class product, not a debug tool.

## Decision

- **clap v4** with derive macros for argument parsing.
- **Subcommand structure**: `toku <verb> [args]` — e.g., `toku add`, `toku list`,
  `toku search`, `toku import`, `toku stats`.
- **Output formats**: `--format table` (default), `--format json`, `--format csv`.
  Respects `NO_COLOR` environment variable.
- **Shell completions**: Auto-generated for bash, zsh, fish, PowerShell via
  `clap_complete`.
- **Man pages**: Auto-generated via `clap_mangen`.
- **Rich table output**: via the `tabled` crate, responsive to terminal width.

## Command Hierarchy

```sh
toku add [--isbn <isbn> | --title <title> --author <author>]
toku show <book>
toku list [--status <status>] [--shelf <shelf>] [--sort <field>]
toku search <query>
toku reading start|update|finish|abandon <book> [--page N] [--rating N]
toku import goodreads|calibre|storygraph <path> [--dry-run]
toku export csv|json|backup [--output <path>]
toku stats [--year <year>]
toku shelf create|add|remove|list <shelf> [<books>...]
toku tag add|remove|list <tag> [<books>...]
toku config [--edit]
```

## Rationale

- clap v4 is the Rust CLI standard — well-documented, actively maintained, generates
  completions and man pages for free.
- Subcommand structure is familiar to users of `git`, `cargo`, `gh`, etc.
- JSON output enables scripting and integration with `jq`, pipes, etc.
- `tabled` provides attractive table output without pulling in a TUI framework.

## Alternatives Considered

| Option | Rejected Because |
|--------|-----------------|
| structopt | Merged into clap v4 — use clap directly |
| argh | Smaller ecosystem, no completions/man pages |
| Custom parser | Unnecessary reinvention |
