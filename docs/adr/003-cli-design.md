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
toku                                            # Launch TUI browser (default)
toku browse                                     # Launch TUI browser (explicit)
toku add [--isbn <isbn> | --title <title> --author <author>] [-T <tag>...] [--status <status>]
toku show <book>
toku list [--status <status>] [--tag <tag>] [--sort <field>]
toku search <query> [--status <status>] [--tag <tag>] [--online]
toku reading start|update|finish|abandon <book> [--page N] [--rating N]
toku import goodreads|calibre|storygraph <path> [--dry-run]
toku export csv|json|backup [--output <path>]
toku stats [--year <year>]
toku tag add|remove|list <tag> [<books>...]
toku file add|list|remove|organize <book> [<path>|<format>] [--all] [--dry-run] [--copy]
toku file verify [<book>|--all]                                 # SHA-256 integrity check
toku file usage [--by format|author|shelf]                      # disk usage breakdown
toku convert <book> --to <format> [--from <format>] [--force]   # optional; needs Calibre
toku bulk tag|status|delete [--status <s>] [--tag <t>] [--dry-run]
toku config [--edit]
```

> **Note**: Shelves were merged into tags (see migration V8). Use `toku tag`
> for all user-defined book groupings. `ReadingStatus` remains a separate
> per-book state machine.

### Sync subcommands (Phase 7)

Sync is opt-in and additive — every command above works fully offline. The
`toku sync` namespace manages multi-device synchronization. See ADR-006
(sync strategy), ADR-008 (wire protocol), and ADR-010 (self-host + zero-knowledge
auth, which supersedes ADR-006/008's auth model) for the underlying design.

```sh
# Account auth (1Password-style: account password + device-generated Secret Key)
toku sync signup [--server <url>] [--email <addr>] [--device-name <name>] [--kit-out <file>]
toku sync login  [--server <url>] [--email <addr>]                  # Re-auth on an enrolled device
toku sync enroll [--server <url>] [--email <addr>] [--library-id <uuid>] [--device-name <name>]

# Deprecated: per-library passphrase setup (prefer signup/login/enroll)
toku sync init [--server <url>] [--library-id <uuid>] [--device-name <name>] [--passphrase]

toku sync status                                   # Show sync state, pending ops, devices, conflicts
toku sync push                                     # Push local changes to the sync server
toku sync pull                                     # Pull remote changes from the sync server
toku sync devices                                  # List devices (user-scoped when logged in)
toku sync deregister <device-id>                   # Deregister another device from the server
toku sync disable                                  # Disable sync (local data preserved)
toku sync purge [--days <n>]                        # Purge tombstoned books past the retention period (default 30)
toku sync rekey                                    # Change the encryption passphrase and re-encrypt server ops
toku sync compact                                  # Snapshot + prune the op log
toku sync conflicts                                # List unresolved conflicts (notes/reviews)
toku sync conflicts show <id>                      # Show the local/remote diff for a conflict
toku sync conflicts resolve <id> --keep local|remote
toku sync conflicts resolve-all --keep local|remote
```

> **Account auth (ADR-010)**: `signup` creates an account, generates a
> high-entropy **Secret Key** on the device, renders an **Emergency Kit** (shown
> once — `--kit-out file.pdf|.html|.txt`), and enrolls the first device as admin.
> `login` re-authenticates an already-enrolled device; `enroll` joins an existing
> account from a new device. All three prompt for the account password (and, for
> `login`/`enroll`, the Secret Key) via hidden input — secrets never appear in
> argv or shell history. The server never sees the password or Secret Key (SRP);
> recovering the shared library data key on a new device uses the wrapped key
> hierarchy fetched over an authenticated session. The Secret Key is never written
> to plaintext config — only derived session/key material is kept in the OS
> keychain.
>
> **Note**: All sync subcommands default the server to `http://localhost:8080`.
> The deprecated `toku sync init` uses a per-library passphrase: `--library-id`
> must match across all devices for a single library; omit it on the first device
> to generate one; `--passphrase` enables client-side encryption (interactive,
> hidden input). Only note and review edits can produce user-visible conflicts;
> all other entities merge silently per ADR-006's entity-specific rules. Like
> every other command, sync subcommands respect `--format table|json|csv` and
> `NO_COLOR`.

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
