# ── Stage 1: Build ──────────────────────────────────────────
FROM rust:1.95-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev clang && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY agos-kernel/ agos-kernel/
COPY Cargo.* ./

WORKDIR /app/agos-kernel
RUN cargo build --release --features "z3-solver,wasm-sandbox,embodiment" && \
    cp target/release/agos-kernel /app/agos-kernel-bin

# ── Stage 2: Runtime ────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

RUN groupadd -r agos && useradd -r -g agos -d /var/lib/agos -s /sbin/nologin agos

RUN mkdir -p /var/lib/agos/aapps /var/lib/agos/data && \
    chown -R agos:agos /var/lib/agos

COPY --from=builder /app/agos-kernel-bin /usr/local/bin/agos-kernel

EXPOSE 7400

USER agos
WORKDIR /var/lib/agos

ENV RUST_LOG=info
HEALTHCHECK --interval=30s --timeout=5s --retries=3 \
  CMD curl -sf http://localhost:7400/health || exit 1

ENTRYPOINT ["agos-kernel"]
