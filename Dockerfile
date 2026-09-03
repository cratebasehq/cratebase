# syntax=docker/dockerfile:1

# ---- frontend build -----------------------------------------------------
# Builds the JS SDK and the admin dashboard's static assets, which get
# embedded straight into the Rust binary in the next stage (rust-embed).
FROM oven/bun:1-slim AS frontend
WORKDIR /app
COPY package.json ./
COPY sdk/js sdk/js
COPY web/admin web/admin
RUN bun install && bun run sdk:build && bun run admin:build

# ---- deps cache layer -------------------------------------------------
# Copies only the manifests first so `cargo build` for dependencies is
# cached across rebuilds that only touch application source.
FROM rust:1-slim-bookworm AS chef
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates/core/Cargo.toml crates/core/Cargo.toml
COPY crates/filter/Cargo.toml crates/filter/Cargo.toml
COPY crates/db/Cargo.toml crates/db/Cargo.toml
COPY crates/storage/Cargo.toml crates/storage/Cargo.toml
COPY crates/auth/Cargo.toml crates/auth/Cargo.toml
COPY crates/server/Cargo.toml crates/server/Cargo.toml
COPY web/admin/dist/.gitkeep web/admin/dist/.gitkeep
RUN mkdir -p crates/core/src crates/filter/src crates/db/src crates/storage/src crates/auth/src crates/server/src \
    && for c in core filter db storage auth; do echo "fn _stub() {}" > crates/$c/src/lib.rs; done \
    && echo "fn main() {}" > crates/server/src/main.rs \
    && cargo build --release --workspace 2>/dev/null || true

# ---- builder ------------------------------------------------------------
FROM planner AS builder
COPY crates crates
COPY Cargo.toml Cargo.lock ./
COPY --from=frontend /app/web/admin/dist web/admin/dist
# Touch sources so cargo doesn't skip the real build using the stub mtimes.
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --release -p cratebase-server

# ---- runtime --------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 cratebase
WORKDIR /app
COPY --from=builder /app/target/release/cratebase /usr/local/bin/cratebase
RUN mkdir -p /app/data && chown -R cratebase:cratebase /app
USER cratebase
ENV CRATEBASE_DATA_DIR=/app/data
EXPOSE 8090
ENTRYPOINT ["cratebase"]
CMD ["serve"]
