# ── Stage 1: Build ────────────────────────────────────────────────────────────
# Edition 2024 requires Rust 1.85+; use 1.85-slim as the minimum.
FROM rust:1.88-slim AS builder

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
COPY opalzero-core/Cargo.toml   opalzero-core/Cargo.toml
COPY opalzero-kernel/Cargo.toml opalzero-kernel/Cargo.toml
COPY opalzero-server/Cargo.toml opalzero-server/Cargo.toml

# opalzero-core has both a lib crate and a bin crate.
RUN mkdir -p opalzero-core/src opalzero-kernel/src opalzero-server/src \
  && echo "pub fn stub() {}" > opalzero-core/src/lib.rs \
  && echo "fn main() {}"     > opalzero-core/src/main.rs \
  && echo "pub fn stub() {}" > opalzero-kernel/src/lib.rs \
  && echo "fn main() {}"     > opalzero-server/src/main.rs

# Compile deps only (stubs will fail to link but deps are fetched and compiled)
RUN cargo build --release -p opalzero-server 2>/dev/null || true

# ── Real source ───────────────────────────────────────────────────────────────
COPY opalzero-core/   opalzero-core/
COPY opalzero-kernel/ opalzero-kernel/
COPY opalzero-server/ opalzero-server/

# Touch the entrypoints so Cargo detects they changed vs the stubs above
RUN touch opalzero-server/src/main.rs \
          opalzero-core/src/lib.rs \
          opalzero-core/src/main.rs \
          opalzero-kernel/src/lib.rs

RUN cargo build --release -p opalzero-server

# ── Stage 2: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates libssl3 curl \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# The compiled server binary
COPY --from=builder /build/target/release/opalzero-server ./opalzero-server

# WASM binaries + JSON manifests.
# Registry::init_default() Strategy 3 uses CWD-relative "professionals/manifests",
# so with WORKDIR=/app these must live at /app/professionals/{*.wasm,manifests/*.json}.
COPY opalzero-core/professionals/ ./professionals/

# Persistent-data directories — overridden by volume mounts in production
RUN mkdir -p missions uploads

EXPOSE 8000

ENV PORT=8000

CMD ["./opalzero-server"]
