# syntax=docker/dockerfile:1

# Multi-stage build for the toku-sync relay server.
#
# Build context must be the repository root, e.g.:
#   docker build -f crates/toku-sync/Dockerfile -t toku-sync .
#
# The builder cross-compiles a static musl binary on the *native* build platform
# (no QEMU for the Rust compile), then copies it into a minimal distroless image.
# Supports linux/amd64 and linux/arm64 (e.g. Raspberry Pi).

# ── Builder ───────────────────────────────────────────────────────────────────
FROM --platform=$BUILDPLATFORM rust:1.88-bookworm AS builder

# Provided automatically by buildx.
ARG TARGETARCH

# Map the Docker target arch to a Rust musl target and install the cross
# toolchain needed to link a static binary for it.
RUN set -eux; \
    case "$TARGETARCH" in \
        amd64) RUST_TARGET="x86_64-unknown-linux-musl"; GCC_PKG="gcc-x86-64-linux-gnu" ;; \
        arm64) RUST_TARGET="aarch64-unknown-linux-musl"; GCC_PKG="gcc-aarch64-linux-gnu" ;; \
        *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac; \
    echo "$RUST_TARGET" > /rust_target; \
    apt-get update; \
    apt-get install -y --no-install-recommends musl-tools "$GCC_PKG"; \
    rm -rf /var/lib/apt/lists/*; \
    rustup target add "$RUST_TARGET"

WORKDIR /build

# Copy the whole workspace (build context = repo root). The .dockerignore keeps
# this small by excluding target/, the native apps, etc.
COPY . .

# Tell cargo/cc which linker to use for the aarch64 musl target.
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-gnu-gcc \
    CC_aarch64_unknown_linux_musl=aarch64-linux-gnu-gcc

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    set -eux; \
    RUST_TARGET="$(cat /rust_target)"; \
    cargo build --release --locked -p toku-sync --target "$RUST_TARGET"; \
    cp "target/$RUST_TARGET/release/toku-sync" /toku-sync

# Pre-create the data dir owned by the runtime's non-root uid so a freshly
# created named volume mounted at /data inherits writable ownership.
RUN mkdir -p /data && chown 65532:65532 /data

# ── Runtime ───────────────────────────────────────────────────────────────────
# distroless static: tiny, no shell, runs as non-root, ships CA certs + /etc/passwd.
FROM gcr.io/distroless/static-debian12:nonroot

LABEL org.opencontainers.image.title="toku-sync" \
      org.opencontainers.image.description="Sync relay server for the Toku personal book manager" \
      org.opencontainers.image.source="https://github.com/kafkade/toku" \
      org.opencontainers.image.licenses="MIT"

COPY --from=builder /toku-sync /toku-sync
COPY --from=builder --chown=65532:65532 /data /data

# Container defaults (overridable). The binary's own default data dir is
# ./toku-sync-data; in the image we point it at the mounted volume.
ENV TOKU_SYNC_DATA_DIR=/data \
    TOKU_SYNC_PORT=8080 \
    TOKU_SYNC_BIND=0.0.0.0 \
    TOKU_SYNC_LOG_LEVEL=info

EXPOSE 8080
VOLUME ["/data"]

# Runs as the distroless "nonroot" user (uid 65532).
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["/toku-sync", "healthcheck"]

ENTRYPOINT ["/toku-sync"]
