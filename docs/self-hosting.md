# Self-hosting Toku

Toku itself is **local-first and works fully offline** — you never need a server to manage
your library, track reading, run statistics, import, or export. The only thing you can
self-host is the **optional** `toku-sync` relay server, which syncs an encrypted copy of
your library across multiple devices.

The complete deployment guide lives in **[`sync-server.md`](./sync-server.md)** and covers:

- **Quick start** with `docker run` and `docker compose up -d`
- **First-run onboarding & admin** — the first account becomes admin, opening/closing
  self-registration, and the optional device-approval gate
- **Configuration** (`TOKU_SYNC_PORT`, `TOKU_SYNC_BIND`, `TOKU_SYNC_DATA_DIR`, `TOKU_SYNC_LOG_LEVEL`)
- **Data persistence & backups** — the `/data` volume is the single source of truth
- **Upgrading** the image while preserving data
- **HTTPS with a reverse proxy** (Caddy / nginx) and TLS recommendations
- **Health checks** and multi-architecture (`amd64` / `arm64`) support
- The **zero-knowledge guarantee** and the **no server-side recovery** warning

A ready-to-use [`docker-compose.yml`](../docker-compose.yml) is provided in the repository
root.

> **Reminder:** the sync server is zero-knowledge — it only ever stores client-encrypted
> ciphertext and holds no key. Losing your password *and* Secret Key / Emergency Kit means
> your data is unrecoverable by design. See [`recovery.md`](./recovery.md).
