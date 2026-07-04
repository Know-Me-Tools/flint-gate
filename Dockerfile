# ─────────────────────────────────────────────────────────────────────────────
# Flint Gate — Multi-stage Docker build
# Build: rust:1.90-bookworm
# Runtime: debian:bookworm-slim
# ─────────────────────────────────────────────────────────────────────────────

# ── Stage 1: Web assets ───────────────────────────────────────────────────────
FROM node:22-alpine AS web-builder

WORKDIR /app

RUN corepack enable && corepack prepare pnpm@9.15.0 --activate

COPY web/package.json web/pnpm-lock.yaml ./web/
COPY sdks/typescript /app/sdks/typescript

WORKDIR /app/web
RUN pnpm install --frozen-lockfile

COPY web/ ./
RUN pnpm run build

# ── Stage 2: Rust builder ─────────────────────────────────────────────────────
FROM rust:1.90-bookworm AS builder

WORKDIR /app

# Install system dependencies for SQLx native client
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies separately from source
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN mkdir -p crates/flint-gate/src && echo 'fn main() {}' > crates/flint-gate/src/main.rs
RUN mkdir -p crates/flint-gate-core/src && echo '' > crates/flint-gate-core/src/lib.rs
RUN mkdir -p crates/flint-gate-client/src && echo '' > crates/flint-gate-client/src/lib.rs
RUN cargo build --release 2>/dev/null || true

# Build the actual source (and embed the admin UI assets)
COPY crates ./crates
COPY --from=web-builder /app/web/dist ./web/dist
RUN find crates -name '*.rs' -exec touch {} + && cargo build --release

# ── Stage 3: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary
COPY --from=builder /app/target/release/flint-gate /usr/local/bin/flint-gate

# Non-root user
RUN useradd -m -u 1001 -s /bin/bash flintgate
USER flintgate

# Config directory
VOLUME ["/app/config"]

# Ports
EXPOSE 4456
EXPOSE 4457

# Environment
ENV RUST_LOG=info,flint_gate=debug
ENV FLINT_GATE_CONFIG=/app/config/config.yaml

ENTRYPOINT ["/usr/local/bin/flint-gate"]
