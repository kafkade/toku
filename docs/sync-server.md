# Self-hosting the Toku sync server

`toku-sync` is the optional relay server that lets you sync your Toku library across multiple
devices. It is a small, single-binary HTTP service backed by SQLite. It stores **only** sync
operations, which are **always** end-to-end encrypted client-side before upload — it can never
read your library. This is a **zero-knowledge** design: see
[Zero-knowledge guarantee](#zero-knowledge-guarantee) below.

Syncing is entirely opt-in. Toku works fully offline without ever running this server. See
[ADR-010](./adr/010-self-host-auth.md) (which supersedes ADR-006 and ADR-008) for the design.

This guide covers deploying the server with Docker.

## Quick start

### Docker

```bash
docker run -d \
  --name toku-sync \
  -p 8080:8080 \
  -v toku-sync-data:/data \
  ghcr.io/kafkade/toku-sync:latest
```

The server is now listening on `http://localhost:8080`. Check it:

```bash
curl http://localhost:8080/health
# {"status":"ok","version":"0.2.1"}
```

### Docker Compose

A ready-to-use [`docker-compose.yml`](../docker-compose.yml) is provided in the repository root:

```yaml
services:
  toku-sync:
    image: ghcr.io/kafkade/toku-sync:latest
    container_name: toku-sync
    restart: unless-stopped
    ports:
      - "8080:8080"
    volumes:
      - toku-sync-data:/data
    environment:
      - TOKU_SYNC_DATA_DIR=/data
      - TOKU_SYNC_PORT=8080
      - TOKU_SYNC_LOG_LEVEL=info
    healthcheck:
      test: ["CMD", "/toku-sync", "healthcheck"]
      interval: 30s
      timeout: 5s
      start_period: 5s
      retries: 3

volumes:
  toku-sync-data:
```

Start it with:

```bash
docker compose up -d
```

## Connecting a client

Once the server is reachable, point your Toku CLI at it:

```bash
toku sync init --server https://sync.example.com
toku sync push
toku sync pull
```

`toku sync init` always prompts for an encryption passphrase — client-side end-to-end
encryption is **mandatory** in hosted mode, so the server only ever stores opaque ciphertext.
Use the same passphrase on every device. See `toku sync --help` for the full command set.

## Zero-knowledge guarantee

In hosted mode the server is **zero-knowledge**: it can never read your library content.

- **Op payloads** are encrypted client-side (AES-256-GCM, key derived from your passphrase)
  before they leave the device. The server **rejects** any plaintext payload with HTTP `422`.
- **Snapshots** (compacted library state) are likewise encrypted before upload; plaintext
  snapshots are rejected with `422`.
- **Re-key** uploads only ever carry re-encrypted ciphertext.

What the server *can* see is limited to the routing/ordering metadata it needs to relay and
order ops. It never sees field values, titles, notes, ratings, or any content:

| Field | Server-visible? | Why |
|-------|-----------------|-----|
| `op_id` (UUID v7) | Yes | Dedup + cursor/ordering |
| `device_id` | Yes | Per-device cursors and device management |
| `hlc` (timestamp) | Yes | Hybrid Logical Clock ordering |
| `entity_type` (e.g. `book`) | Yes | Indexing + bound into the ciphertext AAD |
| `entity_id` (UUID) | Yes | Routing; opaque identifier, no content |
| `op_type` (e.g. `update`) | Yes | Merge semantics; bound into the AAD |
| `payload` | **No** | Always an encrypted envelope (or `null` for content-free ops) |
| snapshot blob | **No** | Always an encrypted envelope |

`entity_type` and `op_type` are intentionally left cleartext: the client binds them into the
authenticated-encryption AAD so the server cannot swap or re-target an op without detection,
and they are needed for indexing. This is an accepted, documented metadata exposure — it
reveals *how many* ops of each kind exist, never their content. Making `entity_type` opaque
was considered and rejected for this reason.

Because the server holds no key, losing your passphrase (and Secret Key / Emergency Kit) means
server-side data is unrecoverable by design. See [`recovery.md`](./recovery.md).

## Configuration

The server is configured entirely through environment variables (or equivalent CLI flags).

| Env variable | Default | Description |
|--------------|---------|-------------|
| `TOKU_SYNC_PORT` | `8080` | Port to listen on |
| `TOKU_SYNC_BIND` | `0.0.0.0` | Address to bind to |
| `TOKU_SYNC_DATA_DIR` | `/data` (image) | Directory for the SQLite database |
| `TOKU_SYNC_LOG_LEVEL` | `info` | Log verbosity: `error`, `warn`, `info`, `debug`, `trace` |

`RUST_LOG` is also honoured and takes precedence over `TOKU_SYNC_LOG_LEVEL` if set, allowing
per-module filters (e.g. `RUST_LOG=toku_sync=debug,tower_http=debug`).

> The container image defaults `TOKU_SYNC_DATA_DIR` to `/data`. When running the binary directly
> (outside Docker) the default is `./toku-sync-data`.

## Data persistence

All server state lives in a single SQLite database at `$TOKU_SYNC_DATA_DIR/sync.db`. Mounting a
volume at `/data` (as shown above) ensures data survives container restarts, recreation, and image
upgrades.

To back up the server, stop the container and copy the volume's contents, or copy `sync.db` while
the server is stopped:

```bash
docker run --rm -v toku-sync-data:/data -v "$PWD:/backup" busybox \
  cp /data/sync.db /backup/sync.db.bak
```

> The image runs as a non-root user (uid `65532`). A freshly created **named** volume inherits the
> correct ownership automatically. If you use a **bind mount** instead, make sure the host directory
> is writable by uid `65532` (e.g. `sudo chown 65532:65532 ./data`).

## Health checks

The image ships a built-in health check used by Docker/Compose and any orchestrator:

```bash
docker inspect --format '{{.State.Health.Status}}' toku-sync
# healthy
```

It runs `toku-sync healthcheck`, which performs a `GET /health` against the local port and exits
non-zero if the server is not responding. You can also poll `GET /health` directly from an external
monitor or Kubernetes liveness/readiness probe.

## Multi-architecture support

The image is published for both `linux/amd64` and `linux/arm64`, so it runs on regular x86 servers
as well as ARM devices such as a Raspberry Pi (64-bit OS required). Docker automatically pulls the
correct variant for your platform; no extra flags are needed.

## HTTPS with a reverse proxy

The server speaks plain HTTP. For internet-facing deployments, terminate TLS with a reverse proxy.
Sync credentials are sent as bearer tokens, so **HTTPS is strongly recommended** for any
non-localhost access.

### Caddy

Caddy obtains and renews Let's Encrypt certificates automatically. A minimal `Caddyfile`:

```caddyfile
sync.example.com {
    reverse_proxy localhost:8080
}
```

```bash
caddy run --config Caddyfile
```

### nginx

```nginx
server {
    listen 443 ssl;
    server_name sync.example.com;

    ssl_certificate     /etc/letsencrypt/live/sync.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/sync.example.com/privkey.pem;

    # Snapshots can be large; allow big request bodies (matches the server's 50 MB limit).
    client_max_body_size 50m;

    location / {
        proxy_pass         http://127.0.0.1:8080;
        proxy_set_header   Host              $host;
        proxy_set_header   X-Real-IP         $remote_addr;
        proxy_set_header   X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header   X-Forwarded-Proto $scheme;
    }
}
```

Obtain certificates with [certbot](https://certbot.eff.org/) (`certbot --nginx`) or your preferred
ACME client, then reload nginx.

## Building the image locally

You can build the image yourself from the repository root (build context must be the repo root):

```bash
docker build -f crates/toku-sync/Dockerfile -t toku-sync .
docker run -d -p 8080:8080 -v toku-sync-data:/data toku-sync
```

For a multi-arch build, use Buildx:

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -f crates/toku-sync/Dockerfile \
  -t toku-sync .
```
