# ── Stage 1: Build ──────────────────────────────────────────────
FROM rust:1.95-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy manifests for dependency caching
COPY Cargo.toml ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release 2>/dev/null; rm -rf src

# Copy real source and build (no ONNX feature for now)
COPY src/ ./src/
RUN touch src/main.rs && cargo build --release

# ── Stage 2: Runtime ────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/stock-market-ai /usr/local/bin/

WORKDIR /app

EXPOSE 8000

ENV RUST_LOG=stock_market_ai=info

CMD ["stock-market-ai"]
