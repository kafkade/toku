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

## The ultimate safety net: local-first

Your **local SQLite library is always the ultimate recovery.** Even if the server is gone,
your account is locked out, or every secret is lost, any device that holds a copy of the
library still has your data in an open, portable format:

```sh
toku export backup   # canonical, portable ZIP archive of your library
```

Keep at least one offline backup. Combined with your Emergency Kit, that is everything you
need to recover from any single loss.
