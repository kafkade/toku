# Recovery & the Secret Key

Toku's optional self-hosted sync uses a **zero-knowledge** design (see
[ADR-010](adr/010-self-host-auth.md)). This document explains, in plain terms, **what can
and cannot be recovered**, and how the **Secret Key** and **Emergency Kit** fit in.

> **Local-first first.** None of this applies to single-device, offline use. Your library
> lives in a local SQLite database and works with no account, no server, and no network.
> The material below only concerns the *optional* hosted sync feature.

## The two secrets

Hosted sync authenticates you with **two** secrets that never leave your device:

1. **Account password** — chosen by you, memorable.
2. **Secret Key** — a 128-bit, high-entropy value generated **on your device**. It is
   formatted for transcription:

   ```text
   TK-XXXXXX-XXXXX-XXXXX-XXXXX-XXXXX-CC
   ```

   - `TK` is a version prefix.
   - The middle groups encode 128 bits of entropy in base32 (`A–Z`, `2–7`).
   - The final two characters are a **checksum**, so a single mistyped or swapped
     character is detected when you enter the key.

Your encryption keys are derived from **both** secrets. The server only ever stores an SRP
verifier and encrypted (wrapped) key material — it **cannot** derive your password, your
Secret Key, or any key that decrypts your data.

## The Emergency Kit

Because the Secret Key is generated on your device and never sent to the server, it is shown
**exactly once**. The **Emergency Kit** is your offline record of it — a one-page document
containing your account email, the server, the Secret Key, and a blank line to write your
password by hand.

Generate one with the CLI:

```sh
# Generate just a Secret Key (shown once)
toku account secret-key generate

# Render an Emergency Kit (generates a fresh Secret Key unless --secret-key is given)
toku account emergency-kit --email you@example.com --server https://toku.example.com \
  --kit-format text                       # plain text to stdout
toku account emergency-kit --email you@example.com --kit-format html --out kit.html
toku account emergency-kit --email you@example.com --kit-format pdf  --out kit.pdf
```

Print it or store it somewhere safe and **offline** (a safe, a password manager's secure
notes, a locked drawer). Do not store it as the only copy on the same device whose data it
protects.

## What can and cannot be recovered

| Situation | Recoverable? | How |
|---|---|---|
| You forget your **password** but still have a logged-in device | Yes | Re-authenticate on that device, then change the password (re-wraps your keys). |
| You lose your **Secret Key** but still have a logged-in device | Yes | The device already holds the unwrapped keys; export a backup and/or generate a new Emergency Kit. |
| You lose **both** secrets but still have a local copy of the library | Yes (data only) | Your local SQLite database is the recovery. `toku export backup` produces a portable archive. The server account itself is not recoverable. |
| You lose the **Secret Key** and have **no** local device/copy | **No** | Server data is **unrecoverable**. There is no server-side escrow and no reset that bypasses the Secret Key. |

### Why there is no "reset"

A password reset that recovered your data would mean the server could decrypt your data —
which would defeat the zero-knowledge guarantee. Toku deliberately has **no server-side key
escrow**. This is the same trade-off as 1Password's Secret Key: stronger privacy in exchange
for personal responsibility over the Secret Key.

## Device enrollment

A new device is enrolled by entering your **account email + password + Secret Key**. The
device performs SRP authentication, derives the account unlock key, and unwraps your keys
locally. The server never receives the secrets and cannot enroll a device on its own.

### First opt-in uploads your existing library

When you first opt into sync from a device that already has a local library — `toku sync
signup`, or `toku sync enroll` that creates a fresh library — Toku automatically **backfills**
your existing books, reading sessions, progress, and tags into the sync log and pushes them.
You do **not** need to run `toku sync compact` to seed the server; opt-in reports how many
items were uploaded. (Compaction is only a maintenance step that snapshots and prunes old
op history.)

