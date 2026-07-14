# ─────────────────────────────────────────────────────────────────────────────
# Flint Gate — Multi-stage Docker build
# Build: rust:1.88-bookworm  (deps require 1.88+: redis, time, home)
# Runtime: debian:bookworm-slim
# ─────────────────────────────────────────────────────────────────────────────

# ── Stage 1: Builder ──────────────────────────────────────────────────────────
FROM rust:1.88-bookworm AS builder

WORKDIR /app

# Install system dependencies for SQLx native client
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependency fetch separately from compilation
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p crates/flint-gate/src \
        crates/flint-gate-core/src \
        crates/flint-gate-client/src \
    && echo 'fn main() {}' > crates/flint-gate/src/main.rs \
    && touch crates/flint-gate-core/src/lib.rs \
    && touch crates/flint-gate-client/src/lib.rs \
    && cargo fetch

# Build the actual source
COPY crates ./crates
RUN cargo build --release

# ── Stage 2: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
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
