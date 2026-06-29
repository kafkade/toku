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

## First-run onboarding & admin

A fresh server starts with **no accounts**. The first person to sign up becomes the
instance administrator; after that, self-registration is closed by default so a
publicly reachable server can't be claimed by a stranger.

### 1. Create the first (admin) account

From your first device, point the CLI at the new server and sign up:

```bash
toku sync signup --server https://sync.example.com --email you@example.com
```

This first account **automatically becomes the admin** (regardless of the registration
setting). During signup the CLI generates your account **Secret Key** and prints an
**Emergency Kit** — save the kit somewhere safe (write `--kit-out kit.pdf` to render a
file). You authenticate with your password *and* Secret Key; the server only ever
receives an SRP verifier and opaque, client-encrypted key material.

> **No server-side recovery.** The server is zero-knowledge and holds no key. If you lose
> both your password and your Secret Key / Emergency Kit, your encrypted data is
> unrecoverable — there is no admin override and no reset link. See
> [`recovery.md`](./recovery.md) and the [Zero-knowledge guarantee](#zero-knowledge-guarantee).

### 2. Add more accounts or devices

Self-registration stays **closed** after the first account. The admin controls who can
join through authenticated admin endpoints (a valid **admin session** bearer token is
required — obtained by logging in as the admin account; there is intentionally no
unauthenticated admin surface). The relevant endpoints are:

| Endpoint | Purpose |
|----------|---------|
| `GET/PUT /api/v1/admin/registration` | Read / open / close self-registration |
| `GET/PUT /api/v1/admin/device-approvals` | Read / toggle the device-approval gate |
| `GET /api/v1/admin/users` | List accounts |
| `POST /api/v1/admin/users/{id}/status` | Enable / disable an account |

For example, to temporarily open self-registration so additional people can sign up
(replace `$ADMIN_TOKEN` with your admin session token and close it again afterwards):

```bash
curl -X PUT https://sync.example.com/api/v1/admin/registration \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"open": true}'
```

New users then run `toku sync signup` exactly as above; additional devices for an
existing account use `toku sync enroll` (password + Secret Key).

The last remaining active admin cannot disable their own account, so an instance always
keeps at least one administrator.

### 3. (Optional) Require device approval

For an extra gate, enable **device approvals**. When enabled, a newly enrolled device
joining a library that already has an active device is held `pending` until an existing
device approves it:

```bash
curl -X PUT https://sync.example.com/api/v1/admin/device-approvals \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"required": true}'
```

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

## Upgrading

The container is stateless — **all** persistent state lives in the `/data` volume — so
upgrading is just pulling a newer image and recreating the container. Your data, accounts,
and devices survive the upgrade because the volume is untouched.

### Docker Compose

```bash
docker compose pull        # fetch the newer image
docker compose up -d       # recreate the container with the same volume
```

### docker run

```bash
docker stop toku-sync && docker rm toku-sync
docker pull ghcr.io/kafkade/toku-sync:latest
docker run -d \
  --name toku-sync \
  -p 8080:8080 \
  -v toku-sync-data:/data \
  ghcr.io/kafkade/toku-sync:latest
```

After recreating, confirm the new version is healthy:

```bash
curl https://sync.example.com/health
# {"status":"ok","version":"0.3.2","protocol_version":2,"min_protocol":1}
```

> **Back up before upgrading.** Take a copy of `sync.db` (see [Data persistence](#data-persistence))
> before pulling a new image so you can roll back if needed. Schema migrations run
> automatically on start and are forward-only.
>
> **Pin a tag for production.** `:latest` is convenient but moves underneath you. For
> reproducible upgrades, pin a specific version (e.g. `ghcr.io/kafkade/toku-sync:0.3.2`)
> and bump it deliberately. Both `MAJOR.MINOR` and full `MAJOR.MINOR.PATCH` tags are
> published alongside `latest`.

## Migrating from the legacy relay model

Early instances ran the **relay model**: a client-chosen `library_id`, an optional single
passphrase, unauthenticated registration, and (sometimes) plaintext ops on the server. The
current model uses **accounts** (email + password) with a high-entropy Secret Key, an
end-to-end key hierarchy, and zero-knowledge ops. A one-time migration moves existing data
and devices onto the new model without stranding anything.

### Wire protocol versions

The server advertises two numbers in `GET /health`:

- `protocol_version` — what the server speaks (currently `2`, the account model).
- `min_protocol` — the lowest client protocol accepted. Fresh and un-migrated instances stay
  at `1`, so legacy clients keep working. After migration it becomes `2`.

Once `min_protocol` is `2`, pre-account clients are rejected with **HTTP 426 (Upgrade
Required)**. Upgrade those installs to a current Toku build before they can sync again.

### One-time client migration

On a device that already has legacy sync configured, run:

```bash
toku sync migrate --email you@example.com
```

This generates a fresh Secret Key + account password, creates your account on the server
(the **first** account becomes admin and adopts all pre-existing libraries/devices),
re-protects every server op and snapshot under the new zero-knowledge key hierarchy
(plaintext ops are encrypted for the first time), and locks the instance to protocol 2.
Your Secret Key and Emergency Kit are shown **once** — store them offline.

**Other devices** rejoin afterwards with `toku sync enroll --email you@example.com` using the
same password + Secret Key.

> **Breaking change:** migration is forward-only and closes the old unauthenticated
> registration path. Minimum client version: the first release that ships `toku sync
> migrate` (account protocol 2). Back up `sync.db` first.

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
