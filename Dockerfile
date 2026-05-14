# ── Stage 1: Build ────────────────────────────────────────────────────────────
# Edition 2024 requires Rust 1.85+; use 1.85-slim as the minimum.
FROM rust:1.85-slim AS builder

WORKDIR /build

# System deps needed to compile reqwest's native-TLS (OpenSSL)
RUN apt-get update \
  && apt-get install -y --no-install-recommends pkg-config libssl-dev \
  && rm -rf /var/lib/apt/lists/*

# ── Layer-cache: copy workspace manifests and stubs before real source ────────
# Cargo resolves and downloads all dependencies on the first `cargo build`.
# Providing stub src files lets us cache that expensive download layer separately
# from the actual application source.

COPY Cargo.toml Cargo.lock ./
COPY axion-core/Cargo.toml   axion-core/Cargo.toml
COPY axion-kernel/Cargo.toml axion-kernel/Cargo.toml
COPY axion-server/Cargo.toml axion-server/Cargo.toml

# axion-core has both a lib crate and a bin crate.
RUN mkdir -p axion-core/src axion-kernel/src axion-server/src \
  && echo "pub fn stub() {}" > axion-core/src/lib.rs \
  && echo "fn main() {}"     > axion-core/src/main.rs \
  && echo "pub fn stub() {}" > axion-kernel/src/lib.rs \
  && echo "fn main() {}"     > axion-server/src/main.rs

# Compile deps only (stubs will fail to link but deps are fetched and compiled)
RUN cargo build --release -p axion-server 2>/dev/null || true

# ── Real source ───────────────────────────────────────────────────────────────
COPY axion-core/   axion-core/
COPY axion-kernel/ axion-kernel/
COPY axion-server/ axion-server/

# Touch the entrypoints so Cargo detects they changed vs the stubs above
RUN touch axion-server/src/main.rs \
          axion-core/src/lib.rs \
          axion-core/src/main.rs \
          axion-kernel/src/lib.rs

RUN cargo build --release -p axion-server

# ── Stage 2: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates libssl3 curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# The compiled server binary
COPY --from=builder /build/target/release/axion-server ./axion-server

# WASM binaries + JSON manifests.
# Registry::init_default() Strategy 3 uses CWD-relative "professionals/manifests",
# so with WORKDIR=/app these must live at /app/professionals/{*.wasm,manifests/*.json}.
COPY axion-core/professionals/ ./professionals/

# Persistent-data directories — overridden by volume mounts in production
RUN mkdir -p missions uploads

EXPOSE 8000

ENV PORT=8000

CMD ["./axion-server"]