Sync does **not** cover ebook file binaries (they stay on the device — see
[ADR-011](adr/011-file-management.md)) or authors, shelves, works, series, and ISBNs yet
(tracked in [#208](https://github.com/kafkade/toku/issues/208)); the CLI warns about this at
opt-in.

### Restoring a device (bootstrap)

When a device joins an **existing** library, Toku **bootstraps** it automatically: it
downloads and applies the latest server snapshot (if one exists after a `compact`), then
pulls any remaining op history. A device that enrolls while **approval is required** stays
pending and has nothing to restore yet — its bootstrap runs on the first `toku sync login`
after an existing device approves it.

You can also trigger this manually — for re-provisioning, or to recover a device whose local
state drifted:

```sh
toku sync bootstrap                 # apply the latest snapshot, then pull remaining ops
toku sync bootstrap --reset-cursor  # discard the local pull cursor and re-sync from scratch
```

`--reset-cursor` forces a full re-download (snapshot + full op tail) — useful if the local
pull cursor is ahead of a freshly restored server, or after a server-side reset. Because your
**local SQLite database stays the primary recovery** (see below), bootstrap is a convenience
for re-provisioning from the server, not a replacement for keeping an offline backup.

## Token storage

After you authenticate, the client keeps a **session token** (and, for hosted accounts, the
derived sync key) so you don't re-enter credentials on every command. Where these live
depends on the platform:

- **OS keychain (preferred).** On macOS (Keychain), Windows (Credential Manager), and Linux
  desktops running a Secret Service provider (GNOME Keyring, KWallet), the token is stored
  in the native credential store, protected by your OS login session.
- **Encrypted-at-rest file fallback.** When no keychain is available — common on **headless
  Linux servers**, containers, and CI — the client falls back to a JSON file at
  `<data_dir>/sync/tokens.json`. On Unix this file is created with `0600` permissions
  (owner read/write only). The client prints a `warning: … using file fallback` when this
  happens. You can force this path with `TOKU_TOKEN_STORE=file` (or
  `TOKU_DISABLE_KEYCHAIN=1`).

> **Tradeoff — the file fallback is plaintext at rest.** The fallback file is protected by
> filesystem permissions (`0600` on Unix), **not** by encryption. Anyone who can read that
> file as your user — or who obtains the disk/backup — can read a live session token. This
> is an intentional, documented tradeoff so headless clients keep working without a
> keychain. To harden a headless or shared host:
>
> - Keep the token on an **encrypted filesystem** (LUKS or equivalent), so an offline disk
>   or a stolen backup does not expose it.
> - Ensure the `<data_dir>` and its `sync/` subdirectory are owner-only (`chmod 700`).
> - On Windows there is no `0600` equivalent for the fallback file; rely on user-profile
>   isolation and prefer the Credential Manager path (the default).
> - Tokens are session credentials, not your secrets. A leaked session can be revoked
>   server-side via the logout endpoints (`POST /api/v1/auth/logout` for a device session,
>   `POST /api/v1/account/logout` for an account session), or by an admin disabling the
>   account — both delete the server-side session so the token can no longer be used.

## The ultimate safety net: local-first

Your **local SQLite library is always the ultimate recovery.** Even if the server is gone,
your account is locked out, or every secret is lost, any device that holds a copy of the
library still has your data in an open, portable format:

```sh
toku export backup   # canonical, portable ZIP archive of your library
```

Keep at least one offline backup. Combined with your Emergency Kit, that is everything you
need to recover from any single loss.

## Encrypted backups without sync (backup passphrase)

You do **not** need a sync account to encrypt a backup. On a device that has never enrolled
in sync, add `--encrypt`:

```sh
toku export backup --encrypt --output library.enc.zip
```

Toku prompts for a **backup passphrase** (entered twice to confirm), derives a key from it
with Argon2id, and seals the archive with AES-256-GCM. For automation you can supply the
passphrase via the `TOKU_BACKUP_PASSPHRASE` environment variable instead of the prompt.

The archive is **self-describing**: the key-derivation salt and parameters are stored inside
it, so it restores on **any** machine with only the passphrase — no `config.toml`, no sync
account, nothing else to carry:

```sh
toku import backup library.enc.zip           # prompts for the same passphrase
TOKU_BACKUP_PASSPHRASE=… toku import backup library.enc.zip   # non-interactive
```

If a sync server **is** configured, `--encrypt` keeps using your enrolled library key exactly
as before; the passphrase path is only the fallback for offline-only users. On restore Toku
detects which kind of encrypted backup it is from the archive itself.

> **⚠️ Lose the passphrase and the backup is gone.** A passphrase-encrypted backup has **no
> backdoor and no reset** — the same zero-knowledge stance as the Secret Key. If you forget
> the passphrase, that archive is permanently unrecoverable. Write the passphrase down and
> store it offline (a password manager, a safe), separate from the backup itself. A wrong
> passphrase on restore fails cleanly ("could not decrypt backup") and never corrupts your
> current library.

## At-rest database encryption (lost passphrase = unrecoverable)

Separate from backups, you can encrypt the **live** `toku.db` itself with SQLCipher (ADR-016,
issue #225). This is **opt-in and off by default**; the default plaintext database is unchanged.
It is available only in builds compiled with the `toku-db` `sqlcipher` feature.

```bash
toku db status            # is the database encrypted?
toku db encrypt           # encrypt in place (prompts for a new passphrase, twice)
toku db encrypt --remember  # also cache the passphrase in the OS keychain (opt-in)
toku db decrypt           # revert to a plaintext database
toku db forget            # drop any keychain-cached passphrase
```

Your passphrase is stretched with Argon2id (m=64 MiB, t=3, p=1) into a 256-bit key that unlocks
the database. **Toku never stores the passphrase or the derived key** — only the KDF salt,
parameters, and a verifier live in `config.toml`'s `[encryption]` section.

> **⚠️ Lose the passphrase and the database is gone.** At-rest encryption has **no backdoor and
> no reset** — the same zero-knowledge stance as the Secret Key and encrypted backups. If you
> forget the passphrase there is **no way**, for you or anyone else, to recover the data in that
> `toku.db`. Write the passphrase down and store it offline (a password manager, a safe),
> separate from the device. A wrong passphrase fails cleanly ("incorrect passphrase or not an
> encrypted Toku database") and never corrupts the file.

Providing the passphrase to short-lived CLI commands, in order of precedence:

1. **`TOKU_DB_PASSPHRASE` environment variable** (opt-in). Convenient for automation, but the
   **weakest** option: it can leak through `ps`, shell history, and CI logs. Use only where those
   risks are acceptable.
2. **OS keychain** (opt-in, via `toku db encrypt --remember`). The stronger convenience option —
   the passphrase is stored by the OS secret store, never in a plaintext file next to the
   database. Honors `TOKU_DISABLE_KEYCHAIN` / `TOKU_TOKEN_STORE=file`.
3. **Interactive prompt** (the baseline). If neither of the above is present, Toku prompts (up to
   three attempts).

Because the encryption is opt-in, the recoverability rules above stack: OS full-disk encryption
still protects a plaintext database, and encrypted backups (`toku export backup --encrypt`) remain
a separate, portable safety net regardless of whether the live database is encrypted.
