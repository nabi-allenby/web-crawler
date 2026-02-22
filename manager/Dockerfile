# Stage 1: Build Rust binary
FROM rust:1.83-bookworm AS builder

ARG PROFILE=release

RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy workspace manifests for dependency resolution
COPY Cargo.toml Cargo.lock ./
COPY shared/Cargo.toml shared/Cargo.toml
COPY manager/Cargo.toml manager/Cargo.toml
COPY feeder/Cargo.toml feeder/Cargo.toml

# Create stub source for workspace members we don't need to compile
RUN mkdir -p shared/src && echo "" > shared/src/lib.rs && \
    mkdir -p feeder/src && echo "fn main() {}" > feeder/src/main.rs && \
    mkdir -p manager/src && echo "fn main() {}" > manager/src/main.rs

# Pre-fetch dependencies (cached layer)
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo fetch --locked

# Copy real source code
COPY shared/src shared/src
COPY manager/src manager/src

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    TARGET_DIR=$([ "$PROFILE" = "dev" ] && echo "debug" || echo "$PROFILE") && \
    cargo build --profile ${PROFILE} --package manager && \
    cp /app/target/${TARGET_DIR}/manager /usr/local/bin/manager

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

# Create non-root user matching Helm securityContext (uid 1000)
RUN useradd -r -u 1000 -m manager
USER manager

COPY --from=builder /usr/local/bin/manager /usr/local/bin/manager

ENTRYPOINT ["/usr/local/bin/manager"]
