# Web dashboard authentication

`toku serve` starts the Toku web dashboard. It runs in one of two modes:

- **Local mode** (default) — no authentication, loopback-only. This preserves the historical
  single-user behaviour: the dashboard renders your local SQLite library directly for one
  trusted user on their own machine.
- **Hosted mode** (`--hosted`) — every route is gated behind a cookie session, so you can expose
  the dashboard on a network (a home server, a LAN, behind a reverse proxy) without leaving your
  library open to anyone who can reach the port.

Hosted-mode auth is **self-contained**: the admin account and sessions live in your local
`toku.db`, so the dashboard does not depend on a running `toku-sync` server. (Forward-compatible:
the session layer stays in place if the web tier later federates to a sync server over REST.)

## Local mode (default)

```bash
toku serve
# Dashboard on http://127.0.0.1:3000 — no login, loopback only.
```

Local mode refuses to bind a non-loopback host. If you pass `--host 0.0.0.0` without `--hosted`,
the server exits with an error rather than silently exposing an unauthenticated dashboard. Use
hosted mode for network access.

## Hosted mode

```bash
toku serve --hosted --host 0.0.0.0 --port 3000
```

Flags / environment variables:

| Flag | Env | Effect |
| --- | --- | --- |
| `--hosted` | `TOKU_WEB_HOSTED=1` | Enable authentication and allow non-loopback binds. |
| `--insecure-cookies` | `TOKU_WEB_INSECURE_COOKIES=1` | Drop the `Secure` attribute on auth cookies (for local HTTP testing only — **never** in production). |

In hosted mode, auth cookies are marked `Secure` by default, so the dashboard expects to be
served over HTTPS (terminate TLS at a reverse proxy such as Caddy, nginx, or Traefik). Only use
`--insecure-cookies` for plain-HTTP testing on `localhost`.

### First-run onboarding

On first run there is no admin account, so every request is redirected to `/setup`. The setup
form asks for an email and a password and then:

1. generates a **Secret Key** and derives the account key hierarchy (per
   [ADR-010](./adr/010-self-host-auth.md)), and
2. stores an SRP verifier (not the password) plus the wrapped keys as the `admin` account.
   The verifier folds in **both** the password and the Secret Key
   (`SHA-256(domain_sep || Secret Key || password)`), so a stolen verifier cannot be
   brute-forced against the password alone.

The **Emergency Kit** (your email + Secret Key) is shown **once**, immediately after setup. Save
it — print it or store it in a password manager. It cannot be recovered: if you lose both your
password and your Secret Key, the account cannot be unlocked. See [recovery](./recovery.md).

Once an admin exists, `/setup` redirects to `/login` and a second admin cannot be created through
it.

### Login, sessions, and lockout

`/login` accepts the admin email, password, and **Secret Key** over TLS. The Secret Key (from
your Emergency Kit) is required because it is folded into the SRP verifier — logging in needs
both secrets, so a leaked password alone is not enough. The server recomputes the SRP verifier
and compares it to the stored verifier in constant time. On success it mints a **fresh** session
token (so a pre-authentication cookie is never reused — session-fixation safe) and sets an
`HttpOnly`, `SameSite=Lax` session cookie. Sessions last 24 hours.

A wrong or malformed Secret Key is rejected exactly like a wrong password (same message, same
timing). After 5 failed attempts the account locks for 15 minutes; correct credentials are
refused with a lockout message until the window passes.

Sign out from the header link (or `POST /logout`), which deletes the session server-side and
clears the cookie.

### CSRF protection

All state-changing requests in hosted mode are protected with a double-submit CSRF token: a
`toku_csrf` cookie plus a matching hidden form field (`csrf_token`) on every form. File uploads
(multipart) carry the token as a `?csrf=` query parameter instead. Mismatched or missing tokens
are rejected with `403`. The `SameSite=Lax` session cookie is an additional CSRF defence.

### Health check

`GET /healthz` is unauthenticated in both modes, for container/orchestration liveness probes.

## Trusted-server trade-off (threat model)

> **Important.** In hosted mode the dashboard renders **server-side HTML from your decrypted
> local library**, so the server process necessarily holds plaintext, and login sends your
> password **and Secret Key** to the server (over TLS) to be verified. This is unavoidable in
> the trusted-server posture: unlike the zero-knowledge sync relay, the dashboard decrypts and
> renders your library, so it must receive both secrets.

This is the **trusted-server** posture. The zero-knowledge guarantees of
[ADR-010](./adr/010-self-host-auth.md) apply to the `toku-sync` *relay* (which only ever sees
end-to-end-encrypted operations), **not** to a dashboard you intentionally point at your own
decrypted library. True in-browser, zero-knowledge unlock (client-side WASM crypto with the
server never seeing plaintext) is **out of scope** for the initial hosted dashboard and is
tracked for the threat-model work in issue #125.

Practical guidance:

- Run hosted mode only on hardware you trust, behind TLS.
- Treat the host as a trusted tier: anyone with root on the box, or the ability to read process
  memory, can read your library regardless of the login gate.
- Keep the dashboard off the public internet unless you have a specific reason and additional
  controls (VPN, authenticating reverse proxy, etc.).
